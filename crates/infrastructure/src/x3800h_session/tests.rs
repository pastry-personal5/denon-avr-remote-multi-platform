//! Unit tests for the X3800H session: the actor, the operate path, the
//! reducer, and the session handle over loopback receivers.

use super::actor::fairness_budget_due;
use super::operate::{admit_intent, operate, state_matches};
use super::reduce::{apply_event, reduce_line, Feed};
use super::*;
use crate::avr_session::AvrSessionEvent;
use denon_avr_application::CanonicalReceiverSession;
use denon_avr_domain::{
    CoreFrame, FieldBaseline, FrameSeq, MasterVolume, MonotonicMillis, ObservationOrigin,
    OperationId, Precondition, PreconditionMismatch, ReceiverFieldValidity as FieldValidity,
    RejectionCause, SyncCause, SyncCycleId, SyncDebt,
};
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

#[test]
fn unsupported_source_and_arbitrary_sound_mode_writes_are_rejected() {
    assert!(admit_intent(&ReceiverIntent::Source(
        denon_avr_domain::SourceId::new("NOT-A-SOURCE").unwrap()
    ))
    .is_err());
    assert!(admit_intent(&ReceiverIntent::Source(
        denon_avr_domain::SourceId::new("CD").unwrap()
    ))
    .is_ok());
    assert!(admit_intent(&ReceiverIntent::SoundMode(
        denon_avr_domain::SoundModeIntent::Select("UNVERIFIED".into())
    ))
    .is_err());
    assert!(admit_intent(&ReceiverIntent::SoundMode(
        denon_avr_domain::SoundModeIntent::Select("DOLBY SURROUND".into())
    ))
    .is_ok());
}

#[test]
fn initial_connection_errors_include_receiver_access_guidance() {
    let message = initial_connection_guidance("connection refused");
    assert!(message.contains("connection refused"));
    assert!(message.contains("Network Control"));
    assert!(message.contains("Telnet client"));
}

#[test]
fn fairness_budget_forces_due_reconciliation_by_count_or_time() {
    let receiver = ReceiverId::new("fairness").unwrap();
    let debt = Some(SyncDebt::new(
        receiver,
        Epoch(1),
        SyncCycleId(1),
        CoreField::Volume,
        SyncCause::ReceiverEvent,
        MonotonicMillis(0),
    ));
    let started = Instant::now() - std::time::Duration::from_secs(1);
    let recent = Instant::now();
    assert!(!fairness_budget_due(&debt, started, recent, 7));
    assert!(fairness_budget_due(&debt, started, recent, 8));
    assert!(fairness_budget_due(
        &debt,
        started,
        recent - MAX_USER_WORK_WINDOW,
        0
    ));
}

#[tokio::test]
async fn cancelled_operation_is_rejected_before_transport_or_debt_creation() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = listener.accept().await;
        std::future::pending::<()>().await;
    });
    let mut avr = AvrSession::connect_addr(address, AvrSessionConfig::default())
        .await
        .unwrap();
    let receiver = ReceiverId::new("cancelled").unwrap();
    let mut initial = ReceiverState::new(receiver);
    initial.establish_epoch(Epoch(1));
    let (states, _) = watch::channel(initial);
    let (reply, result) = oneshot::channel();
    drop(result);
    let mut debt = None;
    let mut feed = Feed {
        states,
        epoch: Epoch(1),
        frame_seq: FrameSeq(0),
        started: Instant::now(),
    };
    let outcome = operate(
        &mut avr,
        &mut feed,
        OperationRequest::new(
            OperationId(99),
            ReceiverIntent::Mute(denon_avr_domain::MuteState::On),
        ),
        &reply,
        &mut debt,
        SyncCycleId(1),
    )
    .await;
    assert!(matches!(
        outcome,
        OperationOutcome::Cancelled {
            operation: OperationId(99)
        }
    ));
    assert!(debt.is_none());
    avr.close().await.unwrap();
    server.abort();
}

#[test]
fn detailed_sound_mode_observation_matches_exact_and_category_intents() {
    let id = ReceiverId::new("living-room").unwrap();
    let mut state = ReceiverState::new(id.clone());
    state.establish_epoch(Epoch(1));
    state.reduce(
        &id,
        Epoch(1),
        FrameSeq(1),
        MonotonicMillis(0),
        MonotonicMillis(100),
        ObservationOrigin::ReceiverFrame,
        CoreFrame::SoundMode(denon_avr_domain::SoundModeStatus {
            id: "STEREO".into(),
            raw: "STEREO".into(),
        }),
    );
    assert!(state_matches(
        &state,
        &ReceiverIntent::SoundMode(denon_avr_domain::SoundModeIntent::Stereo)
    ));
    assert!(!state_matches(
        &state,
        &ReceiverIntent::SoundMode(denon_avr_domain::SoundModeIntent::Direct)
    ));
    assert!(state_matches(
        &state,
        &ReceiverIntent::SoundMode(denon_avr_domain::SoundModeIntent::RecallMusic)
    ));
    assert!(!state_matches(
        &state,
        &ReceiverIntent::SoundMode(denon_avr_domain::SoundModeIntent::RecallMovie)
    ));
}

#[test]
fn malformed_and_unknown_frames_are_diagnostic_only() {
    let id = ReceiverId::new("diagnostic").unwrap();
    let mut state = ReceiverState::new(id.clone());
    state.establish_epoch(Epoch(1));
    let (states, _) = watch::channel(state);
    let mut sequence = FrameSeq(0);
    reduce_line(
        &states,
        Epoch(1),
        &mut sequence,
        Instant::now(),
        "ZZUNKNOWN",
        false,
    );
    reduce_line(
        &states,
        Epoch(1),
        &mut sequence,
        Instant::now(),
        "MV985",
        false,
    );
    assert_eq!(states.borrow().diagnostics.len(), 2);
    assert_eq!(sequence, FrameSeq(2));
    assert!(states.borrow().main_zone.volume.last_good.is_none());
}

#[test]
fn reconnect_lifecycle_installs_new_epoch_before_accepting_frames() {
    let id = ReceiverId::new("living-room").unwrap();
    let mut state = ReceiverState::new(id);
    state.establish_epoch(Epoch(1));
    let (states, _) = watch::channel(state);
    let mut feed = Feed {
        states,
        epoch: Epoch(1),
        frame_seq: FrameSeq(0),
        started: Instant::now(),
    };
    apply_event(AvrSessionEvent::Reconnected, &mut feed);
    let Feed { states, epoch, .. } = feed;
    assert_eq!(epoch, Epoch(2));
    assert_eq!(states.borrow().epoch, Some(Epoch(2)));
}

#[tokio::test]
async fn loopback_session_publishes_one_canonical_state_and_confirms_targeted_control() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        let mut line = Vec::new();
        let mut volume = "MV80";
        loop {
            line.clear();
            if stream.read_until(b'\r', &mut line).await.unwrap() == 0 {
                break;
            }
            let command = std::str::from_utf8(&line).unwrap();
            let response = match command {
                "PW?\r" => "PWON\r",
                "ZM?\r" => "ZMON\r",
                "Z2?\r" => "Z2OFF\r",
                "SI?\r" => "SICD\r",
                "MV?\r" => {
                    if volume == "MV80" {
                        "MV80\r"
                    } else {
                        "MV41\r"
                    }
                }
                "MU?\r" => "MUOFF\r",
                "MS?\r" => "MSSTEREO\r",
                "MV41\r" => {
                    volume = "MV41";
                    continue;
                }
                unexpected => panic!("unexpected command {unexpected:?}"),
            };
            stream
                .get_mut()
                .write_all(response.as_bytes())
                .await
                .unwrap();
        }
    });
    let receiver = ReceiverId::new("loopback-x3800h").unwrap();
    let session = X3800hSession::connect_addr(
        receiver,
        address,
        AvrSessionConfig {
            transmission_interval: std::time::Duration::from_millis(1),
            ..AvrSessionConfig::default()
        },
    )
    .await
    .unwrap();
    let readiness = session.synchronize().await.unwrap();
    assert!(readiness.ready, "{readiness:?}");
    session.observe(CoreField::Volume).await.unwrap();
    assert!(matches!(
        session.current_state().main_zone.volume.synchronization,
        denon_avr_domain::FieldSynchronization::Settled { .. }
    ));
    let subscription = session.state();
    assert_eq!(
        subscription
            .latest()
            .main_zone
            .power
            .last_good
            .unwrap()
            .value,
        denon_avr_domain::ZonePower::On
    );
    let outcome = session
        .operate(OperationRequest::new(
            OperationId(1),
            ReceiverIntent::Volume(MasterVolume::db_half_steps(-78).unwrap()),
        ))
        .await;
    assert!(
        matches!(
            outcome,
            OperationOutcome::ObservedRequestedValue {
                operation: OperationId(1),
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(
        subscription
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
    server.await.unwrap();
}

#[tokio::test]
async fn source_catalog_is_read_over_http_and_stamped_with_the_connection_generation() {
    let telnet = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = telnet.local_addr().unwrap();
    let telnet_server = tokio::spawn(async move {
        let (stream, _) = telnet.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        let mut line = Vec::new();
        loop {
            line.clear();
            if stream.read_until(b'\r', &mut line).await.unwrap() == 0 {
                break;
            }
            let response = match std::str::from_utf8(&line).unwrap() {
                "PW?\r" => "PWON\r",
                "ZM?\r" => "ZMON\r",
                "Z2?\r" => "Z2OFF\r",
                "SI?\r" => "SICD\r",
                "MV?\r" => "MV80\r",
                "MU?\r" => "MUOFF\r",
                "MS?\r" => "MSSTEREO\r",
                unexpected => panic!("unexpected command {unexpected:?}"),
            };
            stream
                .get_mut()
                .write_all(response.as_bytes())
                .await
                .unwrap();
        }
    });
    let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_port = http.local_addr().unwrap().port();
    let http_server = tokio::spawn(async move {
        let (mut stream, _) = http.accept().await.unwrap();
        let mut request = Vec::new();
        let mut chunk = [0_u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = stream.read(&mut chunk).await.unwrap();
            assert_ne!(read, 0, "the client closed before sending a request");
            request.extend_from_slice(&chunk[..read]);
        }
        let request = String::from_utf8_lossy(&request).into_owned();
        assert!(
            request.starts_with("POST /goform/AppCommand.xml HTTP/1.1\r\n"),
            "{request}"
        );
        let body = "<rx><functionrename><list><name>GAME</name><rename>Console</rename></list></functionrename><functiondelete><list><FuncName>GAME</FuncName><use>1</use></list></functiondelete></rx>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    let session = X3800hSession::connect_addr(
        ReceiverId::new("catalog").unwrap(),
        address,
        AvrSessionConfig {
            transmission_interval: std::time::Duration::from_millis(1),
            app_command_port: http_port,
            ..AvrSessionConfig::default()
        },
    )
    .await
    .unwrap();
    session.synchronize().await.unwrap();

    let observation = session.source_catalog().await.unwrap();
    let game = observation
        .catalog
        .entries
        .iter()
        .find(|entry| entry.id.as_str() == "GAME")
        .expect("GAME is in the catalog");
    assert_eq!(game.display_name.as_deref(), Some("Console"));
    // The first connection is epoch one, which is generation zero.
    assert_eq!(observation.catalog.generation, 0);

    session.close().await.unwrap();
    http_server.await.unwrap();
    telnet_server.await.unwrap();
}

#[tokio::test]
async fn an_unreachable_http_endpoint_is_a_connection_error_not_a_hang() {
    let telnet = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = telnet.local_addr().unwrap();
    let telnet_server = tokio::spawn(async move {
        let _ = telnet.accept().await;
        std::future::pending::<()>().await;
    });
    // Bind and release a port so nothing is listening on it.
    let closed_port = {
        let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        probe.local_addr().unwrap().port()
    };
    let session = X3800hSession::connect_addr(
        ReceiverId::new("unreachable-http").unwrap(),
        address,
        AvrSessionConfig {
            app_command_port: closed_port,
            connect_timeout: std::time::Duration::from_millis(500),
            ..AvrSessionConfig::default()
        },
    )
    .await
    .unwrap();
    let error = session.source_catalog().await.unwrap_err();
    assert_eq!(
        error.kind,
        denon_avr_application::ports::OperationErrorKind::Connection
    );
    assert_eq!(error.context, "source catalog");
    session.close().await.unwrap();
    telnet_server.abort();
}

/// A receiver whose volume and mute a test can change behind the session's
/// back, and which records every write it is sent.
#[derive(Default)]
struct FakeReceiver {
    volume: String,
    mute: String,
    writes: Vec<String>,
    /// Stop answering mute queries, so a re-observation fails.
    silent_mute: bool,
}

type SharedReceiver = Arc<std::sync::Mutex<FakeReceiver>>;

fn fake_receiver() -> SharedReceiver {
    Arc::new(std::sync::Mutex::new(FakeReceiver {
        volume: "MV80".into(),
        mute: "MUOFF".into(),
        ..FakeReceiver::default()
    }))
}

/// Serve the Telnet protocol on loopback until aborted. `drop_connection`
/// closes the current connection, which the session sees as a reconnect.
async fn serve(
    shared: SharedReceiver,
    drop_connection: Arc<tokio::sync::Notify>,
) -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let mut stream = BufReader::new(stream);
            let mut line = Vec::new();
            loop {
                line.clear();
                let read = tokio::select! {
                    read = stream.read_until(b'\r', &mut line) => read.unwrap(),
                    _ = drop_connection.notified() => break,
                };
                if read == 0 {
                    break;
                }
                let command = std::str::from_utf8(&line).unwrap().to_owned();
                let response = {
                    let mut receiver = shared.lock().unwrap();
                    match command.as_str() {
                        "PW?\r" => Some("PWON".to_owned()),
                        "ZM?\r" => Some("ZMON".to_owned()),
                        "Z2?\r" => Some("Z2OFF".to_owned()),
                        "SI?\r" => Some("SICD".to_owned()),
                        "MS?\r" => Some("MSSTEREO".to_owned()),
                        "MV?\r" => Some(receiver.volume.clone()),
                        "MU?\r" if receiver.silent_mute => None,
                        "MU?\r" => Some(receiver.mute.clone()),
                        write if write.starts_with("MV") || write.starts_with("MU") => {
                            let write = write.trim_end().to_owned();
                            if write.starts_with("MV") {
                                receiver.volume = write.clone();
                            } else {
                                receiver.mute = write.clone();
                            }
                            receiver.writes.push(write);
                            None
                        }
                        unexpected => panic!("unexpected command {unexpected:?}"),
                    }
                };
                if let Some(response) = response {
                    stream
                        .get_mut()
                        .write_all(format!("{response}\r").as_bytes())
                        .await
                        .unwrap();
                }
            }
        }
    });
    (address, task)
}

async fn connect_to(address: SocketAddr, name: &str) -> Arc<X3800hSession> {
    let session = X3800hSession::connect_addr(
        ReceiverId::new(name).unwrap(),
        address,
        AvrSessionConfig {
            transmission_interval: std::time::Duration::from_millis(1),
            response_timeout: std::time::Duration::from_millis(200),
            reconnect_delay: std::time::Duration::from_millis(10),
            ..AvrSessionConfig::default()
        },
    )
    .await
    .unwrap();
    assert!(session.synchronize().await.unwrap().ready);
    session
}

fn volume_request(half_steps: i16) -> OperationRequest {
    OperationRequest::new(
        OperationId(1),
        ReceiverIntent::Volume(MasterVolume::db_half_steps(half_steps).unwrap()),
    )
}

fn baseline(session: &X3800hSession, fields: &[CoreField]) -> Precondition {
    Precondition::capture(&session.current_state(), fields.iter().copied()).unwrap()
}

fn writes(shared: &SharedReceiver) -> Vec<String> {
    shared.lock().unwrap().writes.clone()
}

fn rejected_with(outcome: &OperationOutcome, expected: RejectionCause) {
    match outcome {
        OperationOutcome::RejectedBeforeDispatch { cause, .. } => {
            assert_eq!(*cause, expected, "{outcome:?}")
        }
        other => panic!("expected a rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn a_matching_precondition_lets_the_write_through_exactly_once() {
    let shared = fake_receiver();
    let (address, server) = serve(Arc::clone(&shared), Default::default()).await;
    let session = connect_to(address, "precondition-match").await;
    let precondition = baseline(&session, &[CoreField::Volume, CoreField::Mute]);

    let outcome = session
        .operate(volume_request(-78).with_precondition(precondition))
        .await;

    assert!(
        matches!(outcome, OperationOutcome::ObservedRequestedValue { .. }),
        "{outcome:?}"
    );
    assert_eq!(writes(&shared), vec!["MV41"]);
    session.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn a_changed_target_refuses_the_write_and_reports_the_field() {
    let shared = fake_receiver();
    let (address, server) = serve(Arc::clone(&shared), Default::default()).await;
    let session = connect_to(address, "precondition-target").await;
    let precondition = baseline(&session, &[CoreField::Volume, CoreField::Mute]);
    // Someone turns the volume down after the decision was made.
    shared.lock().unwrap().volume = "MV60".into();

    let outcome = session
        .operate(volume_request(-78).with_precondition(precondition))
        .await;

    rejected_with(
        &outcome,
        RejectionCause::PreconditionMismatch(PreconditionMismatch::Field(CoreField::Volume)),
    );
    assert!(writes(&shared).is_empty(), "nothing may be written");
    // The re-observation is real: the session now shows the new volume.
    assert_eq!(
        session
            .current_state()
            .main_zone
            .volume
            .last_good
            .unwrap()
            .value,
        MasterVolume::db_half_steps(-40).unwrap()
    );
    session.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn a_changed_non_target_field_refuses_the_write() {
    let shared = fake_receiver();
    let (address, server) = serve(Arc::clone(&shared), Default::default()).await;
    let session = connect_to(address, "precondition-other").await;
    // The decision to raise the volume also depended on the mute state.
    let precondition = baseline(&session, &[CoreField::Volume, CoreField::Mute]);
    shared.lock().unwrap().mute = "MUON".into();

    let outcome = session
        .operate(volume_request(-78).with_precondition(precondition))
        .await;

    rejected_with(
        &outcome,
        RejectionCause::PreconditionMismatch(PreconditionMismatch::Field(CoreField::Mute)),
    );
    assert!(writes(&shared).is_empty(), "nothing may be written");
    // The non-target field was re-observed, not assumed.
    assert_eq!(
        session
            .current_state()
            .main_zone
            .mute
            .last_good
            .unwrap()
            .value,
        denon_avr_domain::MuteState::On
    );
    session.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn a_change_in_an_unnamed_field_does_not_void_the_decision() {
    let shared = fake_receiver();
    let (address, server) = serve(Arc::clone(&shared), Default::default()).await;
    let session = connect_to(address, "precondition-unnamed").await;
    let precondition = baseline(&session, &[CoreField::Volume]);
    shared.lock().unwrap().mute = "MUON".into();

    let outcome = session
        .operate(volume_request(-78).with_precondition(precondition))
        .await;

    assert!(
        matches!(outcome, OperationOutcome::ObservedRequestedValue { .. }),
        "{outcome:?}"
    );
    assert_eq!(writes(&shared), vec!["MV41"]);
    session.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn a_mismatch_takes_precedence_over_already_in_state() {
    let shared = fake_receiver();
    let (address, server) = serve(Arc::clone(&shared), Default::default()).await;
    let session = connect_to(address, "precondition-precedence").await;
    let precondition = baseline(&session, &[CoreField::Volume]);
    // The receiver already holds the requested volume, but not the baseline.
    shared.lock().unwrap().volume = "MV41".into();

    let outcome = session
        .operate(volume_request(-78).with_precondition(precondition))
        .await;
    rejected_with(
        &outcome,
        RejectionCause::PreconditionMismatch(PreconditionMismatch::Field(CoreField::Volume)),
    );

    // Without a precondition, the same request is simply already in state.
    let outcome = session.operate(volume_request(-78)).await;
    assert!(
        matches!(outcome, OperationOutcome::AlreadyObserved { .. }),
        "{outcome:?}"
    );
    assert!(writes(&shared).is_empty());
    session.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn a_baseline_of_no_usable_value_is_void_once_the_field_is_known() {
    let shared = fake_receiver();
    let (address, server) = serve(Arc::clone(&shared), Default::default()).await;
    let session = connect_to(address, "precondition-unknown").await;
    // The decision saw no usable mute state; the receiver reports one now.
    let precondition = Precondition::new(Epoch(1))
        .with(FieldBaseline::capture(
            &session.current_state(),
            CoreField::Volume,
        ))
        .with(FieldBaseline::NoUsableValue(CoreField::Mute));

    let outcome = session
        .operate(volume_request(-78).with_precondition(precondition))
        .await;

    rejected_with(
        &outcome,
        RejectionCause::PreconditionMismatch(PreconditionMismatch::Field(CoreField::Mute)),
    );
    assert!(writes(&shared).is_empty());
    session.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn a_precondition_from_another_epoch_refuses_the_write() {
    let shared = fake_receiver();
    let (address, server) = serve(Arc::clone(&shared), Default::default()).await;
    let session = connect_to(address, "precondition-epoch").await;
    let precondition = Precondition::new(Epoch(9)).with(FieldBaseline::capture(
        &session.current_state(),
        CoreField::Volume,
    ));

    let outcome = session
        .operate(volume_request(-78).with_precondition(precondition))
        .await;

    rejected_with(
        &outcome,
        RejectionCause::PreconditionMismatch(PreconditionMismatch::Epoch),
    );
    assert!(writes(&shared).is_empty());
    session.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn a_reconnect_voids_a_precondition_captured_before_it() {
    let shared = fake_receiver();
    let drop_connection = Arc::new(tokio::sync::Notify::new());
    let (address, server) = serve(Arc::clone(&shared), Arc::clone(&drop_connection)).await;
    let session = connect_to(address, "precondition-reconnect").await;
    let before = baseline(&session, &[CoreField::Volume]);
    assert_eq!(before.epoch(), Epoch(1));

    // The receiver drops the connection and the session reconnects.
    drop_connection.notify_one();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while session.current_state().epoch != Some(Epoch(2)) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the session did not reconnect"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(session.synchronize().await.unwrap().ready);

    let outcome = session
        .operate(volume_request(-78).with_precondition(before))
        .await;
    rejected_with(
        &outcome,
        RejectionCause::PreconditionMismatch(PreconditionMismatch::Epoch),
    );
    assert!(
        writes(&shared).is_empty(),
        "authority from the old connection is void"
    );

    // A decision made on the new connection goes through.
    let after = baseline(&session, &[CoreField::Volume]);
    assert_eq!(after.epoch(), Epoch(2));
    let outcome = session
        .operate(volume_request(-78).with_precondition(after))
        .await;
    assert!(
        matches!(outcome, OperationOutcome::ObservedRequestedValue { .. }),
        "{outcome:?}"
    );
    assert_eq!(writes(&shared), vec!["MV41"]);
    session.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn a_reconnect_reads_every_core_field_again_without_being_asked() {
    let shared = fake_receiver();
    let drop_connection = Arc::new(tokio::sync::Notify::new());
    let (address, server) = serve(Arc::clone(&shared), Arc::clone(&drop_connection)).await;
    let session = connect_to(address, "reconnect-reads").await;

    drop_connection.notify_one();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while session.current_state().epoch != Some(Epoch(2)) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the session did not reconnect"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }

    // Nobody calls `synchronize`. The next periodic sweep is five seconds
    // away, so every field being current again within two is the session
    // reading on its own.
    let current = |state: &ReceiverState| {
        [
            matches!(state.system_power.validity, FieldValidity::Current { .. }),
            matches!(
                state.main_zone.power.validity,
                FieldValidity::Current { .. }
            ),
            matches!(state.zone2_power.validity, FieldValidity::Current { .. }),
            matches!(
                state.main_zone.source.validity,
                FieldValidity::Current { .. }
            ),
            matches!(
                state.main_zone.volume.validity,
                FieldValidity::Current { .. }
            ),
            matches!(state.main_zone.mute.validity, FieldValidity::Current { .. }),
            matches!(
                state.main_zone.sound_mode.validity,
                FieldValidity::Current { .. }
            ),
        ]
        .into_iter()
        .all(|current| current)
    };
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while !current(&session.current_state()) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the fields were still stale two seconds after the reconnect: {:?}",
            session.current_state()
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert_eq!(session.current_state().epoch, Some(Epoch(2)));
    assert!(writes(&shared).is_empty(), "a re-read dispatches nothing");
    session.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn a_field_that_cannot_be_re_observed_refuses_the_write() {
    let shared = fake_receiver();
    let (address, server) = serve(Arc::clone(&shared), Default::default()).await;
    let session = connect_to(address, "precondition-unobservable").await;
    let precondition = baseline(&session, &[CoreField::Volume, CoreField::Mute]);
    shared.lock().unwrap().silent_mute = true;

    let outcome = session
        .operate(volume_request(-78).with_precondition(precondition))
        .await;

    rejected_with(&outcome, RejectionCause::ObservationFailed);
    assert!(writes(&shared).is_empty());
    session.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn unsupported_intents_are_rejected_with_a_typed_cause() {
    let shared = fake_receiver();
    let (address, server) = serve(Arc::clone(&shared), Default::default()).await;
    let session = connect_to(address, "typed-cause").await;
    let outcome = session
        .operate(OperationRequest::new(
            OperationId(2),
            ReceiverIntent::Source(denon_avr_domain::SourceId::new("NOT-A-SOURCE").unwrap()),
        ))
        .await;
    rejected_with(&outcome, RejectionCause::UnsupportedIntent);
    assert!(writes(&shared).is_empty());
    session.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn dropped_operation_requester_does_not_replay_or_rollback_receiver_state() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (dispatched_tx, dispatched_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        let mut line = Vec::new();
        let mut mutating_writes = 0_u8;
        let mut dispatched_tx = Some(dispatched_tx);
        loop {
            line.clear();
            if stream.read_until(b'\r', &mut line).await.unwrap() == 0 {
                break;
            }
            let command = std::str::from_utf8(&line).unwrap();
            if command == "MV205\r" {
                mutating_writes += 1;
                if let Some(sender) = dispatched_tx.take() {
                    let _ = sender.send(());
                }
                continue;
            }
            let response = match command {
                "PW?\r" => "PWON\r",
                "ZM?\r" => "ZMON\r",
                "Z2?\r" => "Z2OFF\r",
                "SI?\r" => "SICD\r",
                "MV?\r" if mutating_writes == 0 => "MV80\r",
                "MV?\r" => "MV205\r",
                "MU?\r" => "MUOFF\r",
                "MS?\r" => "MSSTEREO\r",
                unexpected => panic!("unexpected command {unexpected:?}"),
            };
            stream
                .get_mut()
                .write_all(response.as_bytes())
                .await
                .unwrap();
        }
        assert_eq!(mutating_writes, 1);
    });
    let receiver = ReceiverId::new("dropped-requester").unwrap();
    let session = X3800hSession::connect_addr(
        receiver,
        address,
        AvrSessionConfig {
            transmission_interval: std::time::Duration::from_millis(1),
            ..AvrSessionConfig::default()
        },
    )
    .await
    .unwrap();
    let operation = tokio::spawn({
        let session = Arc::clone(&session);
        async move {
            session
                .operate(OperationRequest::new(
                    OperationId(44),
                    ReceiverIntent::Volume(MasterVolume::db_half_steps(-119).unwrap()),
                ))
                .await
        }
    });
    dispatched_rx.await.unwrap();
    operation.abort();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(
        session
            .current_state()
            .main_zone
            .volume
            .last_good
            .as_ref()
            .unwrap()
            .value,
        MasterVolume::db_half_steps(-119).unwrap()
    );
    session.close().await.unwrap();
    server.await.unwrap();
}
