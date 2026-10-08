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

use super::{locked, Inner, Lease, Operations, Resolution};
use crate::audit::{
    intent_text, AuditDecision, AuditEvent, AuditPage, AuditQuery, AuditRecord, Durability,
    SharedAuditLog, AUDIT_SCHEMA,
};
use crate::clock::SharedClock;
use crate::control::{
    AgentLabel, ApprovalHealth, AuditHealth, ControlError, DryRun, DryRunDecision, OperationStatus,
    PolicyHealth, PolicyView, Principal, ServiceHealth,
};
use crate::ledger::{Ledger, RETENTION};
use crate::policy_source::{LoadedPolicy, PolicyDigest, SharedPolicySource};
use denon_avr_domain::{OperationId, ReceiverId, ReceiverIntent, ReceiverState, WallTime};
use denon_avr_policy::{Decision, Effect as Verdict, PolicyInput};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;
use tracing::warn;

/// How long an audit append or read may take before it counts as failed, so a
/// stalled disk cannot hold a receiver lease or block shutdown.
const AUDIT_TIMEOUT: Duration = Duration::from_secs(5);

/// The window the write cap counts over.
const WRITE_WINDOW: Duration = Duration::from_secs(60);

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
struct Gate {
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
    gate: Mutex<Gate>,
    /// Whether the last audit append succeeded.
    audit_ok: AtomicBool,
    /// One rebuild of the ledger at a time.
    rebuilding: tokio::sync::Mutex<()>,
    writes: Mutex<HashMap<AgentLabel, VecDeque<Instant>>>,
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
            gate: Mutex::new(Gate::default()),
            audit_ok: AtomicBool::new(true),
            rebuilding: tokio::sync::Mutex::new(()),
            writes: Mutex::new(HashMap::new()),
        };
        state.load_policy().await;
        state.ensure_ledger().await;
        state
    }

    // ---- Policy ----

    /// Read the policy again. A load that fails replaces the policy in force
    /// with nothing, so a bad edit never leaves the old rules quietly in place.
    pub(super) async fn load_policy(&self) -> PolicyView {
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
            ledger_ready: locked(&self.gate).ready,
            approval: ApprovalHealth::Unavailable,
        }
    }

    /// What an agent's write needs before the receiver is opened: a policy to
    /// judge it by and a ledger to count it in. The text is for the agent.
    pub(super) async fn precheck(&self) -> Result<Arc<ActivePolicy>, &'static str> {
        let policy = match &*locked(&self.policy) {
            PolicyState::Active(policy) => Arc::clone(policy),
            PolicyState::Unavailable(_) => return Err(POLICY_UNAVAILABLE),
        };
        if !self.ensure_ledger().await {
            return Err(LEDGER_UNAVAILABLE);
        }
        Ok(policy)
    }

    // ---- Audit ----

    fn record(
        &self,
        operation: Option<OperationId>,
        principal: Principal,
        receiver: Option<ReceiverId>,
        event: AuditEvent,
    ) -> AuditRecord {
        AuditRecord {
            schema: AUDIT_SCHEMA,
            run: self.run,
            at: self.clock.now(),
            operation,
            principal,
            receiver,
            event,
        }
    }

    /// Append a record, bounded in time. The outcome is the audit health.
    async fn append(&self, record: AuditRecord, durability: Durability) -> Result<(), ()> {
        let outcome =
            tokio::time::timeout(AUDIT_TIMEOUT, self.audit.append(record, durability)).await;
        let ok = matches!(outcome, Ok(Ok(())));
        self.audit_ok.store(ok, Ordering::SeqCst);
        if !ok {
            match outcome {
                Ok(Err(error)) => warn!(%error, "appending to the audit log"),
                _ => warn!("appending to the audit log timed out"),
            }
            return Err(());
        }
        Ok(())
    }

    /// A page of the log for the Operator. The error does not carry the log's
    /// own text.
    pub(super) async fn audit_page(&self, query: AuditQuery) -> Result<AuditPage, ControlError> {
        match tokio::time::timeout(AUDIT_TIMEOUT, self.audit.query(query)).await {
            Ok(Ok(page)) => Ok(page),
            Ok(Err(error)) => {
                warn!(%error, "reading the audit log");
                Err(ControlError::Unavailable(
                    "the audit log cannot be read".into(),
                ))
            }
            Err(_) => Err(ControlError::Unavailable(
                "the audit log did not answer".into(),
            )),
        }
    }

    /// Record how an agent's operation ended.
    pub(super) async fn record_finished(
        &self,
        id: OperationId,
        label: &AgentLabel,
        receiver: &ReceiverId,
        resolution: &Resolution,
    ) {
        let record = self.record(
            Some(id),
            Principal::Agent(label.clone()),
            Some(receiver.clone()),
            AuditEvent::Finished {
                status: resolution.status.as_str().into(),
                dispatch: resolution.dispatch.clone(),
                confirmed: resolution.confirmed,
                reason: resolution.reason.clone(),
            },
        );
        let _ = self.append(record, Durability::Flushed).await;
    }

    // ---- Ledger ----

    /// Make sure the ledger has been rebuilt from the audit log, trying again if
    /// an earlier attempt failed. What was counted while it was not ready is kept.
    async fn ensure_ledger(&self) -> bool {
        if locked(&self.gate).ready {
            return true;
        }
        let _one_at_a_time = self.rebuilding.lock().await;
        if locked(&self.gate).ready {
            return true;
        }
        let now = self.clock.now();
        let read = tokio::time::timeout(
            AUDIT_TIMEOUT,
            self.audit.since(now.saturating_sub(RETENTION)),
        )
        .await;
        match read {
            Ok(Ok(records)) => {
                let mut rebuilt = Ledger::rebuild(&records, now);
                let mut gate = locked(&self.gate);
                rebuilt.merge(std::mem::take(&mut gate.ledger));
                gate.ledger = rebuilt;
                gate.ready = true;
                true
            }
            Ok(Err(error)) => {
                warn!(%error, "rebuilding the budget ledger from the audit log");
                false
            }
            Err(_) => {
                warn!("rebuilding the budget ledger timed out");
                false
            }
        }
    }

    // ---- Caps ----

    /// Admit a new write by `label`, or refuse it. The caller holds the
    /// operations lock and has already ruled out a retry of something that
    /// exists, which is not a new write.
    pub(super) fn admit_write(
        &self,
        label: &AgentLabel,
        operations: &Operations,
    ) -> Result<(), ControlError> {
        let owner = Principal::Agent(label.clone());
        let unfinished = operations
            .entries
            .values()
            .filter(|entry| entry.owner == owner && !entry.snapshot.borrow().status.is_terminal())
            .count();
        if unfinished >= self.limits.unfinished_operations {
            return Err(ControlError::RateLimited { retry_after: None });
        }
        self.charge_write(label)
    }

    /// Count a write by `label` against its allowance for the last minute.
    pub(super) fn charge_write(&self, label: &AgentLabel) -> Result<(), ControlError> {
        let now = Instant::now();
        let mut writes = locked(&self.writes);
        let recent = writes.entry(label.clone()).or_default();
        while recent
            .front()
            .is_some_and(|at| now.duration_since(*at) >= WRITE_WINDOW)
        {
            recent.pop_front();
        }
        if recent.len() >= self.limits.writes_per_minute as usize {
            let retry_after = recent.front().map_or(WRITE_WINDOW, |oldest| {
                WRITE_WINDOW - now.duration_since(*oldest)
            });
            return Err(ControlError::RateLimited {
                retry_after: Some(retry_after),
            });
        }
        recent.push_back(now);
        Ok(())
    }

    // ---- Evaluation ----

    /// Judge `intent` for `agent` against the receiver's state and the ledger.
    fn evaluate(
        &self,
        policy: &ActivePolicy,
        agent: &str,
        receiver: &ReceiverId,
        intent: &ReceiverIntent,
        state: &ReceiverState,
    ) -> Decision {
        let now = self.clock.now();
        let gate = locked(&self.gate);
        let recent = gate
            .ledger
            .recent(receiver, now, policy.loaded.config.longest_window());
        denon_avr_policy::evaluate(
            &PolicyInput {
                agent,
                receiver,
                intent,
                state,
                recent: &recent,
                now,
            },
            &policy.loaded.config,
        )
    }
}

/// The record of a decision.
fn decided(intent: &ReceiverIntent, decision: &Decision, digest: PolicyDigest) -> AuditEvent {
    AuditEvent::Decided {
        intent: intent_text(intent),
        decision: match decision.effect() {
            Verdict::Allow => AuditDecision::Allow,
            Verdict::RequireApproval => AuditDecision::RequireApproval,
            Verdict::Deny => AuditDecision::Deny,
        },
        reasons: reasons(decision),
        rules: decision.rules().to_vec(),
        baseline: decision
            .baseline()
            .map(|baseline| baseline.fields().collect())
            .unwrap_or_default(),
        policy: Some(digest),
    }
}

fn reasons(decision: &Decision) -> Vec<String> {
    decision.reasons().iter().map(ToString::to_string).collect()
}

/// The limits that fired, as one sentence for the agent. No rule ids.
fn reason_text(decision: &Decision) -> String {
    let reasons = reasons(decision);
    if reasons.is_empty() {
        "the policy does not allow this".into()
    } else {
        reasons.join("; ")
    }
}

/// Decide an agent's request and end it. A request the policy refuses or holds
/// ends here as `denied` or `approval_unavailable`. One it allows ends
/// `rejected` until the gate dispatches.
pub(super) async fn decide(
    agent: &AgentState,
    lease: &Lease,
    id: OperationId,
    receiver: &ReceiverId,
    intent: &ReceiverIntent,
    label: &AgentLabel,
    policy: &ActivePolicy,
) -> Resolution {
    let state = lease.session.state().latest();
    let decision = agent.evaluate(policy, label.as_str(), receiver, intent, &state);
    let record = agent.record(
        Some(id),
        Principal::Agent(label.clone()),
        Some(receiver.clone()),
        decided(intent, &decision, policy.loaded.digest),
    );
    let _ = agent.append(record, Durability::Flushed).await;
    match decision.effect() {
        Verdict::Deny => {
            Resolution::not_dispatched(OperationStatus::Denied, reason_text(&decision))
        }
        Verdict::RequireApproval => {
            Resolution::not_dispatched(OperationStatus::ApprovalUnavailable, reason_text(&decision))
        }
        Verdict::Allow => {
            Resolution::not_dispatched(OperationStatus::Rejected, "agent dispatch is not enabled")
        }
    }
}

/// What the policy would decide for `label`, without making an operation or a
/// record. It opens the receiver to read its state.
pub(super) async fn dry_run(
    inner: &Arc<Inner>,
    agent: &AgentState,
    label: &str,
    receiver: &ReceiverId,
    intent: &ReceiverIntent,
) -> Result<DryRun, ControlError> {
    let policy = match agent.precheck().await {
        Ok(policy) => policy,
        Err(reason) => {
            return Ok(DryRun {
                decision: DryRunDecision::Unavailable {
                    reason: reason.into(),
                },
                policy: agent.digest(),
            })
        }
    };
    let lease = inner.lease(receiver).await?;
    let state = lease.session.state().latest();
    let decision = agent.evaluate(&policy, label, receiver, intent, &state);
    Ok(DryRun {
        decision: match decision.effect() {
            Verdict::Allow => DryRunDecision::Allow,
            Verdict::RequireApproval => DryRunDecision::RequireApproval {
                reasons: reasons(&decision),
                rules: decision.rules().to_vec(),
            },
            Verdict::Deny => DryRunDecision::Deny {
                reasons: reasons(&decision),
                rules: decision.rules().to_vec(),
            },
        },
        policy: Some(policy.loaded.digest),
    })
}

/// What an agent is told when the gate cannot reach the receiver.
pub(super) const RECEIVER_UNREACHABLE: &str = "the receiver could not be reached";

/// The gate's refusals, as the service ends the operation with.
pub(super) fn refusal(reason: &'static str) -> Resolution {
    Resolution::not_dispatched(OperationStatus::Rejected, reason)
}
