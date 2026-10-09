//! The Agent path through the Operation Gate.
//!
//! Everything an agent's request needs besides the receiver session lives here:
//! the policy in force, the audit log, the budget ledger, the clock, and the
//! caps. The gate in `service.rs` stays the one caller of a session's write; this
//! module decides, records, and counts, and hands back what the gate should do.
//!
//! It fails closed. With no policy, no readable audit history, or an evaluation
//! that cannot be recorded, an agent's write ends `rejected` and nothing is sent
//! to the receiver. Agents see fixed text for those faults and the limit that
//! fired for a refusal; rule ids, file paths, and error text stay in the audit
//! log and the Operator's views.
//!
//! This root keeps the state and the policy's lifecycle. `audit` builds the
//! records and appends them within their time bounds. `access` writes callers
//! refused at an endpoint to the log at a bounded rate. `budget` keeps the
//! ledger, the write caps, and the atomic step that evaluates and reserves.
//! `decision` decides a request, runs a dry run, and holds the fixed texts agents
//! are told.

use super::locked;
use crate::audit::{AuditEvent, Durability, SharedAuditLog};
use crate::clock::SharedClock;
use crate::control::{
    AgentLabel, ApprovalHealth, AuditHealth, PolicyHealth, PolicyView, Principal, ServiceHealth,
};
use crate::ledger::Ledger;
use crate::policy_source::{LoadedPolicy, PolicyDigest, SharedPolicySource};
use crate::tokens::SharedTokenStore;
use denon_avr_domain::WallTime;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::time::Instant;

mod access;
mod audit;
mod budget;
mod decision;

use access::AccessBook;
pub(super) use decision::{
    cancelled, decide, dry_run, fixed_reason, refusal, Gate, Request, RECEIVER_UNREACHABLE,
};

const POLICY_UNAVAILABLE: &str = "policy unavailable";
const LEDGER_UNAVAILABLE: &str = "the budget history is not available";

/// The caps on one agent label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentLimits {
    /// New writes (and dry runs, which open the receiver) per minute.
    pub writes_per_minute: u32,
    /// Operations that have not ended.
    pub unfinished_operations: usize,
}

impl Default for AgentLimits {
    fn default() -> Self {
        Self {
            writes_per_minute: 30,
            unfinished_operations: 8,
        }
    }
}

/// What the Agent path needs that the receiver side does not: where the policy
/// comes from, where the audit log is, what time it is, and the caps.
pub struct AgentPath {
    pub policy: SharedPolicySource,
    pub audit: SharedAuditLog,
    pub clock: SharedClock,
    pub limits: AgentLimits,
    /// Where Agent tokens are kept, when the service issues them. Without one the
    /// token methods answer `Unavailable`.
    pub tokens: Option<SharedTokenStore>,
}

impl AgentPath {
    /// The same path with a token store.
    pub fn with_tokens(mut self, tokens: SharedTokenStore) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

/// A policy that loaded, and when.
pub(super) struct ActivePolicy {
    loaded: LoadedPolicy,
    loaded_at: WallTime,
}

enum PolicyState {
    Active(Arc<ActivePolicy>),
    /// The last load failed, or none has happened. Agent writes are refused.
    Unavailable(String),
}

#[derive(Default)]
struct Books {
    ledger: Ledger,
    /// Whether the ledger has been rebuilt from the audit log.
    ready: bool,
}

pub(super) struct AgentState {
    policy_source: SharedPolicySource,
    audit: SharedAuditLog,
    clock: SharedClock,
    limits: AgentLimits,
    /// When this service started: the run its operation ids belong to.
    run: WallTime,
    policy: Mutex<PolicyState>,
    books: Mutex<Books>,
    /// Whether the last audit append succeeded.
    audit_ok: AtomicBool,
    /// One rebuild of the ledger at a time.
    rebuilding: tokio::sync::Mutex<()>,
    /// One load of the policy at a time, from the read to the record, so the
    /// policy in force and the log agree on which file loaded last.
    reloading: tokio::sync::Mutex<()>,
    /// When a refusal made before any decision was last written to the log, and
    /// how many have been refused since without being written.
    refusals: Mutex<(Option<Instant>, u32)>,
    writes: Mutex<HashMap<AgentLabel, VecDeque<Instant>>>,
    tokens: Option<SharedTokenStore>,
    /// The callers refused at an endpoint, and when each was last written.
    access: Mutex<AccessBook>,
}

impl AgentState {
    /// Load the policy, record that, and rebuild the ledger from the audit log.
    /// Neither failing stops the service: each is state that refuses agent
    /// writes until it is put right.
    pub(super) async fn start(path: AgentPath) -> Self {
        let run = path.clock.now();
        let state = Self {
            policy_source: path.policy,
            audit: path.audit,
            clock: path.clock,
            limits: path.limits,
            run,
            policy: Mutex::new(PolicyState::Unavailable(
                "the policy has not been loaded".into(),
            )),
            books: Mutex::new(Books::default()),
            audit_ok: AtomicBool::new(true),
            rebuilding: tokio::sync::Mutex::new(()),
            reloading: tokio::sync::Mutex::new(()),
            refusals: Mutex::new((None, 0)),
            writes: Mutex::new(HashMap::new()),
            tokens: path.tokens,
            access: Mutex::new(AccessBook::default()),
        };
        state.load_policy().await;
        state.ensure_ledger().await;
        state
    }

    // ---- Policy ----

    /// Read the policy again. A load that fails replaces the policy in force
    /// with nothing, so a bad edit never leaves the old rules quietly in place.
    pub(super) async fn load_policy(&self) -> PolicyView {
        let _one_at_a_time = self.reloading.lock().await;
        let now = self.clock.now();
        let (event, view) = match self.policy_source.load().await {
            Ok(loaded) => {
                let digest = loaded.digest;
                let view = PolicyView {
                    digest: Some(digest),
                    loaded_at: Some(now),
                    text: Some(loaded.text.clone()),
                    error: None,
                };
                *locked(&self.policy) = PolicyState::Active(Arc::new(ActivePolicy {
                    loaded,
                    loaded_at: now,
                }));
                (AuditEvent::PolicyLoaded { digest }, view)
            }
            Err(error) => {
                let text = error.to_string();
                *locked(&self.policy) = PolicyState::Unavailable(text.clone());
                let view = PolicyView {
                    digest: None,
                    loaded_at: None,
                    text: None,
                    error: Some(text.clone()),
                };
                (AuditEvent::PolicyLoadFailed { error: text }, view)
            }
        };
        let record = self.record(None, Principal::Operator, None, event);
        // A failure is noted in the audit health and is not the load's problem.
        let _ = self.append(record, Durability::Flushed).await;
        view
    }

    pub(super) fn policy_view(&self) -> PolicyView {
        match &*locked(&self.policy) {
            PolicyState::Active(policy) => PolicyView {
                digest: Some(policy.loaded.digest),
                loaded_at: Some(policy.loaded_at),
                text: Some(policy.loaded.text.clone()),
                error: None,
            },
            PolicyState::Unavailable(error) => PolicyView {
                digest: None,
                loaded_at: None,
                text: None,
                error: Some(error.clone()),
            },
        }
    }

    /// The digest of the policy in force, when there is one.
    pub(super) fn digest(&self) -> Option<PolicyDigest> {
        match &*locked(&self.policy) {
            PolicyState::Active(policy) => Some(policy.loaded.digest),
            PolicyState::Unavailable(_) => None,
        }
    }

    pub(super) fn health(&self) -> ServiceHealth {
        ServiceHealth {
            policy: match &*locked(&self.policy) {
                PolicyState::Active(_) => PolicyHealth::Active,
                PolicyState::Unavailable(_) => PolicyHealth::Unavailable,
            },
            audit: if self.audit_ok.load(Ordering::SeqCst) {
                AuditHealth::Ok
            } else {
                AuditHealth::Failing
            },
            ledger_ready: locked(&self.books).ready,
            approval: ApprovalHealth::Unavailable,
        }
    }

    /// What an agent's write needs before the receiver is opened: a policy to
    /// judge it by and a ledger to count it in. The text is for the agent.
    pub(super) async fn precheck(&self) -> Result<(), &'static str> {
        if self.active_policy().is_none() {
            return Err(POLICY_UNAVAILABLE);
        }
        if !self.ensure_ledger().await {
            return Err(LEDGER_UNAVAILABLE);
        }
        Ok(())
    }

    /// The policy in force now. A decision asks for it at the moment it is made,
    /// not before an await that can take seconds, so a reload that happened
    /// meanwhile, a bad one included, is what decides.
    pub(super) fn active_policy(&self) -> Option<Arc<ActivePolicy>> {
        match &*locked(&self.policy) {
            PolicyState::Active(policy) => Some(Arc::clone(policy)),
            PolicyState::Unavailable(_) => None,
        }
    }

    // ---- Tokens and refused callers ----

    /// The token store, when the service was given one.
    pub(super) fn tokens(&self) -> Option<&SharedTokenStore> {
        self.tokens.as_ref()
    }

    /// The time the service goes by.
    pub(super) fn now(&self) -> WallTime {
        self.clock.now()
    }
}
