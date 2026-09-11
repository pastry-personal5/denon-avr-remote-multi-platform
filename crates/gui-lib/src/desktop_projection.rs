//! Headless projection of canonical receiver evidence for the desktop.
//!
//! This reducer has no Iced or transport dependency.  It is intentionally
//! small so interaction tests can prove that a local draft or historical
//! operation result never masquerades as receiver state.

use denon_avr_domain::{
    Epoch, MasterVolume, MonotonicMillis, OperationId, OperationOutcome,
    ReceiverFieldState as FieldState, ReceiverFieldValidity as FieldValidity, ReceiverId,
    ReceiverIntent, ReceiverState, StaleReason, SystemPower, ZonePower,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresentedValidity {
    Unknown,
    Current,
    CurrentConverging,
    Stale(StaleReason),
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentedField<T> {
    pub observed: Option<T>,
    pub validity: PresentedValidity,
    pub observed_at: Option<MonotonicMillis>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingOperation {
    pub id: OperationId,
    pub intent: ReceiverIntent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPresentation<T> {
    pub observed: PresentedField<T>,
    pub draft: Option<T>,
    pub pending: Option<PendingOperation>,
    pub last_outcome: Option<OperationOutcome>,
}

impl<T> ControlPresentation<T> {
    fn new(observed: PresentedField<T>) -> Self {
        Self {
            observed,
            draft: None,
            pending: None,
            last_outcome: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopProjection {
    pub selected_receiver: Option<ReceiverId>,
    pub state: Option<ReceiverState>,
    pub system_power: PresentedField<SystemPower>,
    pub main_zone_power: ControlPresentation<ZonePower>,
    pub volume: ControlPresentation<MasterVolume>,
    pub last_revision: u64,
    last_epoch: Option<Epoch>,
}

impl Default for DesktopProjection {
    fn default() -> Self {
        Self {
            selected_receiver: None,
            state: None,
            system_power: PresentedField {
                observed: None,
                validity: PresentedValidity::Unknown,
                observed_at: None,
            },
            main_zone_power: ControlPresentation::new(PresentedField {
                observed: None,
                validity: PresentedValidity::Unknown,
                observed_at: None,
            }),
            volume: ControlPresentation::new(PresentedField {
                observed: None,
                validity: PresentedValidity::Unknown,
                observed_at: None,
            }),
            last_revision: 0,
            last_epoch: None,
        }
    }
}

impl DesktopProjection {
    pub fn select_receiver(&mut self, receiver: ReceiverId) {
        if self.selected_receiver.as_ref() != Some(&receiver) {
            self.selected_receiver = Some(receiver);
            self.state = None;
            self.last_revision = 0;
            self.last_epoch = None;
            self.main_zone_power = ControlPresentation::new(unknown());
            self.volume = ControlPresentation::new(unknown());
            self.system_power = unknown();
        }
    }

    /// Accepts only newer state from the selected receiver.  Epoch and
    /// revision are supplied by the canonical owner; presentation never
    /// invents correlation IDs for receiver events.
    pub fn accept_state(&mut self, state: ReceiverState) -> bool {
        if self.selected_receiver.as_ref() != Some(&state.receiver)
            || state
                .epoch
                .zip(self.last_epoch)
                .is_some_and(|(epoch, last)| epoch < last)
            || (state.epoch == self.last_epoch && state.revision.0 <= self.last_revision)
        {
            return false;
        }
        self.last_revision = state.revision.0;
        if state.epoch.is_some() {
            self.last_epoch = state.epoch;
        }
        self.system_power = present(&state.system_power);
        self.main_zone_power.observed = present(&state.main_zone.power);
        self.volume.observed = present(&state.main_zone.volume);
        self.state = Some(state);
        true
    }

    pub fn begin_volume_draft(&mut self, value: MasterVolume) {
        self.volume.draft = Some(value);
    }

    pub fn submit_volume(&mut self, id: OperationId) -> Option<ReceiverIntent> {
        let value = self.volume.draft?;
        let intent = ReceiverIntent::Volume(value);
        self.volume.pending = Some(PendingOperation {
            id,
            intent: intent.clone(),
        });
        Some(intent)
    }

    pub fn record_outcome(&mut self, outcome: OperationOutcome) {
        let id = outcome.operation();
        if self
            .volume
            .pending
            .as_ref()
            .is_some_and(|pending| pending.id == id)
        {
            self.volume.pending = None;
            self.volume.draft = None;
            self.volume.last_outcome = Some(outcome);
        } else if self
            .main_zone_power
            .pending
            .as_ref()
            .is_some_and(|pending| pending.id == id)
        {
            self.main_zone_power.pending = None;
            self.main_zone_power.last_outcome = Some(outcome);
        }
    }
}

fn unknown<T>() -> PresentedField<T> {
    PresentedField {
        observed: None,
        validity: PresentedValidity::Unknown,
        observed_at: None,
    }
}
fn present<T: Clone>(field: &FieldState<T>) -> PresentedField<T> {
    let observed = field
        .last_good
        .as_ref()
        .map(|observation| observation.value.clone());
    let observed_at = field
        .last_good
        .as_ref()
        .map(|observation| observation.observed_at);
    let validity = match &field.validity {
        FieldValidity::Unknown => PresentedValidity::Unknown,
        FieldValidity::Current { .. } => match field.synchronization {
            denon_avr_domain::FieldSynchronization::Converging { .. } => {
                PresentedValidity::CurrentConverging
            }
            _ => PresentedValidity::Current,
        },
        FieldValidity::Stale { reason } => PresentedValidity::Stale(reason.clone()),
        FieldValidity::Unavailable { .. } => PresentedValidity::Unavailable,
    };
    PresentedField {
        observed,
        validity,
        observed_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use denon_avr_domain::{CoreFrame, DispatchCertainty, Epoch, FrameSeq, ObservationOrigin};

    fn state(id: &ReceiverId, volume: MasterVolume) -> ReceiverState {
        let mut state = ReceiverState::new(id.clone());
        state.establish_epoch(Epoch(1));
        state.reduce(
            id,
            Epoch(1),
            FrameSeq(1),
            MonotonicMillis(1),
            MonotonicMillis(100),
            ObservationOrigin::ReceiverFrame,
            CoreFrame::Volume(volume),
        );
        state
    }

    #[test]
    fn draft_and_historical_outcome_do_not_replace_external_observation() {
        let id = ReceiverId::new("living-room").unwrap();
        let mut projection = DesktopProjection::default();
        projection.select_receiver(id.clone());
        projection.accept_state(state(&id, MasterVolume::db_half_steps(-20).unwrap()));
        let requested = MasterVolume::db_half_steps(-18).unwrap();
        projection.begin_volume_draft(requested);
        assert_eq!(
            projection.volume.observed.observed,
            Some(MasterVolume::db_half_steps(-20).unwrap())
        );
        let mut newer = state(&id, MasterVolume::db_half_steps(-10).unwrap());
        newer.revision.0 = 3;
        projection.accept_state(newer);
        assert_eq!(
            projection.volume.observed.observed,
            Some(MasterVolume::db_half_steps(-10).unwrap())
        );
        assert_eq!(projection.volume.draft, Some(requested));
        let intent = projection.submit_volume(OperationId(4)).unwrap();
        assert_eq!(intent, ReceiverIntent::Volume(requested));
        projection.record_outcome(OperationOutcome::ObservedRequestedValue {
            operation: OperationId(4),
            dispatch: DispatchCertainty::CompleteWrite,
            observation: "observed after dispatch".into(),
        });
        assert_eq!(
            projection.volume.observed.observed,
            Some(MasterVolume::db_half_steps(-10).unwrap())
        );
        assert!(projection.volume.draft.is_none());
    }

    #[test]
    fn old_receiver_state_is_rejected_after_switch() {
        let first = ReceiverId::new("first").unwrap();
        let second = ReceiverId::new("second").unwrap();
        let mut projection = DesktopProjection::default();
        projection.select_receiver(first.clone());
        assert!(projection.accept_state(state(&first, MasterVolume::Minimum)));
        projection.select_receiver(second.clone());
        assert!(!projection.accept_state(state(&first, MasterVolume::db_half_steps(0).unwrap())));
        assert!(projection.state.is_none());
    }

    #[test]
    fn delayed_old_epoch_is_rejected_even_with_a_higher_local_revision() {
        let id = ReceiverId::new("living-room").unwrap();
        let mut projection = DesktopProjection::default();
        projection.select_receiver(id.clone());
        let mut current = state(&id, MasterVolume::Minimum);
        current.revision.0 = 10;
        projection.accept_state(current);
        let mut old = state(&id, MasterVolume::db_half_steps(0).unwrap());
        old.revision.0 = 11;
        old.epoch = Some(Epoch(0));
        assert!(!projection.accept_state(old));
    }
}
