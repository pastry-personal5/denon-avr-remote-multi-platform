//! What a decision saw about the receiver, checked again before a write.
//!
//! A policy decision, or a user's approval, rests on a baseline: the receiver
//! epoch and the value of every field the decision consulted. The session
//! re-observes those fields just before it writes and refuses the write if the
//! baseline no longer holds. Operator operations carry no precondition.

use crate::{
    CoreField, Epoch, MasterVolume, MuteState, ReceiverFieldValidity as FieldValidity,
    ReceiverState, SourceId, SystemPower, ZonePower,
};
use std::collections::BTreeMap;

/// The typed value of one core field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValue {
    SystemPower(SystemPower),
    MainZonePower(ZonePower),
    Zone2Power(ZonePower),
    Source(SourceId),
    Volume(MasterVolume),
    Mute(MuteState),
    /// The receiver's sound-mode id, as observed.
    SoundMode(String),
}

impl FieldValue {
    pub fn field(&self) -> CoreField {
        match self {
            Self::SystemPower(_) => CoreField::SystemPower,
            Self::MainZonePower(_) => CoreField::MainZonePower,
            Self::Zone2Power(_) => CoreField::Zone2Power,
            Self::Source(_) => CoreField::Source,
            Self::Volume(_) => CoreField::Volume,
            Self::Mute(_) => CoreField::Mute,
            Self::SoundMode(_) => CoreField::SoundMode,
        }
    }
}

/// What a decision saw for one field: its value, or the fact that no usable
/// value existed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldBaseline {
    Value(FieldValue),
    /// The field was stale, unknown, or unavailable. Unusable data is never
    /// presented as a value.
    NoUsableValue(CoreField),
}

impl FieldBaseline {
    pub fn field(&self) -> CoreField {
        match self {
            Self::Value(value) => value.field(),
            Self::NoUsableValue(field) => *field,
        }
    }

    /// Record what `state` shows for `field`. A value is captured only when the
    /// field's validity is current and it has a last good observation.
    pub fn capture(state: &ReceiverState, field: CoreField) -> Self {
        match field {
            CoreField::SystemPower => usable(&state.system_power, FieldValue::SystemPower),
            CoreField::MainZonePower => usable(&state.main_zone.power, FieldValue::MainZonePower),
            CoreField::Zone2Power => usable(&state.zone2_power, FieldValue::Zone2Power),
            CoreField::Source => usable(&state.main_zone.source, FieldValue::Source),
            CoreField::Volume => usable(&state.main_zone.volume, FieldValue::Volume),
            CoreField::Mute => usable(&state.main_zone.mute, FieldValue::Mute),
            CoreField::SoundMode => usable(&state.main_zone.sound_mode, |mode| {
                FieldValue::SoundMode(mode.id)
            }),
        }
        .map_or(Self::NoUsableValue(field), Self::Value)
    }

    /// Whether `state` still shows what this baseline recorded. A baseline of no
    /// usable value holds only while the field still has none, so a field that
    /// was unknown at decision time and is known now no longer matches.
    pub fn holds_in(&self, state: &ReceiverState) -> bool {
        *self == Self::capture(state, self.field())
    }
}

fn usable<T: Clone>(
    field: &crate::ReceiverFieldState<T>,
    value: impl FnOnce(T) -> FieldValue,
) -> Option<FieldValue> {
    match (&field.validity, &field.last_good) {
        (FieldValidity::Current { .. }, Some(observation)) => {
            Some(value(observation.value.clone()))
        }
        _ => None,
    }
}

/// Why a precondition no longer holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreconditionMismatch {
    /// The receiver connection changed, which voids authority from the old one.
    Epoch,
    /// This field no longer shows what the decision saw.
    Field(CoreField),
}

/// The receiver epoch and one baseline per field a decision consulted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Precondition {
    epoch: Epoch,
    fields: BTreeMap<CoreField, FieldBaseline>,
}

impl Precondition {
    pub fn new(epoch: Epoch) -> Self {
        Self {
            epoch,
            fields: BTreeMap::new(),
        }
    }

    /// Add a baseline. A second baseline for the same field replaces the first.
    pub fn with(mut self, baseline: FieldBaseline) -> Self {
        self.fields.insert(baseline.field(), baseline);
        self
    }

    /// The baseline of `fields` as `state` shows them now, or `None` when the
    /// state has no established epoch to bind to.
    pub fn capture(
        state: &ReceiverState,
        fields: impl IntoIterator<Item = CoreField>,
    ) -> Option<Self> {
        let precondition = fields
            .into_iter()
            .fold(Self::new(state.epoch?), |acc, field| {
                acc.with(FieldBaseline::capture(state, field))
            });
        Some(precondition)
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// The fields the precondition names, in a fixed order.
    pub fn fields(&self) -> impl Iterator<Item = CoreField> + '_ {
        self.fields.keys().copied()
    }

    pub fn baseline(&self, field: CoreField) -> Option<&FieldBaseline> {
        self.fields.get(&field)
    }

    /// The first respect in which `state` differs: the epoch first, then each
    /// named field in a fixed order. `None` means the precondition holds.
    pub fn mismatch(&self, state: &ReceiverState) -> Option<PreconditionMismatch> {
        if state.epoch != Some(self.epoch) {
            return Some(PreconditionMismatch::Epoch);
        }
        self.fields
            .iter()
            .find(|(_, baseline)| !baseline.holds_in(state))
            .map(|(field, _)| PreconditionMismatch::Field(*field))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        FrameSeq, MonotonicMillis, ObservationOrigin, ReceiverId, ReceiverObservation, StaleReason,
    };

    fn observed<T>(receiver: &ReceiverId, value: T) -> ReceiverObservation<T> {
        ReceiverObservation {
            receiver: receiver.clone(),
            epoch: Epoch(1),
            frame_seq: FrameSeq(1),
            observed_at: MonotonicMillis(0),
            origin: ObservationOrigin::ReceiverFrame,
            value,
        }
    }

    fn state_with(volume: i16, mute: MuteState) -> ReceiverState {
        let id = ReceiverId::new("living-room").unwrap();
        let mut state = ReceiverState::new(id.clone());
        state.establish_epoch(Epoch(1));
        state.main_zone.volume.observe(
            observed(&id, MasterVolume::db_half_steps(volume).unwrap()),
            MonotonicMillis(1_000),
        );
        state
            .main_zone
            .mute
            .observe(observed(&id, mute), MonotonicMillis(1_000));
        state
    }

    #[test]
    fn a_usable_field_is_captured_as_its_value() {
        let state = state_with(-78, MuteState::Off);
        assert_eq!(
            FieldBaseline::capture(&state, CoreField::Volume),
            FieldBaseline::Value(FieldValue::Volume(
                MasterVolume::db_half_steps(-78).unwrap()
            ))
        );
    }

    #[test]
    fn stale_unknown_and_unavailable_fields_capture_as_no_usable_value() {
        let mut state = state_with(-78, MuteState::Off);
        // Never observed.
        assert_eq!(
            FieldBaseline::capture(&state, CoreField::Zone2Power),
            FieldBaseline::NoUsableValue(CoreField::Zone2Power)
        );
        // Observed once, now stale: the old value must not be presented as usable.
        state
            .main_zone
            .volume
            .stale(StaleReason::Disconnected, "gone");
        assert_eq!(
            FieldBaseline::capture(&state, CoreField::Volume),
            FieldBaseline::NoUsableValue(CoreField::Volume)
        );
        // Unavailable by receiver evidence.
        state.main_zone.mute.validity = FieldValidity::Unavailable {
            evidence: crate::receiver_state::ReceiverEvidence::UnavailableStatus("MUOFF?".into()),
        };
        assert_eq!(
            FieldBaseline::capture(&state, CoreField::Mute),
            FieldBaseline::NoUsableValue(CoreField::Mute)
        );
    }

    #[test]
    fn a_precondition_holds_while_every_named_field_is_unchanged() {
        let state = state_with(-78, MuteState::Off);
        let precondition =
            Precondition::capture(&state, [CoreField::Volume, CoreField::Mute]).unwrap();
        assert_eq!(precondition.epoch(), Epoch(1));
        assert_eq!(
            precondition.fields().collect::<Vec<_>>(),
            vec![CoreField::Volume, CoreField::Mute]
        );
        assert_eq!(precondition.mismatch(&state), None);
    }

    #[test]
    fn a_changed_field_is_named_and_unnamed_fields_are_ignored() {
        let before = state_with(-78, MuteState::Off);
        let precondition = Precondition::capture(&before, [CoreField::Volume]).unwrap();

        // Mute is not named, so a change there does not void the decision.
        assert_eq!(precondition.mismatch(&state_with(-78, MuteState::On)), None);
        // Volume is named.
        assert_eq!(
            precondition.mismatch(&state_with(-70, MuteState::Off)),
            Some(PreconditionMismatch::Field(CoreField::Volume))
        );
    }

    #[test]
    fn a_different_epoch_voids_the_precondition_before_any_field_is_compared() {
        let state = state_with(-78, MuteState::Off);
        let precondition = Precondition::capture(&state, [CoreField::Volume]).unwrap();
        let mut reconnected = state.clone();
        reconnected.mark_disconnected();
        assert_eq!(
            precondition.mismatch(&reconnected),
            Some(PreconditionMismatch::Epoch)
        );
        reconnected.establish_epoch(Epoch(2));
        assert_eq!(
            precondition.mismatch(&reconnected),
            Some(PreconditionMismatch::Epoch)
        );
    }

    #[test]
    fn no_usable_value_holds_only_while_the_field_stays_unusable() {
        let mut state = state_with(-78, MuteState::Off);
        state
            .main_zone
            .volume
            .stale(StaleReason::QueryFailed, "timeout");
        let precondition = Precondition::capture(&state, [CoreField::Volume]).unwrap();
        assert_eq!(precondition.mismatch(&state), None);
        // The volume became known since the decision: the decision is void.
        assert_eq!(
            precondition.mismatch(&state_with(-78, MuteState::Off)),
            Some(PreconditionMismatch::Field(CoreField::Volume))
        );
    }

    #[test]
    fn a_value_baseline_does_not_hold_once_the_field_has_no_usable_value() {
        let before = state_with(-78, MuteState::Off);
        let precondition = Precondition::capture(&before, [CoreField::Volume]).unwrap();
        let mut after = before.clone();
        after
            .main_zone
            .volume
            .stale(StaleReason::Expired, "expired");
        assert_eq!(
            precondition.mismatch(&after),
            Some(PreconditionMismatch::Field(CoreField::Volume))
        );
    }

    #[test]
    fn the_first_mismatch_is_reported_in_field_order() {
        let before = state_with(-78, MuteState::Off);
        let precondition =
            Precondition::capture(&before, [CoreField::Mute, CoreField::Volume]).unwrap();
        // Volume sorts before Mute, and both changed.
        assert_eq!(
            precondition.mismatch(&state_with(-70, MuteState::On)),
            Some(PreconditionMismatch::Field(CoreField::Volume))
        );
    }

    #[test]
    fn a_state_without_an_epoch_cannot_be_captured() {
        let state = ReceiverState::new(ReceiverId::new("living-room").unwrap());
        assert_eq!(Precondition::capture(&state, [CoreField::Volume]), None);
    }
}
