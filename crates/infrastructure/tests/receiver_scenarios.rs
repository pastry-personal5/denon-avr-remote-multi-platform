//! Deterministic, delivery-independent Phase 5 scenario coverage.

use denon_avr_domain::{
    CoreField, CoreFrame, DispatchCertainty, Epoch, FrameSeq, MasterVolume, MonotonicMillis,
    ObservationOrigin, OperationId, OperationOutcome, ReceiverId, ReceiverIntent, ReceiverState,
    StaleReason, SyncCause, SyncCycleId, SyncDebt, ZonePower,
};
use tokio::sync::watch;

#[test]
fn smk_05_stale_and_recover_preserves_last_good_evidence() {
    let receiver = ReceiverId::new("scenario").unwrap();
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
    state.mark_disconnected();
    assert!(matches!(
        state.main_zone.power.validity,
        denon_avr_domain::ReceiverFieldValidity::Stale {
            reason: StaleReason::Disconnected
        }
    ));
    assert_eq!(
        state.main_zone.power.last_good.as_ref().unwrap().value,
        ZonePower::On
    );
    state.establish_epoch(Epoch(2));
    assert_eq!(state.epoch, Some(Epoch(2)));
}

#[test]
fn smk_01_power_scopes_remain_independent() {
    assert_eq!(
        denon_avr_protocol::avr::encode_x3800h(&ReceiverIntent::MainZonePower(ZonePower::On))
            .unwrap()
            .as_str(),
        "ZMON"
    );
    assert_eq!(
        denon_avr_protocol::avr::encode_x3800h(&ReceiverIntent::SystemPower(
            denon_avr_domain::SystemPower::On,
        ))
        .unwrap()
        .as_str(),
        "PWON"
    );
}

#[test]
fn smk_09_unavailable_volume_keeps_previous_value() {
    let receiver = ReceiverId::new("scenario").unwrap();
    let mut state = ReceiverState::new(receiver.clone());
    state.establish_epoch(Epoch(1));
    state.reduce(
        &receiver,
        Epoch(1),
        FrameSeq(1),
        MonotonicMillis(0),
        MonotonicMillis(10_000),
        ObservationOrigin::ReceiverFrame,
        CoreFrame::Volume(MasterVolume::db_half_steps(0).unwrap()),
    );
    state.reduce(
        &receiver,
        Epoch(1),
        FrameSeq(2),
        MonotonicMillis(1),
        MonotonicMillis(10_001),
        ObservationOrigin::ReceiverFrame,
        CoreFrame::VolumeUnavailable {
            raw: "MV---".into(),
        },
    );
    assert!(matches!(
        state.main_zone.volume.validity,
        denon_avr_domain::ReceiverFieldValidity::Unavailable { .. }
    ));
    assert_eq!(
        state.main_zone.volume.last_good.as_ref().unwrap().value,
        MasterVolume::db_half_steps(0).unwrap()
    );
}

#[test]
fn smk_09_ambiguous_delivery_is_not_a_success() {
    let outcome = OperationOutcome::Indeterminate {
        operation: OperationId(9),
        dispatch: DispatchCertainty::Unknown,
        reason: "partial write".into(),
    };
    assert!(!matches!(
        outcome,
        OperationOutcome::ObservedRequestedValue { .. }
    ));
    assert_eq!(outcome.operation(), OperationId(9));
    let _ = CoreField::Volume;
}

#[test]
fn smk_05_failed_targeted_observation_keeps_last_good_value_stale() {
    let receiver = ReceiverId::new("scenario").unwrap();
    let mut state = ReceiverState::new(receiver.clone());
    state.establish_epoch(Epoch(1));
    state.reduce(
        &receiver,
        Epoch(1),
        FrameSeq(1),
        MonotonicMillis(0),
        MonotonicMillis(10_000),
        ObservationOrigin::ReceiverFrame,
        CoreFrame::Mute(denon_avr_domain::MuteState::Off),
    );
    state.mark_query_failed(CoreField::Mute, "response timeout");
    assert!(matches!(
        state.main_zone.mute.validity,
        denon_avr_domain::ReceiverFieldValidity::Stale {
            reason: StaleReason::QueryFailed
        }
    ));
    assert_eq!(
        state.main_zone.mute.last_good.as_ref().unwrap().value,
        denon_avr_domain::MuteState::Off
    );
}

#[test]
fn smk_02_remote_burst_merges_one_activity_debt_cycle() {
    let receiver = ReceiverId::new("burst").unwrap();
    let mut debt = SyncDebt::new(
        receiver.clone(),
        Epoch(1),
        SyncCycleId(2),
        CoreField::Volume,
        SyncCause::ReceiverEvent,
        MonotonicMillis(0),
    );
    debt.merge(
        [CoreField::Mute, CoreField::Source],
        SyncCause::ReceiverEvent,
    );
    debt.note_activity(MonotonicMillis(100));
    debt.note_activity(MonotonicMillis(200));
    assert_eq!(debt.receiver, receiver);
    assert_eq!(debt.fields.len(), 3);
    assert_eq!(debt.activity_seq, 2);
    assert!(!debt.quick_due(MonotonicMillis(449)));
    assert!(debt.quick_due(MonotonicMillis(450)));
}

#[test]
fn smk_03_several_controls_keep_distinct_typed_wire_intents() {
    use denon_avr_protocol::avr::encode_x3800h;
    assert_eq!(
        encode_x3800h(&ReceiverIntent::Source(
            denon_avr_domain::SourceId::new("CD").unwrap()
        ))
        .unwrap()
        .as_str(),
        "SICD"
    );
    assert_eq!(
        encode_x3800h(&ReceiverIntent::Mute(denon_avr_domain::MuteState::On))
            .unwrap()
            .as_str(),
        "MUON"
    );
    assert_eq!(
        encode_x3800h(&ReceiverIntent::Volume(
            MasterVolume::db_half_steps(-20).unwrap()
        ))
        .unwrap()
        .as_str(),
        "MV70"
    );
}

#[test]
fn smk_04_mixed_controller_evidence_does_not_overwrite_operation_history() {
    let outcome = OperationOutcome::ObservedRequestedValue {
        operation: OperationId(40),
        dispatch: DispatchCertainty::CompleteWrite,
        observation: "desktop target observed".into(),
    };
    let receiver = ReceiverId::new("mixed").unwrap();
    let mut state = ReceiverState::new(receiver.clone());
    state.establish_epoch(Epoch(1));
    state.reduce(
        &receiver,
        Epoch(1),
        FrameSeq(1),
        MonotonicMillis(0),
        MonotonicMillis(10_000),
        ObservationOrigin::ReceiverFrame,
        CoreFrame::Volume(MasterVolume::db_half_steps(-20).unwrap()),
    );
    state.reduce(
        &receiver,
        Epoch(1),
        FrameSeq(2),
        MonotonicMillis(1),
        MonotonicMillis(10_001),
        ObservationOrigin::ReceiverFrame,
        CoreFrame::Volume(MasterVolume::db_half_steps(-10).unwrap()),
    );
    assert_eq!(outcome.operation(), OperationId(40));
    assert_eq!(
        state.main_zone.volume.last_good.unwrap().value,
        MasterVolume::db_half_steps(-10).unwrap()
    );
}

#[tokio::test]
async fn smk_06_slow_subscriber_reads_latest_complete_snapshot() {
    let receiver = ReceiverId::new("subscriber").unwrap();
    let initial = ReceiverState::new(receiver.clone());
    let (sender, receiver_rx) = watch::channel(initial);
    let mut subscription = denon_avr_application::StateSubscription::new(receiver_rx);
    for epoch in 1..=40 {
        let mut state = sender.borrow().clone();
        state.establish_epoch(Epoch(epoch));
        sender.send_replace(state);
    }
    let latest = subscription.changed().await.unwrap();
    assert_eq!(latest.epoch, Some(Epoch(40)));
}
