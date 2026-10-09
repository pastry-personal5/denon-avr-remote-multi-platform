//! The decision procedure: what the gate does with an agent's request, the
//! Operator's write, the dry run, and the fixed texts agents are told.

use super::audit::{decided, reason_text, reasons, OPERATOR_AUDIT_TIMEOUT};
use super::budget::{Judged, Settlement};
use super::{AgentState, POLICY_UNAVAILABLE};
use crate::audit::{intent_text, AuditDecision, AuditEvent, Durability};
use crate::control::{
    AgentLabel, ControlError, DryRun, DryRunDecision, OperationStatus, Principal,
};
use crate::service::{locked, Inner, Lease, Resolution};
use denon_avr_domain::{
    DispatchCertainty, OperationId, OperationOutcome, Precondition, ReceiverId, ReceiverIntent,
    RejectionCause,
};
use denon_avr_policy::{Decision, Effect as Verdict};
use std::sync::atomic::Ordering;
use std::sync::Arc;

const AUDIT_UNAVAILABLE: &str = "audit log unavailable";
const NO_EPOCH: &str = "the receiver's state is not established";

impl AgentState {
    /// An Operator write on a service with an audit log: counted in the ledger
    /// and recorded, never limited, and never held up by a failing log.
    pub(in crate::service) async fn begin_operator_write(
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
        // The Operator's records are best effort, and a log known to be failing is
        // not waited for: the write goes ahead, and the `Finished` record that
        // follows it is what notices the log working again.
        if !self.audit_ok.load(Ordering::SeqCst) {
            return settlement;
        }
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
        if self
            .append_within(record, Durability::Flushed, OPERATOR_AUDIT_TIMEOUT)
            .await
            .is_err()
        {
            return settlement;
        }
        let record = self.record(
            Some(id),
            Principal::Operator,
            Some(receiver.clone()),
            self.dispatching(intent, &state, Vec::new()),
        );
        let _ = self
            .append_within(record, Durability::Synced, OPERATOR_AUDIT_TIMEOUT)
            .await;
        settlement
    }
}

/// An agent's request, as the gate holds it.
#[derive(Clone, Copy)]
pub(in crate::service) struct Request<'a> {
    pub(in crate::service) id: OperationId,
    pub(in crate::service) receiver: &'a ReceiverId,
    pub(in crate::service) intent: &'a ReceiverIntent,
    pub(in crate::service) label: &'a AgentLabel,
}

/// What the gate does with an agent's request after the policy has seen it.
pub(in crate::service) enum Gate {
    /// It ends here, with this resolution.
    Stop(Resolution),
    /// Call the session with this precondition, and settle when it answers.
    Dispatch {
        precondition: Precondition,
        settlement: Settlement,
    },
}

/// Decide an agent's request. A request the policy refuses or holds ends here
/// as `denied` or `approval_unavailable`. One it allows is recorded, twice, and
/// handed back to the gate to dispatch with the precondition it was judged under.
/// If the records cannot be written, it ends `rejected` and nothing is sent: a
/// write with no record of it is worse than no write.
pub(in crate::service) async fn decide(
    inner: &Inner,
    agent: &AgentState,
    lease: &Lease,
    request: &Request<'_>,
) -> Gate {
    let Request {
        id,
        receiver,
        intent,
        label,
    } = *request;
    // The policy in force now, not the one in force before the receiver
    // connected: an edit made meanwhile, a bad one included, decides this.
    let Some(policy) = agent.active_policy() else {
        return Gate::Stop(refusal(POLICY_UNAVAILABLE));
    };
    let policy = &*policy;
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
pub(in crate::service) async fn dry_run(
    inner: &Arc<Inner>,
    agent: &AgentState,
    label: &str,
    receiver: &ReceiverId,
    intent: &ReceiverIntent,
    show_rules: bool,
) -> Result<DryRun, ControlError> {
    let unavailable = |reason: &str| {
        Ok(DryRun {
            decision: DryRunDecision::Unavailable {
                reason: reason.into(),
            },
            policy: agent.digest(),
        })
    };
    if let Err(reason) = agent.precheck().await {
        return unavailable(reason);
    }
    let lease = inner.lease(receiver).await?;
    // The policy in force now, after the receiver has connected.
    let Some(policy) = agent.active_policy() else {
        return unavailable(POLICY_UNAVAILABLE);
    };
    let state = lease.session.state().latest();
    let decision = agent.evaluate(&policy, label, receiver, intent, &state);
    // Rule ids are the Operator's to see; an agent is told the limits.
    let rules = if show_rules {
        decision.rules().to_vec()
    } else {
        Vec::new()
    };
    Ok(DryRun {
        decision: match decision.effect() {
            Verdict::Allow => DryRunDecision::Allow,
            Verdict::RequireApproval => DryRunDecision::RequireApproval {
                reasons: reasons(&decision),
                rules,
            },
            Verdict::Deny => DryRunDecision::Deny {
                reasons: reasons(&decision),
                rules,
            },
        },
        policy: Some(policy.loaded.digest),
    })
}

/// What an agent is told when the gate cannot reach the receiver.
pub(in crate::service) const RECEIVER_UNREACHABLE: &str = "the receiver could not be reached";

/// What an agent is told of a session's answer, in place of the session's own
/// text, which formats the I/O error underneath and can name an address. The
/// audit log and the Operator keep the session's text.
pub(in crate::service) fn fixed_reason(outcome: &OperationOutcome) -> Option<&'static str> {
    match outcome {
        OperationOutcome::RejectedBeforeDispatch { cause, .. } => Some(match cause {
            RejectionCause::UnsupportedIntent => "the receiver does not support this change",
            RejectionCause::ObservationFailed => "the receiver's state could not be read",
            RejectionCause::PreconditionMismatch(_) => {
                "the receiver changed since the request was judged"
            }
            RejectionCause::CommandRefused => "the receiver refused the command",
            RejectionCause::SessionStopped => "the receiver connection closed",
        }),
        OperationOutcome::Indeterminate { .. } => Some("the outcome could not be established"),
        _ => None,
    }
}

/// The gate's refusals, as the service ends the operation with.
pub(in crate::service) fn refusal(reason: &'static str) -> Resolution {
    Resolution::not_dispatched(OperationStatus::Rejected, reason)
}

/// An operation withdrawn before the session was called.
pub(in crate::service) fn cancelled() -> Resolution {
    Resolution {
        status: OperationStatus::Cancelled,
        dispatch: DispatchCertainty::NotDispatched,
        confirmed: false,
        reason: None,
        observation: None,
    }
}
