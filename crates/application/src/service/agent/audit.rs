//! The Agent path's audit records: how they are built, the bounded appends, the
//! Operator's page of the log, and the record of a decision.

use super::AgentState;
use crate::audit::{
    intent_text, AuditDecision, AuditEvent, AuditPage, AuditQuery, AuditRecord, Durability,
    AUDIT_SCHEMA,
};
use crate::control::{ControlError, Principal};
use crate::policy_source::PolicyDigest;
use crate::service::{locked, Resolution};
use denon_avr_domain::{CoreField, OperationId, ReceiverId, ReceiverIntent, ReceiverState};
use denon_avr_policy::{observed_volume, Decision, Effect as Verdict, Level};
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::time::Instant;
use tracing::warn;

/// How long an audit append or read may take before it counts as failed, so a
/// stalled disk cannot hold a receiver lease or block shutdown.
const AUDIT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long an Operator's audit append may take. The Operator's controls are not
/// held up by the log, so this is much shorter than an agent's.
pub(super) const OPERATOR_AUDIT_TIMEOUT: Duration = Duration::from_secs(1);

/// How often a refusal made before any decision is written to the log.
const REFUSAL_LOG_EVERY: Duration = Duration::from_secs(60);

impl AgentState {
    // ---- Audit ----

    pub(super) fn record(
        &self,
        operation: Option<OperationId>,
        principal: Principal,
        receiver: Option<ReceiverId>,
        event: AuditEvent,
    ) -> AuditRecord {
        self.record_as(operation, Some(principal), receiver, event)
    }

    /// A record whose actor may be unknown: a caller with no valid credential.
    pub(super) fn record_as(
        &self,
        operation: Option<OperationId>,
        principal: Option<Principal>,
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

    /// Record that the Operator issued or revoked a token. Best effort and short,
    /// like the Operator's other records: a failing log does not stop it.
    pub(in crate::service) async fn record_token_event(&self, event: AuditEvent) {
        let record = self.record(None, Principal::Operator, None, event);
        let _ = self
            .append_within(record, Durability::Flushed, OPERATOR_AUDIT_TIMEOUT)
            .await;
    }

    /// Append a record, bounded in time. The outcome is the audit health.
    pub(super) async fn append(
        &self,
        record: AuditRecord,
        durability: Durability,
    ) -> Result<(), ()> {
        self.append_within(record, durability, AUDIT_TIMEOUT).await
    }

    pub(super) async fn append_within(
        &self,
        record: AuditRecord,
        durability: Durability,
        limit: Duration,
    ) -> Result<(), ()> {
        let outcome = tokio::time::timeout(limit, self.audit.append(record, durability)).await;
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
    pub(in crate::service) async fn audit_page(
        &self,
        query: AuditQuery,
    ) -> Result<AuditPage, ControlError> {
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
    pub(in crate::service) async fn record_finished(
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
        // The Operator is not held up by the log, so its bound is short.
        let limit = match owner {
            Principal::Operator => OPERATOR_AUDIT_TIMEOUT,
            Principal::Agent(_) => AUDIT_TIMEOUT,
        };
        let _ = self.append_within(record, Durability::Flushed, limit).await;
    }

    /// Record a refusal made before any decision, because there was no policy to
    /// decide by or no history to count in. A client that keeps asking would
    /// otherwise write a line per attempt and push real records out of the log,
    /// so the first is written, then one a minute that says how many were not.
    pub(in crate::service) async fn record_refused(
        &self,
        id: OperationId,
        owner: &Principal,
        receiver: &ReceiverId,
        resolution: &Resolution,
    ) {
        let skipped = {
            let now = Instant::now();
            let mut refusals = locked(&self.refusals);
            let due = refusals
                .0
                .is_none_or(|last| now.duration_since(last) >= REFUSAL_LOG_EVERY);
            if !due {
                refusals.1 += 1;
                return;
            }
            refusals.0 = Some(now);
            std::mem::take(&mut refusals.1)
        };
        let mut resolution = resolution.clone();
        if skipped > 0 {
            let reason = resolution.reason.take().unwrap_or_default();
            resolution.reason = Some(format!(
                "{reason} ({skipped} more refused since the last record)"
            ));
        }
        self.record_finished(id, owner, receiver, &resolution).await;
    }

    pub(super) fn dispatching(
        &self,
        intent: &ReceiverIntent,
        state: &ReceiverState,
        precondition_fields: Vec<CoreField>,
    ) -> AuditEvent {
        let (before, target) = match intent {
            ReceiverIntent::Volume(target) => (
                observed_volume(state).map(Level::half_steps),
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

/// The record of a decision.
pub(super) fn decided(
    intent: &ReceiverIntent,
    decision: &Decision,
    digest: PolicyDigest,
) -> AuditEvent {
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

pub(super) fn reasons(decision: &Decision) -> Vec<String> {
    decision.reasons().iter().map(ToString::to_string).collect()
}

/// The limits that fired, as one sentence for the agent. No rule ids.
pub(super) fn reason_text(decision: &Decision) -> String {
    let reasons = reasons(decision);
    if reasons.is_empty() {
        "the policy does not allow this".into()
    } else {
        reasons.join("; ")
    }
}
