//! Seeded, delivery-independent state-machine properties for Phase 5.

use denon_avr_domain::{
    CoreFrame, Epoch, FrameSeq, MasterVolume, MonotonicMillis, MuteState, ObservationOrigin,
    ReceiverId, ReceiverState, SourceId, SystemPower, ZonePower,
};

fn frame(seed: u64, step: u64) -> CoreFrame {
    match (seed.wrapping_add(step)) % 6 {
        0 => CoreFrame::SystemPower(SystemPower::On),
        1 => CoreFrame::MainZonePower(ZonePower::On),
        2 => CoreFrame::Zone2Power(ZonePower::Off),
        3 => CoreFrame::Source(SourceId::new("CD").unwrap()),
        4 => CoreFrame::Volume(MasterVolume::db_half_steps(-159 + ((step % 196) as i16)).unwrap()),
        _ => CoreFrame::Mute(MuteState::Off),
    }
}

#[test]
fn seeded_interleavings_preserve_revision_and_last_good_evidence() {
    for seed in 0..64_u64 {
        let receiver = ReceiverId::new(format!("property-{seed}")).unwrap();
        let mut state = ReceiverState::new(receiver.clone());
        assert!(state.establish_epoch(Epoch(1)));
        let mut revision = state.revision;
        let mut sequence = FrameSeq(0);
        for step in 0..256_u64 {
            sequence.0 += 1;
            let now = MonotonicMillis(step * 10);
            let old = state.clone();
            assert!(state.reduce(
                &receiver,
                Epoch(1),
                sequence,
                now,
                MonotonicMillis(now.0 + 1_000),
                ObservationOrigin::ReceiverFrame,
                frame(seed, step),
            ));
            assert!(state.revision > revision);
            revision = state.revision;
            // An accepted frame may only change its own field; unrelated
            // last-good evidence survives arbitrary event interleavings.
            if old.system_power.last_good.is_some() {
                assert!(state.system_power.last_good.is_some());
            }
            if old.zone2_power.last_good.is_some() {
                assert!(state.zone2_power.last_good.is_some());
            }
        }
    }
}

#[test]
fn old_epoch_frames_never_replace_new_epoch_evidence() {
    let receiver = ReceiverId::new("property-epoch").unwrap();
    let mut state = ReceiverState::new(receiver.clone());
    state.establish_epoch(Epoch(1));
    state.reduce(
        &receiver,
        Epoch(1),
        FrameSeq(1),
        MonotonicMillis(0),
        MonotonicMillis(1_000),
        ObservationOrigin::ReceiverFrame,
        CoreFrame::Volume(MasterVolume::db_half_steps(-20).unwrap()),
    );
    state.mark_disconnected();
    state.establish_epoch(Epoch(2));
    let before = state.clone();
    assert!(!state.reduce(
        &receiver,
        Epoch(1),
        FrameSeq(99),
        MonotonicMillis(2),
        MonotonicMillis(1_002),
        ObservationOrigin::ReceiverFrame,
        CoreFrame::Volume(MasterVolume::db_half_steps(10).unwrap()),
    ));
    assert_eq!(state, before);
}
