//! The Operation Gate's table of operations: admission, idempotent retries,
//! retention, the transitions readers are told about, cancellation, and how a
//! session's outcome maps to what clients see.

use super::{locked, Inner, SHUT_DOWN};
use crate::control::{
    ControlError, IdempotencyKey, OperationEvent, OperationSnapshot, OperationStatus,
    OperationSubmission, Principal,
};
use denon_avr_domain::{DispatchCertainty, OperationId, OperationOutcome, ReceiverId};
use std::collections::{HashMap, VecDeque};
use tokio::sync::watch;

/// One operation in the gate's table.
pub(super) struct Entry {
    pub(super) owner: Principal,
    key: Option<IdempotencyKey>,
    pub(super) snapshot: watch::Sender<OperationSnapshot>,
}

#[derive(Default)]
pub(super) struct Operations {
    next_id: u64,
    pub(super) closed: bool,
    pub(super) entries: HashMap<OperationId, Entry>,
    /// Admission order, for evicting the oldest finished operations.
    order: VecDeque<OperationId>,
    keys: HashMap<(Principal, IdempotencyKey), OperationId>,
}

pub(super) enum Admission {
    /// A new operation: the caller must run it.
    New(OperationSnapshot),
    /// An operation that already exists: nothing more to run.
    Existing(OperationSnapshot),
}

/// How an operation ended, in the terms clients see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Resolution {
    pub(super) status: OperationStatus,
    pub(super) dispatch: DispatchCertainty,
    pub(super) confirmed: bool,
    pub(super) reason: Option<String>,
    pub(super) observation: Option<String>,
}

impl Resolution {
    pub(super) fn not_dispatched(status: OperationStatus, reason: impl Into<String>) -> Self {
        Self {
            status,
            dispatch: DispatchCertainty::NotDispatched,
            confirmed: false,
            reason: Some(reason.into()),
            observation: None,
        }
    }
}

/// Map the session's outcome to what clients see, as the design's lifecycle
/// table specifies. The session's report is not reinterpreted.
pub(super) fn resolve(outcome: OperationOutcome) -> Resolution {
    match outcome {
        OperationOutcome::ObservedRequestedValue {
            dispatch,
            observation,
            ..
        } => Resolution {
            status: OperationStatus::Completed,
            dispatch,
            confirmed: true,
            reason: None,
            observation: Some(observation),
        },
        OperationOutcome::AlreadyObserved { observation, .. } => Resolution {
            status: OperationStatus::AlreadyInState,
            dispatch: DispatchCertainty::NotDispatched,
            confirmed: true,
            reason: None,
            observation: Some(observation),
        },
        OperationOutcome::RejectedBeforeDispatch { reason, .. } => {
            Resolution::not_dispatched(OperationStatus::Rejected, reason)
        }
        OperationOutcome::Cancelled { .. } => Resolution {
            status: OperationStatus::Cancelled,
            dispatch: DispatchCertainty::NotDispatched,
            confirmed: false,
            reason: None,
            observation: None,
        },
        OperationOutcome::SupersededBeforeDispatch { by, .. } => Resolution {
            status: OperationStatus::Superseded { by },
            dispatch: DispatchCertainty::NotDispatched,
            confirmed: false,
            reason: None,
            observation: None,
        },
        OperationOutcome::Indeterminate {
            dispatch, reason, ..
        } => Resolution {
            status: OperationStatus::Indeterminate,
            dispatch,
            confirmed: false,
            reason: Some(reason),
            observation: None,
        },
    }
}

fn is_cancellable(status: &OperationStatus) -> bool {
    matches!(
        status,
        OperationStatus::Submitted
            | OperationStatus::Allowed
            | OperationStatus::AwaitingApproval
            | OperationStatus::Approved
    )
}

impl Inner {
    pub(super) fn publish(&self, owner: &Principal, snapshot: &OperationSnapshot) {
        if let Some(events) = locked(&self.events).as_ref() {
            // No reader is not an error.
            let _ = events.send((owner.clone(), OperationEvent::Update(snapshot.clone())));
        }
    }

    /// Record a new operation, or find the existing one a retry refers to.
    pub(super) fn admit(
        &self,
        owner: &Principal,
        receiver: &ReceiverId,
        submission: OperationSubmission,
    ) -> Result<Admission, ControlError> {
        let mut operations = locked(&self.operations);
        if operations.closed {
            return Err(ControlError::Unavailable(SHUT_DOWN.into()));
        }
        if let Some(key) = &submission.idempotency_key {
            if let Some(id) = operations.keys.get(&(owner.clone(), key.clone())) {
                let existing = operations.entries[id].snapshot.borrow().clone();
                if existing.receiver != *receiver || existing.intent != submission.intent {
                    return Err(ControlError::InvalidRequest(
                        "the idempotency key was already used for a different request".into(),
                    ));
                }
                return Ok(Admission::Existing(existing));
            }
        }
        // An identical request already running is the same request retried.
        for entry in operations.entries.values() {
            if entry.owner != *owner {
                continue;
            }
            let existing = entry.snapshot.borrow();
            if !existing.status.is_terminal()
                && existing.receiver == *receiver
                && existing.intent == submission.intent
            {
                return Ok(Admission::Existing(existing.clone()));
            }
        }
        // A new request. An agent's is counted against its caps, which a retry
        // of one that exists (above) is not.
        let status = match owner {
            // An Operator request is evaluated trivially as allowed.
            Principal::Operator => OperationStatus::Allowed,
            // An agent's waits for the policy to decide it.
            Principal::Agent(label) => {
                let agent = self.agent.as_ref().ok_or(ControlError::Forbidden)?;
                agent.admit_write(label, &operations)?;
                OperationStatus::Submitted
            }
        };
        self.evict_finished(&mut operations);
        operations.next_id += 1;
        let id = OperationId(operations.next_id);
        let snapshot = OperationSnapshot {
            id,
            receiver: receiver.clone(),
            intent: submission.intent,
            status,
            dispatch: DispatchCertainty::NotDispatched,
            confirmed: false,
            reason: None,
            observation: None,
        };
        if let Some(key) = &submission.idempotency_key {
            operations.keys.insert((owner.clone(), key.clone()), id);
        }
        operations.entries.insert(
            id,
            Entry {
                owner: owner.clone(),
                key: submission.idempotency_key,
                snapshot: watch::channel(snapshot.clone()).0,
            },
        );
        operations.order.push_back(id);
        self.in_flight.send_modify(|count| *count += 1);
        Ok(Admission::New(snapshot))
    }

    /// Drop the oldest finished operations beyond the retention limit. Operations
    /// still running are never dropped.
    fn evict_finished(&self, operations: &mut Operations) {
        let mut excess = operations
            .entries
            .len()
            .saturating_sub(self.settings.retained_operations.saturating_sub(1));
        if excess == 0 {
            return;
        }
        let mut kept = VecDeque::with_capacity(operations.order.len());
        while let Some(id) = operations.order.pop_front() {
            let finished = operations
                .entries
                .get(&id)
                .is_some_and(|entry| entry.snapshot.borrow().status.is_terminal());
            if excess > 0 && finished {
                if let Some(entry) = operations.entries.remove(&id) {
                    if let Some(key) = entry.key {
                        operations.keys.remove(&(entry.owner, key));
                    }
                }
                excess -= 1;
            } else {
                kept.push_back(id);
            }
        }
        operations.order = kept;
    }

    /// Change an operation and tell its readers. Returns the new snapshot, or
    /// `None` when the change was refused or the operation is gone.
    pub(super) fn transition(
        &self,
        id: OperationId,
        change: impl FnOnce(&mut OperationSnapshot) -> bool,
    ) -> Option<OperationSnapshot> {
        let (owner, snapshot) = {
            let operations = locked(&self.operations);
            let entry = operations.entries.get(&id)?;
            let mut applied = false;
            entry.snapshot.send_if_modified(|snapshot| {
                applied = change(snapshot);
                applied
            });
            if !applied {
                return None;
            }
            let snapshot = entry.snapshot.borrow().clone();
            (entry.owner.clone(), snapshot)
        };
        self.publish(&owner, &snapshot);
        Some(snapshot)
    }

    /// Move an operation into the session unless it was cancelled first. The
    /// move and a cancel are decided under one lock, so exactly one of them wins.
    pub(super) fn begin_session(&self, id: OperationId) -> bool {
        self.transition(id, |snapshot| {
            if snapshot.status == OperationStatus::Allowed {
                snapshot.status = OperationStatus::InSession;
                true
            } else {
                false
            }
        })
        .is_some()
    }

    /// End an operation. Returns whether it was still running: one that was
    /// already over, because a cancel won, is left as it is.
    pub(super) fn finish(&self, id: OperationId, resolution: Resolution) -> bool {
        self.transition(id, |snapshot| {
            if snapshot.status.is_terminal() {
                return false;
            }
            snapshot.status = resolution.status;
            snapshot.dispatch = resolution.dispatch;
            snapshot.confirmed = resolution.confirmed;
            snapshot.reason = resolution.reason;
            snapshot.observation = resolution.observation;
            true
        })
        .is_some()
    }

    /// How an operation that has ended ended, as its snapshot says.
    pub(super) fn ended_as(&self, id: OperationId) -> Option<Resolution> {
        let operations = locked(&self.operations);
        let snapshot = operations.entries.get(&id)?.snapshot.borrow().clone();
        snapshot.status.is_terminal().then_some(Resolution {
            status: snapshot.status,
            dispatch: snapshot.dispatch,
            confirmed: snapshot.confirmed,
            reason: snapshot.reason,
            observation: snapshot.observation,
        })
    }

    /// Where an operation is now, or `None` when it is gone.
    pub(super) fn status_of(&self, id: OperationId) -> Option<OperationStatus> {
        locked(&self.operations)
            .entries
            .get(&id)
            .map(|entry| entry.snapshot.borrow().status.clone())
    }

    /// The operation as `viewer` may see it. The Operator sees every operation;
    /// anyone else sees only their own, and cannot tell a hidden one from a
    /// missing one.
    pub(super) fn watch_operation(
        &self,
        viewer: &Principal,
        id: OperationId,
    ) -> Result<watch::Receiver<OperationSnapshot>, ControlError> {
        let operations = locked(&self.operations);
        operations
            .entries
            .get(&id)
            .filter(|entry| *viewer == Principal::Operator || entry.owner == *viewer)
            .map(|entry| entry.snapshot.subscribe())
            .ok_or(ControlError::NotFound("operation"))
    }

    pub(super) fn cancel(
        &self,
        viewer: &Principal,
        id: OperationId,
    ) -> Result<OperationSnapshot, ControlError> {
        let current = self.watch_operation(viewer, id)?.borrow().clone();
        if !is_cancellable(&current.status) {
            return Err(ControlError::TooLate(Box::new(current)));
        }
        let withdrawn = self.transition(id, |snapshot| {
            if is_cancellable(&snapshot.status) {
                snapshot.status = OperationStatus::Cancelled;
                true
            } else {
                false
            }
        });
        match withdrawn {
            Some(snapshot) => Ok(snapshot),
            // The operation reached the session between the read and the change.
            None => {
                let actual = self.watch_operation(viewer, id)?.borrow().clone();
                Err(ControlError::TooLate(Box::new(actual)))
            }
        }
    }
}
