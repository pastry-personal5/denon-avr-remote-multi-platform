//! Delivery-independent Phase 5 smoke coverage.
//!
//! This target imports only reusable domain, application, protocol, and
//! infrastructure packages.  It deliberately does not start a CLI, desktop,
//! or GUI runtime.

use denon_avr_application::{CanonicalReceiverSession, OperationRequest};
use denon_avr_domain::{
    MasterVolume, OperationId, OperationOutcome, ReceiverId, ReceiverIntent, ZonePower,
};
use denon_avr_infrastructure::{AvrSessionConfig, X3800hSession};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

#[tokio::test]
async fn smk_01_connect_sync_and_targeted_control_have_receiver_evidence() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let receiver = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        let mut line = Vec::new();
        let mut volume_changed = false;
        loop {
            line.clear();
            if stream.read_until(b'\r', &mut line).await.unwrap() == 0 {
                break;
            }
            let command = std::str::from_utf8(&line).unwrap();
            let reply = match command {
                "PW?\r" => "PWON\r",
                "ZM?\r" => "ZMON\r",
                "Z2?\r" => "Z2OFF\r",
                "SI?\r" => "SICD\r",
                "MV?\r" if volume_changed => "MV41\r",
                "MV?\r" => "MV80\r",
                "MU?\r" => "MUOFF\r",
                "MS?\r" => "MSSTEREO\r",
                "MV41\r" => {
                    volume_changed = true;
                    continue;
                }
                other => panic!("virtual X3800H rejected {other:?}"),
            };
            stream.get_mut().write_all(reply.as_bytes()).await.unwrap();
        }
    });

    let session = X3800hSession::connect_addr(
        ReceiverId::new("smoke-x3800h").unwrap(),
        address,
        AvrSessionConfig {
            transmission_interval: std::time::Duration::from_millis(1),
            ..AvrSessionConfig::default()
        },
    )
    .await
    .unwrap();
    assert!(session.synchronize().await.unwrap().ready);
    let state = session.state().latest();
    assert_eq!(
        state.main_zone.power.last_good.unwrap().value,
        ZonePower::On
    );

    let outcome = session
        .operate(OperationRequest {
            id: OperationId(7),
            intent: ReceiverIntent::Volume(MasterVolume::db_half_steps(-78).unwrap()),
        })
        .await;
    assert!(matches!(
        outcome,
        OperationOutcome::ObservedRequestedValue { .. }
    ));
    assert_eq!(
        session
            .state()
            .latest()
            .main_zone
            .volume
            .last_good
            .unwrap()
            .value,
        MasterVolume::db_half_steps(-78).unwrap()
    );
    session.close().await.unwrap();
    session.close().await.unwrap();
    receiver.await.unwrap();
}

#[tokio::test]
async fn smk_07_omitted_event_is_repaired_by_targeted_observation() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let receiver = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        let mut line = Vec::new();
        let mut queries = 0_u8;
        loop {
            line.clear();
            if stream.read_until(b'\r', &mut line).await.unwrap() == 0 {
                break;
            }
            let command = std::str::from_utf8(&line).unwrap();
            let reply = match command {
                "PW?\r" => "PWON\r",
                "ZM?\r" => "ZMON\r",
                "Z2?\r" => "Z2OFF\r",
                "SI?\r" => "SICD\r",
                "MV?\r" if queries >= 7 => "MV40\r",
                "MV?\r" => "MV80\r",
                "MU?\r" => "MUOFF\r",
                "MS?\r" => "MSSTEREO\r",
                other => panic!("virtual X3800H rejected {other:?}"),
            };
            if command.ends_with("?\r") {
                queries = queries.saturating_add(1);
            }
            stream.get_mut().write_all(reply.as_bytes()).await.unwrap();
        }
    });
    let session = X3800hSession::connect_addr(
        ReceiverId::new("smoke-omitted-event").unwrap(),
        address,
        AvrSessionConfig {
            transmission_interval: std::time::Duration::from_millis(1),
            ..AvrSessionConfig::default()
        },
    )
    .await
    .unwrap();
    session
        .observe(denon_avr_domain::CoreField::Volume)
        .await
        .unwrap();
    assert_eq!(
        session
            .current_state()
            .main_zone
            .volume
            .last_good
            .unwrap()
            .value,
        MasterVolume::db_half_steps(-80).unwrap()
    );
    session.close().await.unwrap();
    receiver.await.unwrap();
}
