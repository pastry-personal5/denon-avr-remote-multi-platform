//! What a request passes through before a handler sees it: the credential, the
//! sizes and the times, the connection cap, and the health route that shows it.

mod support;

use denon_avr_api_server::Limits;
use std::time::Duration;
use support::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

fn quick() -> Limits {
    Limits {
        head_timeout: Duration::from_millis(300),
        body_timeout: Duration::from_millis(300),
        ..Limits::default()
    }
}

async fn both(fixture: &Fixture) -> Vec<(&'static str, std::path::PathBuf, String)> {
    vec![
        (
            "operator",
            fixture.operator_socket().to_owned(),
            fixture.operator_token(),
        ),
        (
            "agent",
            fixture.agent_socket().to_owned(),
            fixture.agent_token("openclaw").await,
        ),
    ]
}

#[tokio::test]
async fn a_request_without_a_credential_is_401_whatever_the_path() {
    let fixture = Fixture::start(Options::default()).await;
    for socket in [fixture.operator_socket(), fixture.agent_socket()] {
        let mut bodies = Vec::new();
        for (method, path) in [
            ("GET", "/v1/health"),
            ("GET", "/v1/nothing/here"),
            ("GET", "/v1/policy"),
            ("POST", "/v1/tokens"),
            ("DELETE", "/v1/tokens/t-0000002a"),
            ("GET", "/"),
            ("GET", "/v1/operations/events"),
        ] {
            let reply = request(socket, method, path, None, None).await;
            assert_eq!(reply.status, 401, "{method} {path}");
            assert_eq!(reply.code().as_deref(), Some("unauthenticated"));
            bodies.push(reply.text());
        }
        // The same words for a path that exists and one that does not.
        assert!(
            bodies.windows(2).all(|pair| pair[0] == pair[1]),
            "{bodies:?}"
        );
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_refusal_is_audited_without_the_token_it_carried() {
    let fixture = Fixture::start(Options::default()).await;
    let wrong = "dara_THIS-IS-NOT-A-TOKEN-0123456789abcdefghijklmnopq";
    assert_eq!(
        get(fixture.agent_socket(), "/v1/health", wrong)
            .await
            .status,
        401
    );
    let service = fixture.service.operator();
    let page = service
        .audit(denon_avr_application::AuditQuery::new(50))
        .await
        .unwrap();
    let refused: Vec<_> = page
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.record.event,
                denon_avr_application::AuditEvent::AccessRefused { .. }
            )
        })
        .collect();
    assert_eq!(refused.len(), 1);
    assert!(refused[0].record.principal.is_none());
    assert!(!audit_text(&fixture.audit_directory).contains("THIS-IS-NOT-A-TOKEN"));
    fixture.shutdown().await;
}

#[tokio::test]
async fn two_authorization_headers_are_refused() {
    let fixture = Fixture::start(Options::default()).await;
    for (_, socket, token) in both(&fixture).await {
        let bytes = format!(
            "GET /v1/health HTTP/1.1\r\nHost: dar\r\nConnection: close\r\n\
             Authorization: Bearer {token}\r\nAuthorization: Bearer {token}\r\n\r\n"
        );
        let reply = parse_reply(&send_raw(&socket, bytes.as_bytes()).await).unwrap();
        assert_eq!(reply.status, 401);
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_token_in_the_query_string_is_400() {
    let fixture = Fixture::start(Options::default()).await;
    for (_, socket, token) in both(&fixture).await {
        for query in [
            "token=x",
            "access_token=x",
            "Token=x",
            "a=1&api_key=x",
            "bearer=x",
        ] {
            // Even with a perfectly good header beside it.
            let reply = get(&socket, &format!("/v1/health?{query}"), &token).await;
            assert_eq!(reply.status, 400, "{query}");
            assert_eq!(reply.code().as_deref(), Some("invalid_request"));
            assert!(!reply.text().contains("x\""), "the value is never echoed");
        }
        // An ordinary parameter is not one.
        assert_eq!(get(&socket, "/v1/health?limit=5", &token).await.status, 200);
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_scheme_other_than_bearer_is_401() {
    let fixture = Fixture::start(Options::default()).await;
    for (_, socket, token) in both(&fixture).await {
        for header in [
            format!("Basic {token}"),
            "Bearer".to_owned(),
            "Bearer ".to_owned(),
            token.clone(),
            format!("Token {token}"),
        ] {
            let bytes = format!(
                "GET /v1/health HTTP/1.1\r\nHost: dar\r\nConnection: close\r\nAuthorization: {header}\r\n\r\n"
            );
            let reply = parse_reply(&send_raw(&socket, bytes.as_bytes()).await).unwrap();
            assert_eq!(reply.status, 401, "{header}");
        }
        // The scheme's case does not matter.
        let bytes = format!(
            "GET /v1/health HTTP/1.1\r\nHost: dar\r\nConnection: close\r\nAuthorization: bearer {token}\r\n\r\n"
        );
        assert_eq!(
            parse_reply(&send_raw(&socket, bytes.as_bytes()).await)
                .unwrap()
                .status,
            200
        );
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn an_agent_token_is_accepted_only_on_the_agent_endpoint_and_the_operator_token_only_on_the_operator_endpoint(
) {
    let fixture = Fixture::start(Options::default()).await;
    let operator = fixture.operator_token();
    let agent = fixture.agent_token("openclaw").await;
    assert_eq!(
        get(fixture.operator_socket(), "/v1/health", &operator)
            .await
            .status,
        200
    );
    assert_eq!(
        get(fixture.agent_socket(), "/v1/health", &agent)
            .await
            .status,
        200
    );
    assert_eq!(
        get(fixture.operator_socket(), "/v1/health", &agent)
            .await
            .status,
        401
    );
    assert_eq!(
        get(fixture.agent_socket(), "/v1/health", &operator)
            .await
            .status,
        401
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_head_over_16_kib_is_refused() {
    let fixture = Fixture::start(Options::default()).await;
    let token = fixture.operator_token();
    let bytes = format!(
        "GET /v1/health HTTP/1.1\r\nHost: dar\r\nConnection: close\r\nAuthorization: Bearer {token}\r\nX-Padding: {}\r\n\r\n",
        "a".repeat(20 * 1024)
    );
    let reply = send_raw(fixture.operator_socket(), bytes.as_bytes()).await;
    match parse_reply(&reply) {
        Some(reply) => assert!(
            reply.status == 431 || reply.status == 400,
            "{}",
            reply.status
        ),
        None => assert!(reply.is_empty(), "closed without serving it"),
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_slow_head_is_closed_when_the_head_timeout_passes() {
    let fixture = Fixture::start(Options {
        limits: quick(),
        ..Options::default()
    })
    .await;
    let mut stream = UnixStream::connect(fixture.operator_socket())
        .await
        .unwrap();
    // Half a head, then nothing.
    stream
        .write_all(b"GET /v1/health HTTP/1.1\r\nHost: d")
        .await
        .unwrap();
    let mut buffer = Vec::new();
    let started = std::time::Instant::now();
    let closed =
        tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut buffer)).await;
    assert!(
        closed.is_ok(),
        "the server held a half-sent head for three seconds"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_body_over_the_cap_is_413_before_it_is_read() {
    let fixture = Fixture::start(Options::default()).await;
    for (_, socket, token) in both(&fixture).await {
        // A length is declared and no byte of the body is sent.
        let bytes = format!(
            "POST /v1/receivers/living-room/operations HTTP/1.1\r\nHost: dar\r\nConnection: close\r\n\
             Authorization: Bearer {token}\r\nContent-Type: application/json\r\n\
             Content-Length: 100000\r\n\r\n"
        );
        let reply = parse_reply(&send_raw(&socket, bytes.as_bytes()).await).unwrap();
        assert_eq!(reply.status, 413);
        assert_eq!(reply.code().as_deref(), Some("payload_too_large"));
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_chunked_body_is_411() {
    let fixture = Fixture::start(Options::default()).await;
    for (_, socket, token) in both(&fixture).await {
        let bytes = format!(
            "POST /v1/receivers/living-room/operations HTTP/1.1\r\nHost: dar\r\nConnection: close\r\n\
             Authorization: Bearer {token}\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n"
        );
        let reply = parse_reply(&send_raw(&socket, bytes.as_bytes()).await).unwrap();
        assert_eq!(reply.status, 411);
        // A body with no declared length at all.
        let bytes = format!(
            "POST /v1/receivers/living-room/operations HTTP/1.1\r\nHost: dar\r\nConnection: close\r\n\
             Authorization: Bearer {token}\r\n\r\n"
        );
        let reply = parse_reply(&send_raw(&socket, bytes.as_bytes()).await).unwrap();
        assert_eq!(reply.status, 411);
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn the_connection_cap_closes_the_extra_connection() {
    let fixture = Fixture::start(Options {
        limits: Limits {
            operator_connections: 2,
            ..Limits::default()
        },
        ..Options::default()
    })
    .await;
    let socket = fixture.operator_socket().to_owned();
    let token = fixture.operator_token();
    // Two connections held open without a request.
    let first = UnixStream::connect(&socket).await.unwrap();
    let second = UnixStream::connect(&socket).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // A third is closed at once, without being served.
    let mut third = UnixStream::connect(&socket).await.unwrap();
    let mut buffer = Vec::new();
    let closed = tokio::time::timeout(Duration::from_secs(2), third.read_to_end(&mut buffer)).await;
    assert!(closed.is_ok(), "the third connection was held");
    assert!(buffer.is_empty());

    // When one is released, a new connection is served.
    drop(first);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(get(&socket, "/v1/health", &token).await.status, 200);
    drop(second);
    fixture.shutdown().await;
}

#[tokio::test]
async fn health_answers_with_the_contract_version() {
    let fixture = Fixture::start(Options::default()).await;
    for (_, socket, token) in both(&fixture).await {
        let reply = get(&socket, "/v1/health", &token).await;
        assert_eq!(reply.status, 200);
        assert_eq!(
            reply.header("content-type"),
            Some("application/json; charset=utf-8")
        );
        let health = reply.json();
        assert_eq!(health["contract"], 1);
        assert_eq!(health["policy"], "active");
        assert_eq!(health["audit"], "ok");
        assert_eq!(health["approval"], "unavailable");
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn health_on_the_operator_endpoint_reports_the_agent_endpoint_and_the_token_store() {
    let on = Fixture::start(Options::default()).await;
    let token = on.operator_token();
    let health = get(on.operator_socket(), "/v1/health", &token).await.json();
    assert_eq!(health["server"]["agent_endpoint"]["state"], "on");
    assert_eq!(health["server"]["token_store"], "ok");
    on.shutdown().await;

    let off = Fixture::start(Options {
        agent_endpoint: false,
        ..Options::default()
    })
    .await;
    let token = off.operator_token();
    let health = get(off.operator_socket(), "/v1/health", &token)
        .await
        .json();
    assert_eq!(health["server"]["agent_endpoint"]["state"], "off");
    assert!(health["server"]["agent_endpoint"]["reason"]
        .as_str()
        .unwrap()
        .contains("no agent endpoint"));
    off.shutdown().await;
}

#[tokio::test]
async fn health_on_the_agent_endpoint_has_no_server_section() {
    let fixture = Fixture::start(Options::default()).await;
    let token = fixture.agent_token("openclaw").await;
    let health = get(fixture.agent_socket(), "/v1/health", &token)
        .await
        .json();
    assert!(health.get("server").is_none(), "{health}");
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_valid_credential_on_a_path_that_is_not_a_resource_is_404_with_the_contract_error() {
    let fixture = Fixture::start(Options::default()).await;
    for (_, socket, token) in both(&fixture).await {
        let reply = get(&socket, "/v1/nothing", &token).await;
        assert_eq!(reply.status, 404);
        assert_eq!(reply.code().as_deref(), Some("not_found"));
        let reply = request(&socket, "POST", "/v1/health", Some(&token), None).await;
        assert_eq!(reply.status, 405);
        assert_eq!(reply.code().as_deref(), Some("method_not_allowed"));
        assert!(reply.header("allow").is_some());
    }
    fixture.shutdown().await;
}
