//! Canonical, receiver-evidenced core state used by the Phase 5 session.
//!
//! This deliberately does not contain transport, wall-clock, or presentation
//! policy.  In particular, an observation and a requested operation are
//! separate facts.

use crate::{MuteState, SourceId};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReceiverId(String);

impl ReceiverId {
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            return Err("receiver ID must be non-empty and contain no control characters");
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Epoch(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct FrameSeq(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct StateRevision(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct OperationId(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SyncCycleId(pub u64);

/// A monotonic timestamp represented in milliseconds so domain tests do not
/// need a runtime clock type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct MonotonicMillis(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationOrigin {
    ReceiverFrame,
    ReceiverFrameInQueryWindow { query: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation<T> {
    pub receiver: ReceiverId,
    pub epoch: Epoch,
    pub frame_seq: FrameSeq,
    pub observed_at: MonotonicMillis,
    pub origin: ObservationOrigin,
    pub value: T,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaleReason {
    Disconnected,
    QueryFailed,
    Expired,
    ReceiverChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiverEvidence {
    UnavailableStatus(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValidity {
    Unknown,
    Current { valid_until: MonotonicMillis },
    Stale { reason: StaleReason },
    Unavailable { evidence: ReceiverEvidence },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SyncCause {
    Initial,
    LocalControl,
    ReceiverEvent,
    PeriodicSweep,
    ManualRefresh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldSynchronization {
    NotStarted,
    Converging {
        cycle: SyncCycleId,
        cause: SyncCause,
        reconcile_by: MonotonicMillis,
    },
    Settled {
        cycle: SyncCycleId,
        observed_at: MonotonicMillis,
    },
    Suspended {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldIssue {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldState<T> {
    pub last_good: Option<Observation<T>>,
    pub validity: FieldValidity,
    pub synchronization: FieldSynchronization,
    pub last_issue: Option<FieldIssue>,
}

impl<T> Default for FieldState<T> {
    fn default() -> Self {
        Self {
            last_good: None,
            validity: FieldValidity::Unknown,
            synchronization: FieldSynchronization::NotStarted,
            last_issue: None,
        }
    }
}

impl<T> FieldState<T> {
    pub fn observe(&mut self, observation: Observation<T>, valid_until: MonotonicMillis) {
        self.last_good = Some(observation);
        self.validity = FieldValidity::Current { valid_until };
        self.last_issue = None;
    }

    pub fn stale(&mut self, reason: StaleReason, message: impl Into<String>) {
        self.validity = FieldValidity::Stale { reason };
        self.last_issue = Some(FieldIssue {
            message: message.into(),
        });
    }

    pub fn expire_if_due(&mut self, now: MonotonicMillis) -> bool {
        let due =
            matches!(self.validity, FieldValidity::Current { valid_until } if now >= valid_until);
        if due {
            self.stale(
                StaleReason::Expired,
                "receiver observation validity expired",
            );
        }
        due
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemPower {
    On,
    Standby,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZonePower {
    On,
    Off,
}

/// Exact X3800H master-volume values. `Minimum` is not -79.5 dB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MasterVolume {
    Minimum,
    DbHalfSteps(i16),
}

impl MasterVolume {
    pub const LOWEST_HALF_STEP: i16 = -159;
    pub const HIGHEST_HALF_STEP: i16 = 36;

    pub fn db_half_steps(value: i16) -> Result<Self, &'static str> {
        if !(Self::LOWEST_HALF_STEP..=Self::HIGHEST_HALF_STEP).contains(&value) {
            return Err("X3800H volume must be Minimum or -79.5 through +18.0 dB");
        }
        Ok(Self::DbHalfSteps(value))
    }
}

impl fmt::Display for MasterVolume {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Minimum => f.write_str("Min"),
            Self::DbHalfSteps(step) => write!(f, "{:.1} dB", *step as f32 / 2.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundModeStatus {
    pub id: String,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoundModeIntent {
    Auto,
    Direct,
    PureDirect,
    Stereo,
    RecallMovie,
    RecallMusic,
    RecallGame,
    Select(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiverIntent {
    SystemPower(SystemPower),
    MainZonePower(ZonePower),
    Zone2Power(ZonePower),
    Source(SourceId),
    Volume(MasterVolume),
    Mute(MuteState),
    SoundMode(SoundModeIntent),
}

/// A core receiver field.  This is intentionally independent of the value a
/// caller wants to write: synchronization debt is about receiver evidence,
/// not desired state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CoreField {
    SystemPower,
    MainZonePower,
    Zone2Power,
    Source,
    Volume,
    Mute,
    SoundMode,
}

impl ReceiverIntent {
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

/// Typed protocol evidence after parsing. Unknown wire frames intentionally do
/// not enter this enum and therefore cannot mutate canonical state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreFrame {
    SystemPower(SystemPower),
    MainZonePower(ZonePower),
    Zone2Power(ZonePower),
    Source(SourceId),
    Volume(MasterVolume),
    Mute(MuteState),
    SoundMode(SoundModeStatus),
    VolumeUnavailable { raw: String },
}

impl CoreFrame {
    pub fn field(&self) -> CoreField {
        match self {
            Self::SystemPower(_) => CoreField::SystemPower,
            Self::MainZonePower(_) => CoreField::MainZonePower,
            Self::Zone2Power(_) => CoreField::Zone2Power,
            Self::Source(_) => CoreField::Source,
            Self::Volume(_) | Self::VolumeUnavailable { .. } => CoreField::Volume,
            Self::Mute(_) => CoreField::Mute,
            Self::SoundMode(_) => CoreField::SoundMode,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchCertainty {
    NotDispatched,
    PossiblyDispatched,
    /// The local write completed; this is not an acknowledgement that the
    /// receiver applied the command.
    CompleteWrite,
    /// Writing began but completion could not be established.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationOutcome {
    AlreadyObserved {
        operation: OperationId,
        observation: String,
    },
    ObservedRequestedValue {
        operation: OperationId,
        dispatch: DispatchCertainty,
        observation: String,
    },
    RejectedBeforeDispatch {
        operation: OperationId,
        reason: String,
    },
    Cancelled {
        operation: OperationId,
    },
    SupersededBeforeDispatch {
        operation: OperationId,
        by: OperationId,
    },
    Indeterminate {
        operation: OperationId,
        dispatch: DispatchCertainty,
        reason: String,
    },
}

impl OperationOutcome {
    pub fn operation(&self) -> OperationId {
        match self {
            Self::AlreadyObserved { operation, .. }
            | Self::ObservedRequestedValue { operation, .. }
            | Self::RejectedBeforeDispatch { operation, .. }
            | Self::Cancelled { operation }
            | Self::SupersededBeforeDispatch { operation, .. }
            | Self::Indeterminate { operation, .. } => *operation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MainZoneState {
    pub power: FieldState<ZonePower>,
    pub source: FieldState<SourceId>,
    pub volume: FieldState<MasterVolume>,
    pub mute: FieldState<MuteState>,
    pub sound_mode: FieldState<SoundModeStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverState {
    pub receiver: ReceiverId,
    pub revision: StateRevision,
    pub epoch: Option<Epoch>,
    pub system_power: FieldState<SystemPower>,
    pub main_zone: MainZoneState,
    pub zone2_power: FieldState<ZonePower>,
    /// Bounded raw diagnostics for well-formed but unsupported frames.
    pub diagnostics: Vec<String>,
}

impl ReceiverState {
    pub fn new(receiver: ReceiverId) -> Self {
        Self {
            receiver,
            revision: StateRevision(0),
            epoch: None,
            system_power: FieldState::default(),
            main_zone: MainZoneState::default(),
            zone2_power: FieldState::default(),
            diagnostics: Vec::new(),
        }
    }

    pub fn record_diagnostic(&mut self, frame: impl Into<String>) {
        const MAX_DIAGNOSTICS: usize = 32;
        self.diagnostics.push(frame.into());
        if self.diagnostics.len() > MAX_DIAGNOSTICS {
            let excess = self.diagnostics.len() - MAX_DIAGNOSTICS;
            self.diagnostics.drain(..excess);
        }
        self.revision.0 = self.revision.0.saturating_add(1);
    }

    pub fn mark_disconnected(&mut self) {
        stale_if_observed(&mut self.system_power);
        stale_if_observed(&mut self.main_zone.power);
        stale_if_observed(&mut self.main_zone.source);
        stale_if_observed(&mut self.main_zone.volume);
        stale_if_observed(&mut self.main_zone.mute);
        stale_if_observed(&mut self.main_zone.sound_mode);
        stale_if_observed(&mut self.zone2_power);
        self.epoch = None;
        self.revision.0 = self.revision.0.saturating_add(1);
    }

    /// Expires only fields whose independent validity window elapsed. Values
    /// remain available through `last_good` for stale-but-useful rendering.
    pub fn expire_fields(&mut self, now: MonotonicMillis) -> bool {
        let mut expired = false;
        expired |= self.system_power.expire_if_due(now);
        expired |= self.main_zone.power.expire_if_due(now);
        expired |= self.main_zone.source.expire_if_due(now);
        expired |= self.main_zone.volume.expire_if_due(now);
        expired |= self.main_zone.mute.expire_if_due(now);
        expired |= self.main_zone.sound_mode.expire_if_due(now);
        expired |= self.zone2_power.expire_if_due(now);
        if expired {
            self.revision.0 = self.revision.0.saturating_add(1);
        }
        expired
    }

    pub fn establish_epoch(&mut self, epoch: Epoch) -> bool {
        if self.epoch.is_some_and(|current| epoch <= current) {
            return false;
        }
        self.epoch = Some(epoch);
        self.revision.0 = self.revision.0.saturating_add(1);
        true
    }

    pub fn mark_converging(
        &mut self,
        field: CoreField,
        cycle: SyncCycleId,
        cause: SyncCause,
        reconcile_by: MonotonicMillis,
    ) {
        self.field_mut(field)
            .set_synchronization(FieldSynchronization::Converging {
                cycle,
                cause,
                reconcile_by,
            });
        self.revision.0 = self.revision.0.saturating_add(1);
    }

    pub fn mark_settled(&mut self, field: CoreField, cycle: SyncCycleId, at: MonotonicMillis) {
        self.field_mut(field)
            .set_synchronization(FieldSynchronization::Settled {
                cycle,
                observed_at: at,
            });
        self.revision.0 = self.revision.0.saturating_add(1);
    }

    /// Records failed reconciliation without discarding the last-good value.
    pub fn mark_query_failed(&mut self, field: CoreField, message: impl Into<String>) {
        let message = message.into();
        match field {
            CoreField::SystemPower => mark_failed(&mut self.system_power, &message),
            CoreField::MainZonePower => mark_failed(&mut self.main_zone.power, &message),
            CoreField::Zone2Power => mark_failed(&mut self.zone2_power, &message),
            CoreField::Source => mark_failed(&mut self.main_zone.source, &message),
            CoreField::Volume => mark_failed(&mut self.main_zone.volume, &message),
            CoreField::Mute => mark_failed(&mut self.main_zone.mute, &message),
            CoreField::SoundMode => mark_failed(&mut self.main_zone.sound_mode, &message),
        }
        self.revision.0 = self.revision.0.saturating_add(1);
    }

    fn field_mut(&mut self, field: CoreField) -> &mut dyn SyncMarker {
        match field {
            CoreField::SystemPower => &mut self.system_power,
            CoreField::MainZonePower => &mut self.main_zone.power,
            CoreField::Zone2Power => &mut self.zone2_power,
            CoreField::Source => &mut self.main_zone.source,
            CoreField::Volume => &mut self.main_zone.volume,
            CoreField::Mute => &mut self.main_zone.mute,
            CoreField::SoundMode => &mut self.main_zone.sound_mode,
        }
    }

    /// Applies evidence only when it belongs to the selected receiver and the
    /// current established connection. This is the pure canonical reducer
    /// used by the session actor.
    #[allow(clippy::too_many_arguments)]
    pub fn reduce(
        &mut self,
        receiver: &ReceiverId,
        epoch: Epoch,
        frame_seq: FrameSeq,
        observed_at: MonotonicMillis,
        valid_until: MonotonicMillis,
        origin: ObservationOrigin,
        frame: CoreFrame,
    ) -> bool {
        if &self.receiver != receiver || self.epoch != Some(epoch) {
            return false;
        }
        macro_rules! observed {
            ($value:expr) => {
                Observation {
                    receiver: receiver.clone(),
                    epoch,
                    frame_seq,
                    observed_at,
                    origin,
                    value: $value,
                }
            };
        }
        match frame {
            CoreFrame::SystemPower(value) => {
                self.system_power.observe(observed!(value), valid_until)
            }
            CoreFrame::MainZonePower(value) => {
                self.main_zone.power.observe(observed!(value), valid_until)
            }
            CoreFrame::Zone2Power(value) => self.zone2_power.observe(observed!(value), valid_until),
            CoreFrame::Source(value) => {
                self.main_zone.source.observe(observed!(value), valid_until)
            }
            CoreFrame::Volume(value) => {
                self.main_zone.volume.observe(observed!(value), valid_until)
            }
            CoreFrame::Mute(value) => self.main_zone.mute.observe(observed!(value), valid_until),
            CoreFrame::SoundMode(value) => self
                .main_zone
                .sound_mode
                .observe(observed!(value), valid_until),
            CoreFrame::VolumeUnavailable { raw } => {
                self.main_zone.volume.validity = FieldValidity::Unavailable {
                    evidence: ReceiverEvidence::UnavailableStatus(raw),
                };
            }
        }
        self.revision.0 = self.revision.0.saturating_add(1);
        true
    }
}

trait SyncMarker {
    fn set_synchronization(&mut self, value: FieldSynchronization);
}

impl<T> SyncMarker for FieldState<T> {
    fn set_synchronization(&mut self, value: FieldSynchronization) {
        self.synchronization = value;
    }
}

fn stale_if_observed<T>(field: &mut FieldState<T>) {
    if field.last_good.is_some() {
        field.stale(StaleReason::Disconnected, "receiver session disconnected");
    }
}

fn mark_failed<T>(field: &mut FieldState<T>, message: &str) {
    if field.last_good.is_some() {
        field.stale(StaleReason::QueryFailed, message);
    } else {
        field.last_issue = Some(FieldIssue {
            message: message.to_owned(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disconnect_preserves_last_good_evidence() {
        let id = ReceiverId::new("living-room").unwrap();
        let mut state = ReceiverState::new(id.clone());
        state.main_zone.power.observe(
            Observation {
                receiver: id,
                epoch: Epoch(1),
                frame_seq: FrameSeq(4),
                observed_at: MonotonicMillis(10),
                origin: ObservationOrigin::ReceiverFrame,
                value: ZonePower::On,
            },
            MonotonicMillis(20),
        );
        state.mark_disconnected();
        assert!(state.main_zone.power.last_good.is_some());
        assert!(matches!(
            state.main_zone.power.validity,
            FieldValidity::Stale {
                reason: StaleReason::Disconnected
            }
        ));
    }
    #[test]
    fn volume_has_no_normalized_out_of_range_value() {
        assert!(MasterVolume::db_half_steps(36).is_ok());
        assert!(MasterVolume::db_half_steps(37).is_err());
    }

    #[test]
    fn old_epoch_evidence_cannot_mutate_current_state() {
        let receiver = ReceiverId::new("living-room").unwrap();
        let mut state = ReceiverState::new(receiver.clone());
        assert!(state.establish_epoch(Epoch(2)));
        assert!(!state.reduce(
            &receiver,
            Epoch(1),
            FrameSeq(1),
            MonotonicMillis(0),
            MonotonicMillis(10),
            ObservationOrigin::ReceiverFrame,
            CoreFrame::MainZonePower(ZonePower::On),
        ));
        assert!(state.main_zone.power.last_good.is_none());
    }

    #[test]
    fn receiver_unavailable_does_not_erase_last_good_value() {
        let receiver = ReceiverId::new("living-room").unwrap();
        let mut state = ReceiverState::new(receiver.clone());
        state.establish_epoch(Epoch(1));
        state.reduce(
            &receiver,
            Epoch(1),
            FrameSeq(1),
            MonotonicMillis(0),
            MonotonicMillis(10),
            ObservationOrigin::ReceiverFrame,
            CoreFrame::Volume(MasterVolume::db_half_steps(-20).unwrap()),
        );
        state.reduce(
            &receiver,
            Epoch(1),
            FrameSeq(2),
            MonotonicMillis(1),
            MonotonicMillis(11),
            ObservationOrigin::ReceiverFrame,
            CoreFrame::VolumeUnavailable {
                raw: "MV---".into(),
            },
        );
        assert_eq!(
            state
                .main_zone
                .volume
                .last_good
                .as_ref()
                .map(|item| item.value),
            Some(MasterVolume::db_half_steps(-20).unwrap())
        );
        assert!(matches!(
            state.main_zone.volume.validity,
            FieldValidity::Unavailable { .. }
        ));
    }

    #[test]
    fn property_sweep_accepts_only_x3800h_half_step_volume_range() {
        for step in -220..=220 {
            let accepted = MasterVolume::db_half_steps(step).is_ok();
            let expected =
                (MasterVolume::LOWEST_HALF_STEP..=MasterVolume::HIGHEST_HALF_STEP).contains(&step);
            assert_eq!(
                accepted, expected,
                "unexpected acceptance for half-step {step}"
            );
        }
    }

    #[test]
    fn validity_expiry_preserves_last_good_evidence() {
        let receiver = ReceiverId::new("living-room").unwrap();
        let mut state = ReceiverState::new(receiver.clone());
        state.establish_epoch(Epoch(1));
        state.reduce(
            &receiver,
            Epoch(1),
            FrameSeq(1),
            MonotonicMillis(10),
            MonotonicMillis(20),
            ObservationOrigin::ReceiverFrame,
            CoreFrame::MainZonePower(ZonePower::On),
        );
        assert!(state.expire_fields(MonotonicMillis(20)));
        assert_eq!(
            state
                .main_zone
                .power
                .last_good
                .as_ref()
                .map(|item| item.value),
            Some(ZonePower::On)
        );
        assert!(matches!(
            state.main_zone.power.validity,
            FieldValidity::Stale {
                reason: StaleReason::Expired
            }
        ));
        assert!(!state.expire_fields(MonotonicMillis(21)));
    }

    #[test]
    fn query_failure_marks_observed_field_stale_without_erasing_value() {
        let receiver = ReceiverId::new("query-failure").unwrap();
        let mut state = ReceiverState::new(receiver.clone());
        state.establish_epoch(Epoch(1));
        state.reduce(
            &receiver,
            Epoch(1),
            FrameSeq(1),
            MonotonicMillis(0),
            MonotonicMillis(10_000),
            ObservationOrigin::ReceiverFrame,
            CoreFrame::MainZonePower(ZonePower::On),
        );
        state.mark_query_failed(CoreField::MainZonePower, "timeout");
        assert!(matches!(
            state.main_zone.power.validity,
            FieldValidity::Stale {
                reason: StaleReason::QueryFailed
            }
        ));
        assert_eq!(
            state.main_zone.power.last_good.unwrap().value,
            ZonePower::On
        );
    }

    #[test]
    fn diagnostics_are_bounded_and_keep_the_newest_unknown_frames() {
        let mut state = ReceiverState::new(ReceiverId::new("diagnostic").unwrap());
        for index in 0..40 {
            state.record_diagnostic(format!("ZZ{index}"));
        }
        assert_eq!(state.diagnostics.len(), 32);
        assert_eq!(state.diagnostics.first().unwrap(), "ZZ8");
        assert_eq!(state.diagnostics.last().unwrap(), "ZZ39");
    }
}
