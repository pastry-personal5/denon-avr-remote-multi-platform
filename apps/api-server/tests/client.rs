//! The port behaves the same whether it is the service in this process or the
//! service across the socket. One function, written against the port's traits,
//! is run against `ControlService` directly and against `ApiClient` through the
//! server, and what it saw is compared line by line. Milestone 6's conformance
//! suite for the MCP adapter follows the same shape.

mod support;

use denon_avr_api_client::{ApiClient, Audience, Endpoint, Token};
use denon_avr_application::{
    AgentControl, AgentLabel, ControlError, DryRun, IdempotencyKey, OperationStatus,
    OperationSubmission, OperatorAdmin, OperatorControl, Principal, ReceiverReads, ReceiverSummary,
    StateSubscription,
};
use denon_avr_domain::{
    CoreField, FieldIssue, FieldSynchronization, FrameSeq, MasterVolume, MonotonicMillis,
    MuteState, OperationId, ReceiverId, ReceiverIntent, StaleReason, SyncCycleId,
};
use std::sync::Arc;
use std::time::Duration;
use support::*;

/// What the conformance run is handed: the port as each caller sees it.
trait Harness {
    fn operator(&self) -> Arc<dyn OperatorControl>;
    fn agent(&self) -> Arc<dyn AgentControl>;
    /// An agent's handle to the Operator's methods, which must refuse it.
    fn agent_admin(&self) -> Arc<dyn OperatorAdmin>;
}

struct InProcess {
    operator: Arc<denon_avr_application::ServiceHandle>,
    agent: Arc<denon_avr_application::ServiceHandle>,
}

impl InProcess {
    async fn new(fixture: &Fixture) -> Self {
        // The agent in the other run has a token, so this one has the same.
        fixture.agent_token("openclaw").await;
        Self {
            operator: fixture.service.handle(Principal::Operator).unwrap(),
            agent: fixture
                .service
                .handle(Principal::Agent(AgentLabel::new("openclaw").unwrap()))
                .unwrap(),
        }
    }
}

impl Harness for InProcess {
    fn operator(&self) -> Arc<dyn OperatorControl> {
        self.operator.clone()
    }
    fn agent(&self) -> Arc<dyn AgentControl> {
        self.agent.clone()
    }
    fn agent_admin(&self) -> Arc<dyn OperatorAdmin> {
        self.agent.clone()
    }
}

struct OverSocket {
    operator: Arc<ApiClient>,
    agent: Arc<ApiClient>,
}

impl OverSocket {
    async fn new(fixture: &Fixture) -> Self {
        let client = |socket: &std::path::Path, token: String, audience| {
            Arc::new(
                ApiClient::connect(Endpoint {
                    socket: socket.to_owned(),
                    token: Token::new(token),
                    audience,
                })
                .unwrap(),
            )
        };
        Self {
            operator: client(
                fixture.operator_socket(),
                fixture.operator_token(),
                Audience::Operator,
            ),
            agent: client(
                fixture.agent_socket(),
                fixture.agent_token("openclaw").await,
                Audience::Agent,
            ),
        }
    }
}

impl Harness for OverSocket {
    fn operator(&self) -> Arc<dyn OperatorControl> {
        self.operator.clone()
    }
    fn agent(&self) -> Arc<dyn AgentControl> {
        self.agent.clone()
    }
    fn agent_admin(&self) -> Arc<dyn OperatorAdmin> {
        self.agent.clone()
    }
}

fn summary(summary: &ReceiverSummary) -> String {
    // The connection is left out: whether the receiver is held depends on timing.
    format!(
        "{} {:?} {:?}",
        summary.id.as_str(),
        summary.model,
        summary.capabilities
    )
}

/// A rate limit's wait is whole seconds that depend on when it was asked.
fn error_line(error: &ControlError) -> String {
    match error {
        ControlError::RateLimited { retry_after } => format!(
            "RateLimited, retry in 1..=60 s: {}",
            retry_after.is_some_and(|wait| (1..=60).contains(&wait.as_secs()))
        ),
        other => format!("{other:?}"),
    }
}

fn dry_run_line(result: &Result<DryRun, ControlError>) -> String {
    match result {
        Ok(dry_run) => format!("{:?}", dry_run.decision),
        Err(error) => error_line(error),
    }
}

async fn until_in_session(control: &dyn AgentControl, id: OperationId) {
    for _ in 0..200 {
        if control.operation(id, None).await.unwrap().status == OperationStatus::InSession {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("operation {} never reached the session", id.0);
}

/// Everything the port does that a caller can see, as lines of text. Returns the
/// lines and a subscription that is still open, for the caller to end.
async fn conformance(fixture: &Fixture, harness: &dyn Harness) -> (Vec<String>, StateSubscription) {
    let (operator, agent, agent_admin) =
        (harness.operator(), harness.agent(), harness.agent_admin());
    let room = living_room();
    let mut log = Vec::new();
    let mut note = |label: &str, line: String| log.push(format!("{label}: {line}"));

    // Receivers: saved ones only, and no address in either list.
    for (who, port) in [
        ("operator", &*operator as &dyn ReceiverReads),
        ("agent", &*agent as &dyn ReceiverReads),
    ] {
        let list = port.receivers().await.unwrap();
        note(
            &format!("receivers/{who}"),
            format!("{:?}", list.iter().map(summary).collect::<Vec<_>>()),
        );
    }
    note("health/operator", format!("{:?}", operator.health().await));
    note("health/agent", format!("{:?}", agent.health().await));

    // State: the first snapshot, then the next after a change. The Operator is
    // also given the receiver's own text, which the agent never is.
    let mut seen_by_operator = operator.state(&room).await.unwrap();
    let mut seen_by_agent = agent.state(&room).await.unwrap();
    for (who, subscription, with_text) in [
        ("operator", &seen_by_operator, true),
        ("agent", &seen_by_agent, false),
    ] {
        note(
            &format!("state/{who}/first"),
            format!("{:?}", normalize(&subscription.latest(), with_text)),
        );
    }
    fixture.connector.latest().states.send_modify(|state| {
        state.main_zone.volume.last_issue = Some(FieldIssue {
            message: "the query failed for 192.0.2.10".into(),
        });
        state.diagnostics.push("unsupported frame ZZ".into());
        state.revision.0 += 1;
    });
    for (who, subscription, with_text) in [
        ("operator", &mut seen_by_operator, true),
        ("agent", &mut seen_by_agent, false),
    ] {
        let state = subscription.changed().await.unwrap();
        note(
            &format!("state/{who}/after text"),
            format!("{:?}", normalize(&state, with_text)),
        );
    }
    fixture.connector.latest().turn_volume(-30.0);
    for (who, subscription, with_text) in [
        ("operator", &mut seen_by_operator, true),
        ("agent", &mut seen_by_agent, false),
    ] {
        let state = subscription.changed().await.unwrap();
        note(
            &format!("state/{who}/after volume"),
            format!("{:?}", normalize(&state, with_text)),
        );
    }
    drop(seen_by_agent);

    // Sources: the entries both are told; the receiver's own text, the Operator.
    let catalog = operator.source_catalog(&room).await.unwrap();
    note(
        "sources/operator",
        format!(
            "{:?} {} {:?} {:?}",
            catalog.catalog.entries,
            catalog.catalog.generation,
            catalog.catalog.error,
            catalog.raw_response
        ),
    );
    let catalog = agent.source_catalog(&room).await.unwrap();
    note(
        "sources/agent",
        format!(
            "{:?} {}",
            catalog.catalog.entries, catalog.catalog.generation
        ),
    );

    // Dry runs, which write nothing: allowed, held for approval, and as another.
    let mute_on = || ReceiverIntent::Mute(MuteState::On);
    let loud = || ReceiverIntent::Volume(MasterVolume::db_half_steps(-50).unwrap());
    note(
        "dry run/operator",
        dry_run_line(&operator.dry_run(&room, mute_on()).await),
    );
    note(
        "dry run/agent",
        dry_run_line(&agent.dry_run(&room, mute_on()).await),
    );
    note(
        "dry run/agent loud",
        dry_run_line(&agent.dry_run(&room, loud()).await),
    );
    note(
        "dry run/operator as openclaw",
        dry_run_line(
            &operator
                .dry_run_as(AgentLabel::new("openclaw").unwrap(), &room, loud())
                .await,
        ),
    );

    // A submission is completed, and a retry with its key is the same operation.
    let key = IdempotencyKey::new("conformance-1").unwrap();
    let submit =
        |key: IdempotencyKey| OperationSubmission::new(mute_on()).with_idempotency_key(key);
    let first = agent.submit(&room, submit(key.clone())).await.unwrap();
    let done = agent
        .operation(first.id, Some(Duration::from_secs(5)))
        .await
        .unwrap();
    note(
        "submit/agent",
        format!(
            "id {} {} {:?} confirmed {}",
            done.id.0,
            done.status.as_str(),
            done.dispatch,
            done.confirmed
        ),
    );
    let retry = agent.submit(&room, submit(key)).await.unwrap();
    note("retry/agent", format!("same id: {}", retry.id == first.id));
    assert_eq!(done.status, OperationStatus::Completed);

    // Cancelling an operation already in the session is too late, and says what
    // the operation is.
    fixture.connector.hold_operations();
    let held = agent
        .submit(
            &room,
            OperationSubmission::new(ReceiverIntent::Mute(MuteState::Off)),
        )
        .await
        .unwrap();
    until_in_session(&*agent, held.id).await;
    let too_late = agent.cancel(held.id).await;
    note("cancel/agent", error_line(&too_late.clone().unwrap_err()));
    assert!(matches!(too_late, Err(ControlError::TooLate(_))));
    fixture.connector.release_operations();
    let ended = agent
        .operation(held.id, Some(Duration::from_secs(5)))
        .await
        .unwrap();
    note("cancel/agent then", ended.status.as_str().to_owned());

    // What does not exist, in both of the words the port has for it.
    note(
        "not found/operation",
        error_line(
            &operator
                .operation(OperationId(9_999), None)
                .await
                .unwrap_err(),
        ),
    );
    let nowhere = ReceiverId::new("nowhere").unwrap();
    note(
        "not found/receiver",
        error_line(&operator.state(&nowhere).await.map(|_| ()).unwrap_err()),
    );
    note(
        "not found/agent receiver",
        error_line(&agent.state(&nowhere).await.map(|_| ()).unwrap_err()),
    );

    // What an agent may not call.
    note(
        "forbidden/tokens",
        error_line(&agent_admin.tokens().await.map(|_| ()).unwrap_err()),
    );
    note(
        "forbidden/policy",
        error_line(&agent_admin.policy().await.map(|_| ()).unwrap_err()),
    );
    note(
        "forbidden/refresh",
        error_line(&agent_admin.refresh(&room).await.map(|_| ()).unwrap_err()),
    );

    // The Operator's administration.
    note(
        "configuration",
        format!("{:?}", operator.configuration().await.unwrap()),
    );
    note(
        "discover",
        format!("{:?}", operator.discover(Duration::from_millis(50)).await),
    );
    note(
        "tokens",
        format!(
            "{:?}",
            operator
                .tokens()
                .await
                .unwrap()
                .iter()
                .map(|token| (token.label.as_str().to_owned(), token.is_active()))
                .collect::<Vec<_>>()
        ),
    );
    let policy = operator.policy().await.unwrap();
    note(
        "policy",
        format!("{:?} {}", policy.digest, policy.text.is_some()),
    );

    // The write cap, last because it spends the agent's budget. An identical write
    // still running is one operation, so each is waited for before the next.
    let mut limited = None;
    for step in 0..60 {
        let value = if step % 2 == 0 {
            MuteState::On
        } else {
            MuteState::Off
        };
        match agent
            .submit(&room, OperationSubmission::new(ReceiverIntent::Mute(value)))
            .await
        {
            Ok(operation) => {
                agent
                    .operation(operation.id, Some(Duration::from_secs(5)))
                    .await
                    .unwrap();
            }
            Err(error) => {
                limited = Some(error);
                break;
            }
        }
    }
    let limited = limited.expect("the write cap is reached");
    note("rate limited/agent", error_line(&limited));
    assert!(matches!(limited, ControlError::RateLimited { .. }));

    let open = operator.state(&room).await.unwrap();
    (log, open)
}

async fn run(over_socket: bool) -> Vec<String> {
    let fixture = Fixture::start(Options::default()).await;
    let (mut log, mut open) = if over_socket {
        let harness = OverSocket::new(&fixture).await;
        conformance(&fixture, &harness).await
    } else {
        let harness = InProcess::new(&fixture).await;
        conformance(&fixture, &harness).await
    };
    // Shutting the server down closes the sessions, which ends a subscription with
    // the same error whichever way it was made.
    fixture.shutdown().await;
    // A subscription that never ends would hang here, so it is given a limit.
    let after = match tokio::time::timeout(Duration::from_secs(5), open.changed()).await {
        Ok(result) => format!("{:?}", result.map(|_| ())),
        Err(_) => "it did not end".to_owned(),
    };
    log.push(format!("after shutdown: {after}"));
    log
}

#[tokio::test]
async fn the_port_behaves_the_same_in_process_and_over_the_socket() {
    let in_process = run(false).await;
    let over_socket = run(true).await;
    for (local, remote) in in_process.iter().zip(&over_socket) {
        assert_eq!(local, remote, "the two runs differ");
    }
    assert_eq!(in_process.len(), over_socket.len());
    // The comparison is not between two empty runs.
    assert!(in_process.len() > 30, "{in_process:#?}");
    for needle in [
        "TooLate",
        "NotFound(\"operation\")",
        "NotFound(\"receiver\")",
        "Forbidden",
        "RateLimited",
        "session closed",
    ] {
        assert!(
            in_process.iter().any(|line| line.contains(needle)),
            "no line mentions {needle}: {in_process:#?}"
        );
    }
}

#[test]
fn the_normalization_of_a_state_ignores_the_stamps_and_nothing_else() {
    let state = receiver_state(-35.0);
    let base = normalize(&state, true);
    let agent_base = normalize(&state, false);

    // A stamp: where and when an observation was made, which a state read over the
    // socket gets placeholders for.
    let stamped = |change: &dyn Fn(&mut denon_avr_domain::ReceiverState)| {
        let mut changed = state.clone();
        change(&mut changed);
        changed
    };
    for (what, state) in [
        (
            "a frame number",
            stamped(&|s| s.main_zone.volume.last_good.as_mut().unwrap().frame_seq = FrameSeq(99)),
        ),
        (
            "an observation time",
            stamped(&|s| {
                s.main_zone.volume.last_good.as_mut().unwrap().observed_at = MonotonicMillis(5)
            }),
        ),
        (
            "an observation's epoch",
            stamped(&|s| {
                s.main_zone.volume.last_good.as_mut().unwrap().epoch = denon_avr_domain::Epoch(7)
            }),
        ),
        (
            "when a value stops being current",
            stamped(&|s| {
                s.main_zone.volume.validity = denon_avr_domain::ReceiverFieldValidity::Current {
                    valid_until: MonotonicMillis(1),
                }
            }),
        ),
        (
            "synchronization",
            stamped(&|s| {
                s.main_zone.volume.synchronization = FieldSynchronization::Settled {
                    cycle: SyncCycleId(3),
                    observed_at: MonotonicMillis(9),
                }
            }),
        ),
    ] {
        assert_eq!(normalize(&state, true), base, "{what} is a stamp");
    }

    // Everything else is compared: each of these changes the normalization.
    let stale = |reason: StaleReason| {
        stamped(&|s| {
            s.main_zone.volume.validity = denon_avr_domain::ReceiverFieldValidity::Stale {
                reason: reason.clone(),
            }
        })
    };
    let different = [
        (
            "a value",
            stamped(&|s| s.main_zone.volume.last_good.as_mut().unwrap().value = master(-30.0)),
        ),
        ("validity going stale", stale(StaleReason::Disconnected)),
        (
            "the stale reason",
            // Compared with the state that is stale for another reason, below.
            stale(StaleReason::Expired),
        ),
        (
            "validity going unavailable",
            stamped(&|s| {
                s.main_zone.volume.validity = denon_avr_domain::ReceiverFieldValidity::Unavailable {
                    evidence: denon_avr_domain::receiver_state::ReceiverEvidence::UnavailableStatus(
                        "x".into(),
                    ),
                }
            }),
        ),
        (
            "the epoch",
            stamped(&|s| s.epoch = Some(denon_avr_domain::Epoch(2))),
        ),
        ("the revision", stamped(&|s| s.revision.0 += 1)),
    ];
    for (what, state) in &different {
        assert_ne!(normalize(state, true), base, "{what} must show");
        assert_ne!(normalize(state, false), agent_base, "{what} must show");
    }
    assert_ne!(
        normalize(&stale(StaleReason::Disconnected), false),
        normalize(&stale(StaleReason::Expired), false),
        "the reason a field is stale must show"
    );
    let _ = CoreField::Volume;

    // The Operator's text: shown to the Operator, and not compared for an agent.
    let with_issue = stamped(&|s| {
        s.main_zone.mute.last_issue = Some(FieldIssue {
            message: "x".into(),
        })
    });
    assert_ne!(normalize(&with_issue, true), base);
    assert_eq!(normalize(&with_issue, false), agent_base);
    let with_diagnostic = stamped(&|s| s.diagnostics.push("y".into()));
    assert_ne!(normalize(&with_diagnostic, true), base);
    assert_eq!(normalize(&with_diagnostic, false), agent_base);
}
