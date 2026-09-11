use denon_avr_domain::{
    CoreFrame, Epoch, FrameSeq, MasterVolume, MonotonicMillis, ObservationOrigin, ReceiverId,
    ReceiverState, ZonePower,
};

#[test]
fn every_profile_half_step_round_trips_as_a_valid_domain_value() {
    for step in MasterVolume::LOWEST_HALF_STEP..=MasterVolume::HIGHEST_HALF_STEP {
        assert_eq!(
            MasterVolume::db_half_steps(step).unwrap(),
            MasterVolume::DbHalfSteps(step)
        );
    }
}

#[test]
fn reducer_rejects_frames_from_an_old_epoch_after_reconnect() {
    let receiver = ReceiverId::new("property-receiver").unwrap();
    let mut state = ReceiverState::new(receiver.clone());
    state.establish_epoch(Epoch(2));
    assert!(!state.reduce(
        &receiver,
        Epoch(1),
        FrameSeq(1),
        MonotonicMillis(0),
        MonotonicMillis(100),
        ObservationOrigin::ReceiverFrame,
        CoreFrame::MainZonePower(ZonePower::On),
    ));
    assert!(state.main_zone.power.last_good.is_none());
}
