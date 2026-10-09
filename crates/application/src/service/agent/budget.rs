//! The budget: the ledger rebuilt from the audit log, the write caps, and the
//! atomic step that evaluates a request and reserves its place in the ledger.

use super::{ActivePolicy, AgentState, Books};
use crate::control::{AgentLabel, ControlError, Principal};
use crate::ledger::{Ledger, OpKey, RETENTION};
use crate::service::{locked, Operations};
use denon_avr_domain::{
    OperationId, Precondition, ReceiverId, ReceiverIntent, ReceiverState, WallTime,
};
use denon_avr_policy::{observed_volume, Decision, Level, PolicyInput, RecentChange};
use std::time::Duration;
use tokio::time::Instant;
use tracing::warn;

/// How long reading the audit history to rebuild the ledger may take. It reads a
/// day of records, which takes longer than appending one.
const REBUILD_TIMEOUT: Duration = Duration::from_secs(30);

/// More labels than this are swept for ones that have gone quiet.
const SWEEP_LABELS_ABOVE: usize = 64;

/// The window the write cap counts over.
const WRITE_WINDOW: Duration = Duration::from_secs(60);

impl AgentState {
    // ---- Ledger ----

    /// Make sure the ledger has been rebuilt from the audit log, trying again if
    /// an earlier attempt failed. What was counted while it was not ready is kept.
    pub(super) async fn ensure_ledger(&self) -> bool {
        if locked(&self.books).ready {
            return true;
        }
        let _one_at_a_time = self.rebuilding.lock().await;
        if locked(&self.books).ready {
            return true;
        }
        let now = self.clock.now();
        let read = tokio::time::timeout(
            REBUILD_TIMEOUT,
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
    pub(in crate::service) fn admit_write(
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
    pub(in crate::service) fn charge_write(&self, label: &AgentLabel) -> Result<(), ControlError> {
        let now = Instant::now();
        let mut writes = locked(&self.writes);
        if writes.len() > SWEEP_LABELS_ABOVE {
            // Forget the labels that have not written for a minute.
            writes.retain(|_, recent| {
                recent
                    .back()
                    .is_some_and(|at| now.duration_since(*at) < WRITE_WINDOW)
            });
        }
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

    /// How many labels the write cap is tracking.
    #[cfg(test)]
    pub(in crate::service) fn tracked_labels(&self) -> usize {
        locked(&self.writes).len()
    }

    /// How many entries the ledger holds, however old.
    #[cfg(test)]
    pub(in crate::service) fn stored(&self) -> usize {
        locked(&self.books).ledger.stored()
    }

    /// How many volume changes the ledger counts for `receiver`.
    #[cfg(test)]
    pub(in crate::service) fn counted(&self, receiver: &ReceiverId) -> usize {
        let now = self.clock.now();
        locked(&self.books)
            .ledger
            .recent(receiver, now, RETENTION)
            .len()
    }

    // ---- Evaluation ----

    /// Judge `intent` for `agent` against the receiver's state and the ledger.
    pub(super) fn evaluate(
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
    pub(super) fn judge(
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
    pub(super) fn reserve(
        &self,
        gate: &mut Books,
        now: WallTime,
        receiver: &ReceiverId,
        intent: &ReceiverIntent,
        state: &ReceiverState,
        id: OperationId,
    ) -> Settlement {
        // Entries last a day, so a service that runs for weeks does not keep, or
        // scan, every write it has ever made.
        gate.ledger.prune(now);
        let key = match intent {
            ReceiverIntent::Volume(target) => {
                let key = OpKey::new(self.run, id);
                gate.ledger.reserve(
                    receiver,
                    key,
                    RecentChange {
                        at: now,
                        before: observed_volume(state),
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
    pub(in crate::service) fn settle(&self, settlement: &Settlement, dispatched: bool) {
        if let Some(key) = settlement.key {
            locked(&self.books)
                .ledger
                .settle(&settlement.receiver, key, dispatched);
        }
    }
}

/// What the atomic step found.
pub(super) enum Judged {
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
pub(in crate::service) struct Settlement {
    receiver: ReceiverId,
    key: Option<OpKey>,
}
