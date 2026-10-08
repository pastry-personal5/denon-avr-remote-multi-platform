//! The client against a server that says exactly what the test tells it to: a
//! listener on a Unix socket that reads one request and writes canned bytes. It is
//! not a server framework; the client's dependencies must not hold one, and a test
//! that needs a real server is in `apps/api-server`.

use denon_avr_api_client::{ApiClient, Audience, ConnectError, Endpoint, Token};
use denon_avr_api_contract::events::EventName;
use denon_avr_api_contract::{AgentStateView, HealthDto};
use denon_avr_application::{
    ControlError, OperationControl, OperatorAdmin, ReceiverReads, TokenId,
};
use denon_avr_domain::{OperationId, ReceiverId, ReceiverState};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::task::JoinHandle;

const SECRET: &str = "daro_SECRETSECRETSECRETSECRETSECRETSECRETSECRE";

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A short path: a socket's path is limited to about a hundred bytes.
fn socket_path() -> PathBuf {
    PathBuf::from(format!(
        "/tmp/dar-cl-{}-{}.sock",
        std::process::id() % 100_000,
        NEXT.fetch_add(1, Ordering::SeqCst)
    ))
}

struct Fake {
    path: PathBuf,
    served: Option<JoinHandle<Vec<String>>>,
}

impl Fake {
    /// The heads of the requests it was sent, once it has answered them all.
    async fn heads(mut self) -> Vec<String> {
        self.served.take().unwrap().await.unwrap()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Read a request's head.
async fn read_head(stream: &mut UnixStream) -> String {
    let mut head = Vec::new();
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        let mut buffer = [0u8; 1024];
        let read = stream.read(&mut buffer).await.unwrap_or(0);
        if read == 0 {
            break;
        }
        head.extend_from_slice(&buffer[..read]);
    }
    String::from_utf8_lossy(&head).into_owned()
}

/// Answer each connection with the next of `responses`, then stop. The heads it
/// read come back.
fn fake(responses: Vec<Vec<u8>>) -> Fake {
    let path = socket_path();
    let listener = UnixListener::bind(&path).unwrap();
    let served = tokio::spawn(async move {
        let mut heads = Vec::new();
        for response in responses {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            heads.push(read_head(&mut stream).await);
            let _ = stream.write_all(&response).await;
            let _ = stream.shutdown().await;
        }
        heads
    });
    Fake {
        path,
        served: Some(served),
    }
}

fn client(path: &Path, audience: Audience) -> ApiClient {
    ApiClient::connect(Endpoint {
        socket: path.to_owned(),
        token: Token::new(SECRET),
        audience,
    })
    .unwrap()
}

fn response(status: &str, content_type: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n\
         connection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn living_room() -> ReceiverId {
    ReceiverId::new("living-room").unwrap()
}

fn state_event(revision: u64) -> String {
    let mut state = ReceiverState::new(living_room());
    state.revision.0 = revision;
    let json = serde_json::to_string(&AgentStateView::from(&state)).unwrap();
    format!("event: state\ndata: {json}\n\n")
}

/// The head of a stream whose body is delimited by the connection closing.
const STREAM_HEAD: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                           connection: close\r\n\r\n";

#[tokio::test]
async fn a_connection_refused_is_receiver_service_unavailable() {
    let nowhere = socket_path();

    // An agent is told what the design says, and nothing about where it looked.
    let agent = client(&nowhere, Audience::Agent);
    match agent.receivers().await {
        Err(ControlError::Unavailable(text)) => {
            assert_eq!(text, "receiver service unavailable")
        }
        other => panic!("{other:?}"),
    }
    // The Operator is told where.
    let operator = client(&nowhere, Audience::Operator);
    match operator.receivers().await {
        Err(ControlError::Unavailable(text)) => {
            assert!(text.starts_with("receiver service unavailable"), "{text}");
            assert!(text.contains(nowhere.to_str().unwrap()), "{text}");
        }
        other => panic!("{other:?}"),
    }
    // A stream that cannot be opened is the same error.
    assert!(matches!(
        agent.state(&living_room()).await,
        Err(ControlError::Unavailable(_))
    ));
}

#[tokio::test]
async fn the_credential_never_appears_in_debug_or_errors() {
    let nowhere = socket_path();
    let endpoint = Endpoint {
        socket: nowhere.clone(),
        token: Token::new(SECRET),
        audience: Audience::Operator,
    };
    let shown = format!("{endpoint:?} {:?}", endpoint.token);
    assert!(shown.contains("<redacted>"), "{shown}");
    assert!(!shown.contains(SECRET), "{shown}");

    let client = ApiClient::connect(endpoint).unwrap();
    let shown = format!("{client:?}");
    assert!(!shown.contains(SECRET), "{shown}");

    let refused = format!("{:?}", client.receivers().await);
    assert!(!refused.contains(SECRET), "{refused}");

    // A token that cannot be sent is refused without being repeated.
    let bad = ApiClient::connect(Endpoint {
        socket: nowhere,
        token: Token::new("two words"),
        audience: Audience::Agent,
    })
    .unwrap_err();
    assert_eq!(bad, ConnectError::TokenNotWellFormed);
    assert!(!format!("{bad} {bad:?}").contains("two words"));

    // It is sent, and as the bearer credential.
    let server = fake(vec![response(
        "200 OK",
        "application/json",
        r#"{"receivers":[]}"#,
    )]);
    let client = ApiClient::connect(Endpoint {
        socket: server.path.clone(),
        token: Token::new(SECRET),
        audience: Audience::Agent,
    })
    .unwrap();
    client.receivers().await.unwrap();
    let heads = server.heads().await;
    assert!(
        heads[0].to_ascii_lowercase().contains(&format!(
            "authorization: bearer {}",
            SECRET.to_ascii_lowercase()
        )),
        "{}",
        heads[0]
    );
}

#[tokio::test]
async fn a_socket_path_the_system_cannot_hold_is_refused_when_the_client_is_made() {
    let long = PathBuf::from(format!("/tmp/{}", "x".repeat(120)));
    let error = ApiClient::connect(Endpoint {
        socket: long,
        token: Token::new(SECRET),
        audience: Audience::Agent,
    })
    .unwrap_err();
    assert!(
        matches!(error, ConnectError::SocketPathTooLong { .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn an_unknown_status_is_unavailable_and_not_a_guess() {
    // A teapot, a redirect, and a success that is not what the contract says.
    let server = fake(vec![
        response("418 I'm a teapot", "text/plain", "short and stout"),
        response("302 Found", "text/plain", ""),
        response("200 OK", "application/json", "this is not the list"),
        response(
            "400 Bad Request",
            "application/json",
            r#"{"error":{"code":"invalid_request","message":"no such field"}}"#,
        ),
    ]);
    let client = client(&server.path, Audience::Agent);
    for expected in ["the server answered 418", "the server answered 302"] {
        match client.receivers().await {
            Err(ControlError::Unavailable(text)) => assert_eq!(text, expected),
            other => panic!("{other:?}"),
        }
    }
    match client.receivers().await {
        Err(ControlError::Unavailable(text)) => {
            assert!(text.contains("could not be read"), "{text}")
        }
        other => panic!("{other:?}"),
    }
    // An error the contract does describe is the port's error for it.
    assert_eq!(
        client.receivers().await,
        Err(ControlError::InvalidRequest("no such field".into()))
    );
}

#[tokio::test]
async fn a_server_of_another_version_is_reported() {
    let health = HealthDto {
        contract: 99,
        ..serde_json::from_str(
            r#"{"contract":1,"policy":"active","audit":"ok","ledger_ready":true,"approval":"unavailable"}"#,
        )
        .unwrap()
    };
    let server = fake(vec![response(
        "200 OK",
        "application/json",
        &serde_json::to_string(&health).unwrap(),
    )]);
    match client(&server.path, Audience::Agent).health().await {
        Err(ControlError::Unavailable(text)) => {
            assert!(text.contains("99") && text.contains("version"), "{text}")
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn an_agent_client_refuses_the_operators_methods_without_sending_a_request() {
    let listener_path = socket_path();
    let listener = UnixListener::bind(&listener_path).unwrap();
    let agent = client(&listener_path, Audience::Agent);

    assert_eq!(agent.tokens().await, Err(ControlError::Forbidden));
    assert_eq!(
        agent.policy().await.map(|_| ()),
        Err(ControlError::Forbidden)
    );
    assert_eq!(
        agent.configuration().await.map(|_| ()),
        Err(ControlError::Forbidden)
    );
    assert_eq!(
        agent
            .revoke_token(TokenId::new("t-0000ffff").unwrap())
            .await
            .map(|_| ()),
        Err(ControlError::Forbidden)
    );
    assert!(agent.server_health().await.is_err());

    let nothing = tokio::time::timeout(Duration::from_millis(200), listener.accept()).await;
    assert!(nothing.is_err(), "a connection was made");
    let _ = std::fs::remove_file(&listener_path);
}

/// A server that answers one connection by writing each part after its pause, then
/// closing.
fn scripted(parts: Vec<(u64, String)>) -> Fake {
    let path = socket_path();
    let listener = UnixListener::bind(&path).unwrap();
    let served = tokio::spawn(async move {
        let mut heads = Vec::new();
        if let Ok((mut stream, _)) = listener.accept().await {
            heads.push(read_head(&mut stream).await);
            for (pause, part) in parts {
                tokio::time::sleep(Duration::from_millis(pause)).await;
                let _ = stream.write_all(part.as_bytes()).await;
            }
            let _ = stream.shutdown().await;
        }
        heads
    });
    Fake {
        path,
        served: Some(served),
    }
}

#[tokio::test]
async fn state_ends_with_session_closed_on_an_end_event_or_a_closed_stream() {
    // The states are spaced out, so the second is seen before the end that follows.
    let ended = scripted(vec![
        (0, format!("{STREAM_HEAD}{}", state_event(1))),
        (150, state_event(2)),
        (
            150,
            "event: end\ndata: {\"reason\":\"session_closed\"}\n\n".to_owned(),
        ),
    ]);
    let mut subscription = client(&ended.path, Audience::Agent)
        .state(&living_room())
        .await
        .unwrap();
    // The first state is there when the call returns.
    assert_eq!(subscription.latest().revision.0, 1);
    let next = subscription.changed().await.unwrap();
    assert_eq!(next.revision.0, 2, "a later state replaces the first");
    let closed = subscription.changed().await.unwrap_err();
    assert!(closed.to_string().contains("session closed"), "{closed}");

    // The same, with no end event: the connection just closes.
    let dropped = scripted(vec![(0, format!("{STREAM_HEAD}{}", state_event(1)))]);
    let mut subscription = client(&dropped.path, Audience::Agent)
        .state(&living_room())
        .await
        .unwrap();
    assert_eq!(subscription.latest().revision.0, 1);
    let closed = subscription.changed().await.unwrap_err();
    assert!(closed.to_string().contains("session closed"), "{closed}");
}

#[tokio::test]
async fn a_stream_the_server_refuses_is_the_ports_error() {
    let server = fake(vec![response(
        "404 Not Found",
        "application/json",
        r#"{"error":{"code":"not_found","message":"no such receiver","what":"receiver"}}"#,
    )]);
    let error = client(&server.path, Audience::Agent)
        .state(&living_room())
        .await
        .map(|_| ())
        .unwrap_err();
    assert_eq!(error, ControlError::NotFound("receiver"));
}

#[tokio::test]
async fn dropping_a_state_subscription_closes_its_connection() {
    let path = socket_path();
    let listener = UnixListener::bind(&path).unwrap();
    let client = client(&path, Audience::Agent);
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_head(&mut stream).await;
        stream
            .write_all(format!("{STREAM_HEAD}{}", state_event(1)).as_bytes())
            .await
            .unwrap();
        // Wait for the client to hang up.
        let mut buffer = [0u8; 16];
        tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer))
            .await
            .expect("the client closed the connection")
            .unwrap()
    });
    let subscription = client.state(&living_room()).await.unwrap();
    drop(subscription);
    assert_eq!(
        server.await.unwrap(),
        0,
        "the server read the end of the stream"
    );
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn operation_events_end_when_the_stream_does_and_a_missed_one_is_passed_on() {
    let body = "event: missed\ndata: {}\n\nevent: end\ndata: {\"reason\":\"shutdown\"}\n\n";
    let server = fake(vec![format!("{STREAM_HEAD}{body}").into_bytes()]);
    let client = client(&server.path, Audience::Operator);
    let mut events = client.operation_events().await.unwrap();
    assert_eq!(
        events.next().await,
        Some(denon_avr_application::OperationEvent::Missed)
    );
    assert_eq!(events.next().await, None);
    // The name of the end event is part of the contract this test reads.
    assert_eq!(EventName::End.as_str(), "end");
}

#[tokio::test]
async fn too_late_carries_the_operation_as_it_is() {
    let body = r#"{"error":{"code":"too_late","message":"it reached the receiver","operation":
        {"id":7,"receiver":"living-room","intent":{"kind":"mute","value":"on"},
         "status":"in_session","dispatch":"not_dispatched","confirmed":false}}}"#;
    let server = fake(vec![response("409 Conflict", "application/json", body)]);
    match client(&server.path, Audience::Agent)
        .cancel(OperationId(7))
        .await
    {
        Err(ControlError::TooLate(snapshot)) => {
            assert_eq!(snapshot.id, OperationId(7));
            assert_eq!(snapshot.status.as_str(), "in_session");
        }
        other => panic!("{other:?}"),
    }
}
