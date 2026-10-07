//! Headless projection of canonical receiver evidence for the desktop.
//!
//! The control-service port delivers a complete `ReceiverState` with per-field
//! validity. The views read the established display model (`MainZoneSnapshot`
//! and `Zone2Snapshot`), so this module is the one place that turns one into
//! the other. It has no Iced or transport dependency and is tested headless.
//!
//! The rule it applies is the one the adapter applied in 3.0.0: a field is
//! usable only when its validity is `Current`. A stale value is reported as
//! unavailable and never offered as something to act on.

use denon_avr_domain::{
    FieldError, FieldErrorKind, FieldStatus, Freshness, Input, MainZoneSnapshot, MasterVolume,
    MuteState, PowerState, ReceiverFieldState, ReceiverFieldValidity, ReceiverIntent,
    ReceiverState, SoundModeCategory, SoundModeIntent, StaleReason, StateAuthority, SurroundMode,
    Volume, VolumeLevel, Zone2Snapshot, ZonePower,
};
use denon_avr_domain::{MainZoneControl, SourceId};

/// What the views display for one receiver state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Projected {
    pub snapshot: MainZoneSnapshot,
    pub zone2: Zone2Snapshot,
}

/// The message a field carries while nothing has been read. The launch wait and
/// the status wait key on it.
pub const NOT_QUERIED: &str = "not queried";

/// Project `state` into the display model.
pub fn project(state: &ReceiverState) -> Projected {
    let main = &state.main_zone;
    let power = status(&main.power, |power| Some(power_state(*power)));
    let input = status(&main.source, |source| Input::new(source.as_str()).ok());
    let volume = status(&main.volume, |volume| Some(display_volume(*volume)));
    let mute = status(&main.mute, |mute| Some(*mute));
    let surround_mode = status(&main.sound_mode, |mode| {
        SurroundMode::new(mode.id.clone()).ok()
    });
    let any_current = matches!(power, FieldStatus::Value(_))
        || matches!(input, FieldStatus::Value(_))
        || matches!(volume, FieldStatus::Value(_))
        || matches!(mute, FieldStatus::Value(_))
        || matches!(surround_mode, FieldStatus::Value(_));
    let (freshness, authority) = freshness_of(any_current, state);
    let snapshot = MainZoneSnapshot {
        power,
        input,
        volume,
        mute,
        surround_mode,
        freshness,
        authority,
        ..MainZoneSnapshot::default()
    };

    let zone2_power = status(&state.zone2_power, |power| Some(power_state(*power)));
    let (freshness, authority) = freshness_of(matches!(zone2_power, FieldStatus::Value(_)), state);
    let zone2 = Zone2Snapshot {
        power: zone2_power,
        freshness,
        authority,
    };
    Projected { snapshot, zone2 }
}

/// What the display model says about how current a projection is.
fn freshness_of(any_current: bool, state: &ReceiverState) -> (Freshness, StateAuthority) {
    if any_current {
        (Freshness::Live, StateAuthority::Authoritative)
    } else if state.epoch.is_none() {
        (Freshness::Invalidated, StateAuthority::Unconfirmed)
    } else {
        (Freshness::Unknown, StateAuthority::Unconfirmed)
    }
}

fn power_state(power: ZonePower) -> PowerState {
    match power {
        ZonePower::On => PowerState::On,
        ZonePower::Off => PowerState::Standby,
    }
}

fn error(kind: FieldErrorKind, message: impl Into<String>) -> FieldError {
    FieldError {
        kind,
        message: message.into(),
    }
}

fn not_queried() -> FieldError {
    error(FieldErrorKind::Unavailable, NOT_QUERIED)
}

/// One canonical field as the display model's status.
fn status<T, U>(
    field: &ReceiverFieldState<T>,
    convert: impl FnOnce(&T) -> Option<U>,
) -> FieldStatus<U> {
    let issue = || field.last_issue.as_ref().map(|issue| issue.message.clone());
    match &field.validity {
        ReceiverFieldValidity::Current { .. } => field
            .last_good
            .as_ref()
            .and_then(|observation| convert(&observation.value))
            .map(FieldStatus::Value)
            .unwrap_or_else(|| {
                FieldStatus::Unavailable(error(
                    FieldErrorKind::Malformed,
                    "the receiver value could not be shown",
                ))
            }),
        ReceiverFieldValidity::Unknown => FieldStatus::Unavailable(not_queried()),
        // A connection that is gone reads as not yet queried, as it did when the
        // controller invalidated everything on a disconnect: the status wait
        // that follows a reconnect depends on it.
        ReceiverFieldValidity::Stale {
            reason: StaleReason::Disconnected | StaleReason::ReceiverChanged,
        } => FieldStatus::Unavailable(not_queried()),
        ReceiverFieldValidity::Stale {
            reason: StaleReason::QueryFailed,
        } => FieldStatus::Unavailable(error(
            FieldErrorKind::Timeout,
            issue().unwrap_or_else(|| "the receiver query failed".into()),
        )),
        ReceiverFieldValidity::Stale {
            reason: StaleReason::Expired,
        } => FieldStatus::Unavailable(error(
            FieldErrorKind::Timeout,
            issue().unwrap_or_else(|| "the receiver observation expired".into()),
        )),
        ReceiverFieldValidity::Unavailable { .. } => FieldStatus::Unavailable(error(
            FieldErrorKind::Unavailable,
            issue().unwrap_or_else(|| "the receiver reports no value".into()),
        )),
    }
}

/// The display form of a canonical volume. `Minimum` is shown at the bottom of
/// the slider, which is where a request for the minimum puts it.
pub fn display_volume(value: MasterVolume) -> Volume {
    match value {
        MasterVolume::Minimum => Volume::from_parts("MIN", -800),
        MasterVolume::DbHalfSteps(step) => {
            let native = step + 160;
            let code = if native % 2 == 0 {
                format!("{:02}", native / 2)
            } else {
                format!("{:02}5", native / 2)
            };
            Volume::from_parts(code, step * 5)
        }
    }
}

/// The canonical volume for a UI level. Levels the receiver cannot represent
/// are clamped to its range: the slider's bottom is `Minimum` and its top is
/// +18.0 dB. A native code counts half-dB steps in fives (800 is 0 dB and 805 is
/// +0.5 dB), so the half step survives the conversion.
pub fn master_volume(level: VolumeLevel) -> MasterVolume {
    let half_steps = i32::from(level.to_native_code()) / 5 - 160;
    let half_steps = half_steps.min(i32::from(MasterVolume::HIGHEST_HALF_STEP));
    i16::try_from(half_steps)
        .ok()
        .and_then(|steps| MasterVolume::db_half_steps(steps).ok())
        .unwrap_or(MasterVolume::Minimum)
}

/// The intent the receiver is asked for when the user acts on `control`.
pub fn intent_for(control: &MainZoneControl) -> Result<ReceiverIntent, String> {
    Ok(match control {
        MainZoneControl::Power(power) => main_zone_power_intent(*power),
        MainZoneControl::Input(input) => ReceiverIntent::Source(
            SourceId::new(input.as_str()).map_err(|error| format!("receiver source: {error}"))?,
        ),
        MainZoneControl::Volume(level) => ReceiverIntent::Volume(master_volume(*level)),
        MainZoneControl::Mute(mute) => mute_intent(*mute),
        MainZoneControl::SurroundMode(mode) | MainZoneControl::SelectSoundMode { mode, .. } => {
            ReceiverIntent::SoundMode(SoundModeIntent::Select(mode.as_str().into()))
        }
        MainZoneControl::RecallSoundModeCategory(category) => recall_intent(*category),
    })
}

/// The intent for a sound mode category recall.
pub fn recall_intent(category: SoundModeCategory) -> ReceiverIntent {
    ReceiverIntent::SoundMode(match category {
        SoundModeCategory::Movie => SoundModeIntent::RecallMovie,
        SoundModeCategory::Music => SoundModeIntent::RecallMusic,
        SoundModeCategory::Game => SoundModeIntent::RecallGame,
        SoundModeCategory::Pure => SoundModeIntent::PureDirect,
    })
}

pub fn mute_intent(mute: MuteState) -> ReceiverIntent {
    ReceiverIntent::Mute(mute)
}

pub fn main_zone_power_intent(power: PowerState) -> ReceiverIntent {
    ReceiverIntent::MainZonePower(match power {
        PowerState::On => ZonePower::On,
        PowerState::Standby => ZonePower::Off,
    })
}

pub fn zone2_power_intent(power: PowerState) -> ReceiverIntent {
    ReceiverIntent::Zone2Power(match power {
        PowerState::On => ZonePower::On,
        PowerState::Standby => ZonePower::Off,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use denon_avr_domain::{
        CoreFrame, Epoch, FrameSeq, MonotonicMillis, ObservationOrigin, ReceiverId,
        SoundModeStatus, SourceId,
    };

    fn state() -> (ReceiverState, ReceiverId) {
        let id = ReceiverId::new("living-room").unwrap();
        let mut state = ReceiverState::new(id.clone());
        state.establish_epoch(Epoch(1));
        (state, id)
    }

    fn feed(state: &mut ReceiverState, id: &ReceiverId, seq: u64, frame: CoreFrame) {
        state.reduce(
            id,
            Epoch(1),
            FrameSeq(seq),
            MonotonicMillis(seq),
            MonotonicMillis(10_000),
            ObservationOrigin::ReceiverFrame,
            frame,
        );
    }

    #[test]
    fn nothing_read_yet_is_not_queried_and_is_not_a_value() {
        let (state, _) = state();
        let projected = project(&state);
        fn message<T>(field: &FieldStatus<T>) -> String {
            match field {
                FieldStatus::Unavailable(error) => error.message.clone(),
                FieldStatus::Value(_) => panic!("no value was read"),
            }
        }
        assert_eq!(message(&projected.snapshot.power), NOT_QUERIED);
        assert_eq!(message(&projected.snapshot.mute), NOT_QUERIED);
        assert_eq!(message(&projected.zone2.power), NOT_QUERIED);
        assert_eq!(projected.snapshot.freshness, Freshness::Unknown);
    }

    #[test]
    fn current_fields_become_the_values_the_views_read() {
        let (mut state, id) = state();
        feed(&mut state, &id, 1, CoreFrame::MainZonePower(ZonePower::On));
        feed(
            &mut state,
            &id,
            2,
            CoreFrame::Source(SourceId::new("GAME").unwrap()),
        );
        feed(
            &mut state,
            &id,
            3,
            CoreFrame::Volume(MasterVolume::db_half_steps(-90).unwrap()),
        );
        feed(&mut state, &id, 4, CoreFrame::Mute(MuteState::Off));
        feed(
            &mut state,
            &id,
            5,
            CoreFrame::SoundMode(SoundModeStatus {
                id: "DOLBY SURROUND".into(),
                raw: "MSDOLBY SURROUND".into(),
            }),
        );
        feed(&mut state, &id, 6, CoreFrame::Zone2Power(ZonePower::Off));
        let projected = project(&state);
        assert_eq!(projected.snapshot.power.value(), Some(&PowerState::On));
        assert_eq!(projected.snapshot.input.value().unwrap().as_str(), "GAME");
        assert_eq!(projected.snapshot.volume.value().unwrap().db_tenths(), -450);
        assert_eq!(projected.snapshot.mute.value(), Some(&MuteState::Off));
        assert_eq!(
            projected.snapshot.surround_mode.value().unwrap().as_str(),
            "DOLBY SURROUND"
        );
        assert_eq!(projected.zone2.power.value(), Some(&PowerState::Standby));
        assert_eq!(projected.snapshot.freshness, Freshness::Live);
    }

    #[test]
    fn a_disconnect_keeps_no_usable_value_and_reads_as_not_queried_again() {
        let (mut state, id) = state();
        feed(&mut state, &id, 1, CoreFrame::MainZonePower(ZonePower::On));
        state.mark_disconnected();
        let projected = project(&state);
        match &projected.snapshot.power {
            FieldStatus::Unavailable(error) => assert_eq!(error.message, NOT_QUERIED),
            FieldStatus::Value(_) => panic!("a stale value must not be usable"),
        }
        assert_eq!(projected.snapshot.freshness, Freshness::Invalidated);
    }

    #[test]
    fn an_expired_observation_is_unavailable_with_a_reason() {
        let (mut state, id) = state();
        feed(&mut state, &id, 1, CoreFrame::Mute(MuteState::On));
        assert!(state.expire_fields(MonotonicMillis(20_000)));
        match project(&state).snapshot.mute {
            FieldStatus::Unavailable(error) => {
                assert_eq!(error.kind, FieldErrorKind::Timeout);
                assert!(error.message.contains("expired"), "{}", error.message);
            }
            FieldStatus::Value(_) => panic!("an expired value must not be usable"),
        }
    }

    #[test]
    fn a_failed_query_reports_the_issue_and_keeps_the_last_value_out_of_use() {
        let (mut state, id) = state();
        feed(&mut state, &id, 1, CoreFrame::Mute(MuteState::On));
        state.mark_query_failed(denon_avr_domain::CoreField::Mute, "no answer");
        match project(&state).snapshot.mute {
            FieldStatus::Unavailable(error) => assert_eq!(error.message, "no answer"),
            FieldStatus::Value(_) => panic!("a failed query must not leave a usable value"),
        }
    }

    #[test]
    fn volume_converts_at_the_ends_and_in_the_middle() {
        assert_eq!(display_volume(MasterVolume::Minimum).db_tenths(), -800);
        let zero = display_volume(MasterVolume::db_half_steps(0).unwrap());
        assert_eq!((zero.code(), zero.db_tenths()), ("80", 0));
        let half = display_volume(MasterVolume::db_half_steps(-159).unwrap());
        assert_eq!((half.code(), half.db_tenths()), ("005", -795));
        let top = display_volume(MasterVolume::db_half_steps(36).unwrap());
        assert_eq!((top.code(), top.db_tenths()), ("98", 180));
    }

    #[test]
    fn slider_levels_map_to_the_receivers_range_and_round_trip() {
        let level = |native: u16| VolumeLevel::from_native_code(native).unwrap();
        // The slider's bottom is Minimum.
        assert_eq!(master_volume(level(0)), MasterVolume::Minimum);
        // -79.5 dB is the first real step.
        assert_eq!(
            master_volume(level(5)),
            MasterVolume::db_half_steps(-159).unwrap()
        );
        assert_eq!(
            master_volume(level(800)),
            MasterVolume::db_half_steps(0).unwrap()
        );
        // A half step is kept, where 3.0.0 rounded it down to the whole dB.
        assert_eq!(
            master_volume(level(805)),
            MasterVolume::db_half_steps(1).unwrap()
        );
        // A level above the receiver's +18.0 dB, which the slider cannot offer
        // but a stored level could, clamps to it.
        assert_eq!(
            master_volume(level(985)),
            MasterVolume::db_half_steps(36).unwrap()
        );
        // What the receiver reports is what a request for it asks for.
        for step in [-159, -100, -1, 0, 1, 35, 36] {
            let volume = MasterVolume::db_half_steps(step).unwrap();
            let shown = display_volume(volume);
            assert_eq!(master_volume(shown.level().unwrap()), volume, "{step}");
        }
    }

    #[test]
    fn category_recalls_map_to_their_intents() {
        assert_eq!(
            recall_intent(SoundModeCategory::Pure),
            ReceiverIntent::SoundMode(SoundModeIntent::PureDirect)
        );
        assert_eq!(
            recall_intent(SoundModeCategory::Game),
            ReceiverIntent::SoundMode(SoundModeIntent::RecallGame)
        );
    }
}
