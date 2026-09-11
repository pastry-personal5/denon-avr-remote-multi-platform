//! Reviewed, deterministic raw-frame replay for the X3800H core profile.

use denon_avr_domain::{
    Epoch, FrameSeq, MasterVolume, MonotonicMillis, ObservationOrigin, ReceiverId, ReceiverState,
    ZonePower,
};
use denon_avr_infrastructure::x3800h_reducer::core_frame;
use denon_avr_protocol::avr::parse_x3800h;

#[test]
fn reviewed_trace_replays_typed_frames_and_ignores_unknown_wire_data() {
    let receiver = ReceiverId::new("trace-x3800h").unwrap();
    let mut state = ReceiverState::new(receiver.clone());
    state.establish_epoch(Epoch(7));

    let frames = [
        "PWON", "ZMON", "Z2OFF", "SICD", "MV805", "MUOFF", "MSSTEREO", "ZZFUTURE", "MV---",
    ];
    for (index, raw) in frames.into_iter().enumerate() {
        let parsed = parse_x3800h(raw).unwrap();
        if let Some(frame) = core_frame(parsed) {
            state.reduce(
                &receiver,
                Epoch(7),
                FrameSeq(index as u64 + 1),
                MonotonicMillis(index as u64),
                MonotonicMillis(10_000 + index as u64),
                ObservationOrigin::ReceiverFrame,
                frame,
            );
        }
    }

    assert_eq!(state.epoch, Some(Epoch(7)));
    assert_eq!(state.revision.0, 9);
    assert_eq!(
        state.main_zone.power.last_good.as_ref().unwrap().value,
        ZonePower::On
    );
    assert_eq!(
        state
            .main_zone
            .source
            .last_good
            .as_ref()
            .unwrap()
            .value
            .as_str(),
        "CD"
    );
    assert_eq!(
        state.main_zone.volume.last_good.as_ref().unwrap().value,
        MasterVolume::db_half_steps(1).unwrap()
    );
    assert!(matches!(
        state.main_zone.volume.validity,
        denon_avr_domain::ReceiverFieldValidity::Unavailable { .. }
    ));
    assert_eq!(
        state.zone2_power.last_good.as_ref().unwrap().value,
        ZonePower::Off
    );
}

#[test]
fn replay_rejects_malformed_typed_frames_without_panicking() {
    assert!(parse_x3800h("MV985").is_err());
    assert!(parse_x3800h("MV8x").is_err());
    assert!(parse_x3800h("SI").is_err());
    assert!(matches!(
        parse_x3800h("MS"),
        Ok(denon_avr_protocol::avr::X3800hFrame::Unknown(_))
    ));
}
