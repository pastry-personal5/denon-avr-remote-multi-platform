//! Explicitly armed, state-restoring live validation.
//!
//! This target is never part of ordinary checks. It performs only a reversible
//! mute sequence; additional write sequences require a separately reviewed
//! fixture and must not be enabled by accident.

use denon_avr_application::{CanonicalReceiverSession, OperationRequest};
use denon_avr_domain::{MasterVolume, MuteState, OperationId, ReceiverId, ReceiverIntent};
use denon_avr_infrastructure::{AvrSessionConfig, X3800hSession};

#[tokio::test]
#[ignore = "requires explicit write arm and a physical AVC-X3800H"]
async fn armed_live_mute_round_trip_restores_original_state() {
    assert_eq!(
        std::env::var("ALLOW_RECEIVER_WRITES").as_deref(),
        Ok("1"),
        "set ALLOW_RECEIVER_WRITES=1 to arm live writes"
    );
    let ceiling = std::env::var("DENON_X3800H_SAFE_VOLUME_HALF_STEPS")
        .expect("declare DENON_X3800H_SAFE_VOLUME_HALF_STEPS before live writes")
        .parse::<i16>()
        .expect("safe volume ceiling must be an integer half-step value");
    assert!((-159..=36).contains(&ceiling));
    let host =
        std::env::var("DENON_X3800H_HOST").expect("set DENON_X3800H_HOST before live writes");
    let session = X3800hSession::connect(
        ReceiverId::new(host.clone()).expect("host is a valid receiver identity"),
        &host,
        AvrSessionConfig::default(),
    )
    .await
    .expect("connect to live receiver");
    session
        .synchronize()
        .await
        .expect("read-only synchronization");
    let state = session.current_state();
    let observed_volume = state
        .main_zone
        .volume
        .last_good
        .expect("volume must be observed before writing")
        .value;
    if let MasterVolume::DbHalfSteps(value) = observed_volume {
        assert!(
            value <= ceiling,
            "receiver volume {value} exceeds declared safe ceiling {ceiling}"
        );
    }
    let original = state
        .main_zone
        .mute
        .last_good
        .expect("mute must be observed before writing")
        .value;
    let temporary = match original {
        MuteState::On => MuteState::Off,
        MuteState::Off => MuteState::On,
    };
    let toggled = session
        .operate(OperationRequest {
            id: OperationId(1),
            intent: ReceiverIntent::Mute(temporary),
        })
        .await;
    assert!(matches!(
        toggled,
        denon_avr_domain::OperationOutcome::ObservedRequestedValue { .. }
    ));
    let restored = session
        .operate(OperationRequest {
            id: OperationId(2),
            intent: ReceiverIntent::Mute(original),
        })
        .await;
    assert!(matches!(
        restored,
        denon_avr_domain::OperationOutcome::ObservedRequestedValue { .. }
    ));
    session.close().await.expect("close live write session");
}
