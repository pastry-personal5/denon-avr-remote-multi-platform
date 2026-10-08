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
use crate::ledger::{Ledger, OpKey, RETENTION};
use crate::policy_source::{LoadedPolicy, PolicyDigest, SharedPolicySource};
use denon_avr_domain::{
    CoreField, DispatchCertainty, FieldBaseline, FieldValue, OperationId, Precondition, ReceiverId,
    ReceiverIntent, ReceiverState, WallTime,
};
use denon_avr_policy::{Decision, Effect as Verdict, Level, PolicyInput, RecentChange};
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
const AUDIT_UNAVAILABLE: &str = "audit log unavailable";
const NO_EPOCH: &str = "the receiver's state is not established";

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
            books: Mutex::new(Books::default()),
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
            ledger_ready: locked(&self.books).ready,
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

    /// Record how an operation ended.
    pub(super) async fn record_finished(
        &self,
        id: OperationId,
        owner: &Principal,
        receiver: &ReceiverId,
        resolution: &Resolution,
    ) {
        let record = self.record(
            Some(id),
            owner.clone(),
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
        if locked(&self.books).ready {
            return true;
        }
        let _one_at_a_time = self.rebuilding.lock().await;
        if locked(&self.books).ready {
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
                let mut gate = locked(&self.books);
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

    /// How many volume changes the ledger counts for `receiver`.
    #[cfg(test)]
    pub(super) fn counted(&self, receiver: &ReceiverId) -> usize {
        let now = self.clock.now();
        locked(&self.books)
            .ledger
            .recent(receiver, now, RETENTION)
            .len()
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
        let gate = locked(&self.books);
        Self::evaluate_in(&gate, now, policy, agent, receiver, intent, state)
    }

    fn evaluate_in(
        gate: &Books,
        now: WallTime,
        policy: &ActivePolicy,
        agent: &str,
        receiver: &ReceiverId,
        intent: &ReceiverIntent,
        state: &ReceiverState,
    ) -> Decision {
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

    /// The atomic step. Under the ledger lock, with nothing awaited, it reads
    /// the ledger, evaluates, binds an allow to the state it read, and reserves
    /// the volume change in the ledger. Another request can only see the ledger
    /// before this one or after it, so two agents each within the budget cannot
    /// together exceed it.
    fn judge(
        &self,
        policy: &ActivePolicy,
        label: &AgentLabel,
        receiver: &ReceiverId,
        intent: &ReceiverIntent,
        state: &ReceiverState,
        id: OperationId,
    ) -> Judged {
        let now = self.clock.now();
        let mut gate = locked(&self.books);
        let decision =
            Self::evaluate_in(&gate, now, policy, label.as_str(), receiver, intent, state);
        let Some(baseline) = (match &decision {
            Decision::Allow { baseline } => Some(baseline),
            _ => None,
        }) else {
            return Judged::Refused(decision);
        };
        // The baseline names every field the decision read, matched or not, so
        // a change in any of them before the write voids it.
        let Some(precondition) = Precondition::capture(state, baseline.fields()) else {
            return Judged::NoEpoch(decision);
        };
        let settlement = self.reserve(&mut gate, now, receiver, intent, state, id);
        Judged::Allowed {
            decision,
            precondition,
            settlement,
        }
    }

    /// Count a volume change from now on, if `intent` is one.
    fn reserve(
        &self,
        gate: &mut Books,
        now: WallTime,
        receiver: &ReceiverId,
        intent: &ReceiverIntent,
        state: &ReceiverState,
        id: OperationId,
    ) -> Settlement {
        let key = match intent {
            ReceiverIntent::Volume(target) => {
                let key = OpKey::new(self.run, id);
                gate.ledger.reserve(
                    receiver,
                    key,
                    RecentChange {
                        at: now,
                        before: observed_level(state),
                        target: Level::of(*target),
                    },
                );
                Some(key)
            }
            _ => None,
        };
        Settlement {
            receiver: receiver.clone(),
            key,
        }
    }

    /// The operation ended. What may have been written stays counted; the rest
    /// is released.
    pub(super) fn settle(&self, settlement: &Settlement, dispatched: bool) {
        if let Some(key) = settlement.key {
            locked(&self.books)
                .ledger
                .settle(&settlement.receiver, key, dispatched);
        }
    }

    /// An Operator write on a service with an audit log: counted in the ledger
    /// and recorded, never limited, and never held up by a failing log.
    pub(super) async fn begin_operator_write(
        &self,
        lease: &Lease,
        id: OperationId,
        receiver: &ReceiverId,
        intent: &ReceiverIntent,
    ) -> Settlement {
        let state = lease.session.state().latest();
        let settlement = {
            let now = self.clock.now();
            let mut gate = locked(&self.books);
            self.reserve(&mut gate, now, receiver, intent, &state, id)
        };
        let decided = AuditEvent::Decided {
            intent: intent_text(intent),
            decision: AuditDecision::Allow,
            reasons: Vec::new(),
            rules: Vec::new(),
            baseline: Vec::new(),
            policy: None,
        };
        let record = self.record(
            Some(id),
            Principal::Operator,
            Some(receiver.clone()),
            decided,
        );
        let _ = self.append(record, Durability::Flushed).await;
        let record = self.record(
            Some(id),
            Principal::Operator,
            Some(receiver.clone()),
            self.dispatching(intent, &state, Vec::new()),
        );
        let _ = self.append(record, Durability::Synced).await;
        settlement
    }

    fn dispatching(
        &self,
        intent: &ReceiverIntent,
        state: &ReceiverState,
        precondition_fields: Vec<CoreField>,
    ) -> AuditEvent {
        let (before, target) = match intent {
            ReceiverIntent::Volume(target) => (
                observed_level(state).map(Level::half_steps),
                Some(Level::of(*target).half_steps()),
            ),
            _ => (None, None),
        };
        AuditEvent::Dispatching {
            intent: intent_text(intent),
            before,
            target,
            precondition_fields,
        }
    }
}

/// The volume the receiver shows, or `None` when it is stale, unknown, or
/// unavailable.
fn observed_level(state: &ReceiverState) -> Option<Level> {
    match FieldBaseline::capture(state, CoreField::Volume) {
        FieldBaseline::Value(FieldValue::Volume(volume)) => Some(Level::of(volume)),
        _ => None,
    }
}

/// What the atomic step found.
enum Judged {
    /// The policy denies the request or holds it for approval.
    Refused(Decision),
    /// Allowed, but the receiver has no established state to bind the write to.
    NoEpoch(Decision),
    Allowed {
        decision: Decision,
        precondition: Precondition,
        settlement: Settlement,
    },
}

/// A place reserved in the ledger for a write, to settle when it ends.
pub(super) struct Settlement {
    receiver: ReceiverId,
    key: Option<OpKey>,
}

/// An agent's request, as the gate holds it.
#[derive(Clone, Copy)]
pub(super) struct Request<'a> {
    pub(super) id: OperationId,
    pub(super) receiver: &'a ReceiverId,
    pub(super) intent: &'a ReceiverIntent,
    pub(super) label: &'a AgentLabel,
}

/// What the gate does with an agent's request after the policy has seen it.
pub(super) enum Gate {
    /// It ends here, with this resolution.
    Stop(Resolution),
    /// Call the session with this precondition, and settle when it answers.
    Dispatch {
        precondition: Precondition,
        settlement: Settlement,
    },
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

/// Decide an agent's request. A request the policy refuses or holds ends here
/// as `denied` or `approval_unavailable`. One it allows is recorded, twice, and
/// handed back to the gate to dispatch with the precondition it was judged under.
/// If the records cannot be written, it ends `rejected` and nothing is sent: a
/// write with no record of it is worse than no write.
pub(super) async fn decide(
    inner: &Inner,
    agent: &AgentState,
    lease: &Lease,
    request: &Request<'_>,
    policy: &ActivePolicy,
) -> Gate {
    let Request {
        id,
        receiver,
        intent,
        label,
    } = *request;
    let state = lease.session.state().latest();
    let principal = Principal::Agent(label.clone());
    let decided_record = |decision: &Decision| {
        agent.record(
            Some(id),
            principal.clone(),
            Some(receiver.clone()),
            decided(intent, decision, policy.loaded.digest),
        )
    };

    match agent.judge(policy, label, receiver, intent, &state, id) {
        Judged::Refused(decision) => {
            let _ = agent
                .append(decided_record(&decision), Durability::Flushed)
                .await;
            let status = match decision.effect() {
                Verdict::Deny => OperationStatus::Denied,
                _ => OperationStatus::ApprovalUnavailable,
            };
            Gate::Stop(Resolution::not_dispatched(status, reason_text(&decision)))
        }
        Judged::NoEpoch(decision) => {
            let _ = agent
                .append(decided_record(&decision), Durability::Flushed)
                .await;
            Gate::Stop(refusal(NO_EPOCH))
        }
        Judged::Allowed {
            decision,
            precondition,
            settlement,
        } => {
            // From here the request can be withdrawn. If a cancel already won,
            // nothing was sent and nothing stays counted.
            let moved = inner
                .transition(id, |snapshot| {
                    if snapshot.status == OperationStatus::Submitted {
                        snapshot.status = OperationStatus::Allowed;
                        true
                    } else {
                        false
                    }
                })
                .is_some();
            if !moved {
                agent.settle(&settlement, false);
                return Gate::Stop(cancelled());
            }

            if agent
                .append(decided_record(&decision), Durability::Flushed)
                .await
                .is_err()
            {
                agent.settle(&settlement, false);
                return Gate::Stop(refusal(AUDIT_UNAVAILABLE));
            }
            if inner.status_of(id) != Some(OperationStatus::Allowed) {
                agent.settle(&settlement, false);
                return Gate::Stop(cancelled());
            }

            // The record that the write may happen, on disk before it does.
            let dispatching = agent.record(
                Some(id),
                principal.clone(),
                Some(receiver.clone()),
                agent.dispatching(intent, &state, precondition.fields().collect()),
            );
            if agent.append(dispatching, Durability::Synced).await.is_err() {
                agent.settle(&settlement, false);
                return Gate::Stop(refusal(AUDIT_UNAVAILABLE));
            }
            Gate::Dispatch {
                precondition,
                settlement,
            }
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

/// An operation withdrawn before the session was called.
pub(super) fn cancelled() -> Resolution {
    Resolution {
        status: OperationStatus::Cancelled,
        dispatch: DispatchCertainty::NotDispatched,
        confirmed: false,
        reason: None,
        observation: None,
    }
}
