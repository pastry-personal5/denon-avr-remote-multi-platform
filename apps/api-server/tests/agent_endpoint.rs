//! The Agent endpoint: what it refuses, what it never says, and that policy still
//! decides every write that reaches the receiver through it.

mod support;

use denon_avr_api_contract::paths::request as path;
use denon_avr_api_contract::routes::{Audience, Route, TABLE};
use denon_avr_api_server::Limits;
use denon_avr_application::TokenStore as _;
use denon_avr_application::{
    AgentLabel, AgentLimits, AgentPath, AuditEvent, ControlService, EndpointKind, OperationControl,
    Principal, RefusalReason, ServiceConfig,
};
use denon_avr_domain::{FieldIssue, ReceiverFieldValidity, StaleReason};
use denon_avr_infrastructure::{AuditLimits, JsonlAuditLog, SystemClock, YamlPolicySource};
use serde_json::{json, Value};
use std::sync::Arc;
use support::*;

fn operator_rows() -> Vec<&'static Route> {
    TABLE
        .iter()
        .filter(|route| route.audience == Audience::Operator)
        .collect()
}

fn body_for(route: &Route) -> Option<Vec<u8>> {
    matches!(route.method.as_str(), "POST" | "PUT").then(|| b"{}".to_vec())
}

async fn call(socket: &std::path::Path, route: &Route, token: Option<&str>) -> Reply {
    request(
        socket,
        route.method.as_str(),
        &sample_path(route.pattern),
        token,
        body_for(route).as_deref(),
    )
    .await
}

fn refusals(
    records: &[denon_avr_application::AuditRecord],
) -> Vec<(
    Option<Principal>,
    EndpointKind,
    RefusalReason,
    Option<String>,
)> {
    records
        .iter()
        .filter_map(|record| match &record.event {
            AuditEvent::AccessRefused {
                endpoint,
                reason,
                resource,
                ..
            } => Some((
                record.principal.clone(),
                *endpoint,
                *reason,
                resource.clone(),
            )),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn every_operator_row_of_the_route_table_is_404_on_the_agent_endpoint_for_an_agent_token_and_is_audited(
) {
    let fixture = Fixture::start(Options::default()).await;
    let rows = operator_rows();
    assert!(
        rows.len() >= 13,
        "the table lost Operator rows: {}",
        rows.len()
    );
    let operator = fixture.operator_token();
    let before_tokens = fixture.tokens.list().len();

    // Each row is probed by an agent of its own, so each refusal is its own record.
    for (n, route) in rows.iter().enumerate() {
        let token = fixture.agent_token(&format!("probe-{n}")).await;
        let reply = call(fixture.agent_socket(), route, Some(&token)).await;
        assert_eq!(
            reply.status,
            404,
            "{} {}",
            route.method.as_str(),
            route.pattern
        );
        assert_eq!(reply.code().as_deref(), Some("not_found"));
    }

    // Nothing the rows do happened.
    assert_eq!(fixture.connector.operate_calls(), 0);
    assert_eq!(
        fixture.tokens.list().len(),
        before_tokens + rows.len(),
        "no token was issued or revoked"
    );
    assert!(fixture.tokens.list().iter().all(|t| t.revoked.is_none()));
    assert_eq!(locked(&fixture.config.0).receivers.len(), 1);
    let _ = operator;

    // Every probe is in the log, under its own agent, with the pattern it was after.
    let records = audit_records(&fixture).await;
    let seen = refusals(&records);
    for (n, route) in rows.iter().enumerate() {
        let label = AgentLabel::new(format!("probe-{n}")).unwrap();
        assert!(
            seen.iter().any(|(who, endpoint, reason, resource)| {
                *who == Some(Principal::Agent(label.clone()))
                    && *endpoint == EndpointKind::Agent
                    && *reason == RefusalReason::ResourceNotServed
                    && resource.as_deref() == Some(route.pattern)
            }),
            "{} {} was not audited: {seen:?}",
            route.method.as_str(),
            route.pattern
        );
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn with_the_operator_token_the_same_rows_are_401_and_audited_as_the_operator_token_on_the_agent_endpoint(
) {
    let fixture = Fixture::start(Options::default()).await;
    let operator = fixture.operator_token();
    for route in operator_rows() {
        let reply = call(fixture.agent_socket(), route, Some(&operator)).await;
        assert_eq!(
            reply.status,
            401,
            "{} {}",
            route.method.as_str(),
            route.pattern
        );
        assert_eq!(reply.code().as_deref(), Some("unauthenticated"));
    }
    assert_eq!(fixture.connector.operate_calls(), 0);
    let records = audit_records(&fixture).await;
    let seen = refusals(&records);
    assert!(
        seen.iter().any(|(who, _, reason, _)| who.is_none()
            && *reason == RefusalReason::OperatorTokenOnAgentEndpoint),
        "{seen:?}"
    );
    // The log never holds the token that was presented.
    assert!(!audit_text(&fixture.audit_directory).contains(&operator));
    fixture.shutdown().await;
}

#[tokio::test]
async fn with_no_token_they_are_401() {
    let fixture = Fixture::start(Options::default()).await;
    for route in operator_rows() {
        let reply = call(fixture.agent_socket(), route, None).await;
        assert_eq!(
            reply.status,
            401,
            "{} {}",
            route.method.as_str(),
            route.pattern
        );
    }
    let seen = refusals(&audit_records(&fixture).await);
    assert!(seen
        .iter()
        .any(|(_, _, reason, _)| *reason == RefusalReason::NoCredential));
    fixture.shutdown().await;
}

#[tokio::test]
async fn an_agent_token_on_the_operator_endpoint_is_401_and_audited() {
    let fixture = Fixture::start(Options::default()).await;
    let agent = fixture.agent_token("openclaw").await;
    for route in operator_rows() {
        let reply = call(fixture.operator_socket(), route, Some(&agent)).await;
        assert_eq!(
            reply.status,
            401,
            "{} {}",
            route.method.as_str(),
            route.pattern
        );
    }
    // Not one of them did anything.
    assert_eq!(fixture.tokens.list().len(), 1);
    let seen = refusals(&audit_records(&fixture).await);
    assert!(seen.iter().any(
        |(_, endpoint, reason, _)| *endpoint == EndpointKind::Operator
            && *reason == RefusalReason::AgentTokenOnOperatorEndpoint
    ));
    fixture.shutdown().await;
}

#[tokio::test]
async fn every_both_row_works_on_the_agent_endpoint() {
    let fixture = Fixture::start(Options::default()).await;
    let token = fixture.agent_token("openclaw").await;
    let socket = fixture.agent_socket().to_owned();
    let room = living_room();
    let ok = |reply: &Reply, what: &str| assert_eq!(reply.status, 200, "{what}: {}", reply.text());

    ok(&get(&socket, &path::health(), &token).await, "health");
    ok(&get(&socket, &path::receivers(), &token).await, "receivers");
    ok(&get(&socket, &path::state(&room), &token).await, "state");
    ok(
        &get(&socket, &path::sources(&room), &token).await,
        "sources",
    );
    let body = json!({ "intent": { "kind": "mute", "value": "on" } });
    let dry = request(
        &socket,
        "POST",
        &path::dry_run(&room),
        Some(&token),
        Some(&serde_json::to_vec(&body).unwrap()),
    )
    .await;
    ok(&dry, "dry-run");
    let submitted = request(
        &socket,
        "POST",
        &path::submit(&room),
        Some(&token),
        Some(&serde_json::to_vec(&body).unwrap()),
    )
    .await;
    ok(&submitted, "submit");
    let id = submitted.json()["id"].as_u64().unwrap();
    ok(
        &get(&socket, &path::operation(id, Some(5_000)), &token).await,
        "operation",
    );
    let cancel = request(
        &socket,
        "POST",
        &path::cancel(id),
        Some(&token),
        Some(b"{}"),
    )
    .await;
    assert!(
        cancel.status == 200 || cancel.status == 409,
        "cancel: {}",
        cancel.status
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn no_address_reaches_an_agent_in_any_response_or_error() {
    let fixture = Fixture::start(Options::default()).await;
    // The receiver's state holds text a session can leave: a failed query's message,
    // a raw frame, the receiver's own status text.
    {
        let mut state = locked(&fixture.connector.initial);
        state.main_zone.mute.stale(
            StaleReason::QueryFailed,
            "connecting to 192.0.2.10:23 failed",
        );
        state.zone2_power.last_issue = Some(FieldIssue {
            message: "connecting to 192.0.2.10:23 failed".into(),
        });
        state.zone2_power.validity = ReceiverFieldValidity::Unavailable {
            evidence: denon_avr_domain::receiver_state::ReceiverEvidence::UnavailableStatus(
                "RAWFRAME-192.0.2.10".into(),
            ),
        };
        state.diagnostics = vec!["malformed: MVMAX 98 at 192.0.2.10".into()];
    }
    let operator = fixture.operator_token();
    let agent = fixture.agent_token("openclaw").await;
    let room = living_room();
    let forbidden = [
        "192.0.2",
        "192.168",
        "MVMAX",
        "RAWFRAME",
        "malformed",
        "connecting to",
        "timed out talking",
    ];
    let scan = |what: &str, reply: &Reply| {
        let text = reply.text();
        for needle in forbidden {
            assert!(!text.contains(needle), "{what} carries {needle:?}: {text}");
        }
    };
    let body = serde_json::to_vec(&json!({ "intent": { "kind": "mute", "value": "on" } })).unwrap();
    let socket = fixture.agent_socket().to_owned();

    // Every shared route, and the errors they make.
    for target in [
        path::health(),
        path::receivers(),
        path::state(&room),
        path::sources(&room),
        path::operation(1, None),
        path::operation(999, None),
        path::state(&denon_avr_domain::ReceiverId::new("nowhere").unwrap()),
        "/v1/not-a-resource".to_owned(),
        "/v1/receivers/%FF/state".to_owned(),
    ] {
        scan(&target, &get(&socket, &target, &agent).await);
    }
    for target in [path::dry_run(&room), path::submit(&room)] {
        scan(
            &target,
            &request(&socket, "POST", &target, Some(&agent), Some(&body)).await,
        );
        scan(
            &target,
            &request(
                &socket,
                "POST",
                &target,
                Some(&agent),
                Some(b"{\"nope\":1}"),
            )
            .await,
        );
    }

    // The receiver cannot be reached: the Operator is told why, an agent is not.
    wait_for("the receiver to be released", || {
        fixture.connector.open_sessions() == 0
    })
    .await;
    *locked(&fixture.connector.fail) = Some("connecting to 192.0.2.10:23 was refused".into());
    let reply = get(&socket, &path::state(&room), &agent).await;
    assert_eq!(reply.status, 503);
    scan("an unreachable receiver", &reply);
    let reply = get(&socket, &path::sources(&room), &agent).await;
    scan("an unreachable receiver's sources", &reply);
    let reply = request(
        &socket,
        "POST",
        &path::submit(&room),
        Some(&agent),
        Some(&body),
    )
    .await;
    scan("a write to an unreachable receiver", &reply);
    if let Some(id) = reply.json()["id"].as_u64() {
        scan(
            "its outcome",
            &get(&socket, &path::operation(id, Some(5_000)), &agent).await,
        );
    }
    *locked(&fixture.connector.fail) = None;

    // The same state, to the Operator, carries all of it, so the check can fail.
    let state = get(fixture.operator_socket(), &path::state(&room), &operator)
        .await
        .text();
    assert!(
        state.contains("192.0.2.10") && state.contains("MVMAX"),
        "{state}"
    );
    let sources = get(fixture.operator_socket(), &path::sources(&room), &operator)
        .await
        .text();
    assert!(sources.contains("192.0.2.10"));
    fixture.shutdown().await;
}

#[tokio::test]
async fn the_receivers_list_for_an_agent_has_no_host_and_no_ad_hoc_id() {
    let fixture = Fixture::start(Options::default()).await;
    let operator = fixture.operator_token();
    let agent = fixture.agent_token("openclaw").await;
    let made = request(
        fixture.operator_socket(),
        "POST",
        &path::ad_hoc(),
        Some(&operator),
        Some(br#"{"host":"192.0.2.50"}"#),
    )
    .await;
    assert_eq!(made.status, 200);

    let reply = get(fixture.agent_socket(), &path::receivers(), &agent).await;
    let text = reply.text();
    assert!(text.contains("living-room"));
    for needle in ["adhoc", "192.0.2", "host", "address"] {
        assert!(!text.contains(needle), "{needle}: {text}");
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn an_ad_hoc_receiver_is_404_to_an_agent() {
    let fixture = Fixture::start(Options::default()).await;
    let operator = fixture.operator_token();
    let agent = fixture.agent_token("openclaw").await;
    request(
        fixture.operator_socket(),
        "POST",
        &path::ad_hoc(),
        Some(&operator),
        Some(br#"{"host":"192.0.2.50"}"#),
    )
    .await;
    let id = denon_avr_domain::ReceiverId::ad_hoc("192.0.2.50").unwrap();
    for target in [path::state(&id), path::sources(&id)] {
        let reply = get(fixture.agent_socket(), &target, &agent).await;
        assert_eq!(reply.status, 404, "{target}");
        assert_eq!(reply.json()["error"]["what"], "receiver");
    }
    let body = br#"{"intent":{"kind":"mute","value":"on"}}"#;
    let reply = request(
        fixture.agent_socket(),
        "POST",
        &path::submit(&id),
        Some(&agent),
        Some(body),
    )
    .await;
    assert_eq!(reply.status, 404);
    assert_eq!(fixture.connector.operate_calls(), 0);
    fixture.shutdown().await;
}

#[tokio::test]
async fn an_agent_cannot_read_or_cancel_another_agents_operation() {
    let fixture = Fixture::start(Options::default()).await;
    let a = fixture.agent_token("agent-a").await;
    let b = fixture.agent_token("agent-b").await;
    let operator = fixture.operator_token();
    let socket = fixture.agent_socket().to_owned();
    let body = br#"{"intent":{"kind":"mute","value":"on"}}"#;
    let made = request(
        &socket,
        "POST",
        &path::submit(&living_room()),
        Some(&a),
        Some(body),
    )
    .await;
    let id = made.json()["id"].as_u64().unwrap();

    // The owner sees it; another agent is told there is no such operation.
    assert_eq!(
        get(&socket, &path::operation(id, Some(5_000)), &a)
            .await
            .status,
        200
    );
    let seen_by_b = get(&socket, &path::operation(id, None), &b).await;
    assert_eq!(
        (seen_by_b.status, seen_by_b.json()["error"]["what"].clone()),
        (404, json!("operation"))
    );
    let cancelled_by_b = request(&socket, "POST", &path::cancel(id), Some(&b), Some(b"{}")).await;
    assert_eq!(cancelled_by_b.status, 404);
    // The Operator sees every operation.
    assert_eq!(
        get(
            fixture.operator_socket(),
            &path::operation(id, None),
            &operator
        )
        .await
        .status,
        200
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn an_agent_cannot_name_as_agent_in_a_dry_run() {
    let fixture = Fixture::start(Options::default()).await;
    let agent = fixture.agent_token("openclaw").await;
    let body = br#"{"intent":{"kind":"mute","value":"on"},"as_agent":"someone-else"}"#;
    let reply = request(
        fixture.agent_socket(),
        "POST",
        &path::dry_run(&living_room()),
        Some(&agent),
        Some(body),
    )
    .await;
    assert_eq!(reply.status, 400);
    assert!(reply.json()["error"]["message"]
        .as_str()
        .unwrap()
        .contains("as_agent"));
    fixture.shutdown().await;
}

#[tokio::test]
async fn an_agents_dry_run_shows_no_rule_ids() {
    let fixture = Fixture::start(Options::default()).await;
    let agent = fixture.agent_token("openclaw").await;
    let operator = fixture.operator_token();
    // -25.0 dB is half step -50: above the ceiling, so held for approval.
    let intent = json!({ "kind": "volume", "level": { "half_steps": -50 } });
    let asked = serde_json::to_vec(&json!({ "intent": intent })).unwrap();

    let reply = request(
        fixture.agent_socket(),
        "POST",
        &path::dry_run(&living_room()),
        Some(&agent),
        Some(&asked),
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    let answer = reply.json();
    assert_eq!(answer["decision"], "require_approval");
    assert!(!answer["reasons"].as_array().unwrap().is_empty());
    assert!(answer.get("rules").is_none(), "{answer}");
    for rule in ["volume-ceiling", "volume-step", "volume-budget"] {
        assert!(!reply.text().contains(rule), "{rule}");
    }

    // The Operator, asking as that agent, sees which rules fired.
    let as_agent =
        serde_json::to_vec(&json!({ "intent": intent, "as_agent": "openclaw" })).unwrap();
    let reply = request(
        fixture.operator_socket(),
        "POST",
        &path::dry_run(&living_room()),
        Some(&operator),
        Some(&as_agent),
    )
    .await;
    assert_eq!(reply.status, 200);
    assert!(reply.json()["rules"]
        .as_array()
        .unwrap()
        .iter()
        .any(|rule| rule == "volume-ceiling"));
    fixture.shutdown().await;
}

#[tokio::test]
async fn an_agent_write_through_the_server_is_decided_by_policy() {
    let fixture = Fixture::start(Options::default()).await; // the volume is -35 dB
    let agent = fixture.agent_token("openclaw").await;
    let socket = fixture.agent_socket().to_owned();
    let room = living_room();
    let write = |intent: Value| {
        let (socket, agent, room) = (socket.clone(), agent.clone(), room.clone());
        async move {
            let body = serde_json::to_vec(&json!({ "intent": intent })).unwrap();
            let made = request(
                &socket,
                "POST",
                &path::submit(&room),
                Some(&agent),
                Some(&body),
            )
            .await;
            assert_eq!(made.status, 200, "{}", made.text());
            let id = made.json()["id"].as_u64().unwrap();
            get(&socket, &path::operation(id, Some(5_000)), &agent)
                .await
                .json()
        }
    };

    // Allowed: a mute reaches the receiver once.
    let done = write(json!({ "kind": "mute", "value": "on" })).await;
    assert_eq!(done["status"], "completed");
    assert_eq!(fixture.connector.operate_calls(), 1);

    // Above the ceiling (-30 dB): held, and nothing is sent.
    let held = write(json!({ "kind": "volume", "level": { "half_steps": -50 } })).await;
    assert_eq!(held["status"], "approval_unavailable");
    assert_eq!(held["dispatch"], "not_dispatched");
    assert_eq!(fixture.connector.operate_calls(), 1);

    // Above the hard limit (-20 dB): denied, and nothing is sent.
    let denied = write(json!({ "kind": "volume", "level": { "half_steps": -30 } })).await;
    assert_eq!(denied["status"], "denied");
    assert_eq!(fixture.connector.operate_calls(), 1);

    // Within the limits: a small rise from -35 dB to -34 dB goes through.
    let rise = write(json!({ "kind": "volume", "level": { "half_steps": -68 } })).await;
    assert_eq!(rise["status"], "completed");
    assert_eq!(fixture.connector.operate_calls(), 2);
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_label_typo_is_refused_at_issue_and_through_the_server() {
    let fixture = Fixture::start(Options::default()).await;
    let operator = fixture.operator_token();
    for label in ["Claude-Code", "claude code", "CLAUDE", "unauthenticated"] {
        let body = serde_json::to_vec(&json!({ "label": label })).unwrap();
        let reply = request(
            fixture.operator_socket(),
            "POST",
            &path::tokens(),
            Some(&operator),
            Some(&body),
        )
        .await;
        assert_eq!(reply.status, 400, "{label}");
    }
    assert!(fixture.tokens.list().is_empty());
    fixture.shutdown().await;
}

#[tokio::test]
async fn an_agent_that_holds_every_agent_connection_does_not_starve_the_operator() {
    let fixture = Fixture::start(Options {
        limits: Limits {
            agent_connections: 2,
            ..Limits::default()
        },
        ..Options::default()
    })
    .await;
    let operator = fixture.operator_token();
    let agent = fixture.agent_token("openclaw").await;
    let held = futures_connect(fixture.agent_socket(), 2).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // The Agent endpoint is full, so one more connection is closed unserved. The
    // Operator's endpoint is a different one, and is not.
    let request_bytes = format!(
        "GET /v1/health HTTP/1.1\r\nHost: dar\r\nConnection: close\r\nAuthorization: Bearer {agent}\r\n\r\n"
    );
    let reply = send_raw(fixture.agent_socket(), request_bytes.as_bytes()).await;
    assert!(reply.is_empty(), "served while the endpoint was full");
    assert_eq!(
        get(fixture.operator_socket(), "/v1/health", &operator)
            .await
            .status,
        200
    );

    // And when the agent lets go, it is served again.
    drop(held);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(
        get(fixture.agent_socket(), "/v1/health", &agent)
            .await
            .status,
        200
    );
    fixture.shutdown().await;
}

async fn futures_connect(socket: &std::path::Path, count: usize) -> Vec<tokio::net::UnixStream> {
    let mut held = Vec::new();
    for _ in 0..count {
        held.push(tokio::net::UnixStream::connect(socket).await.unwrap());
    }
    held
}

#[tokio::test]
async fn a_flood_of_refused_requests_writes_a_bounded_number_of_records_and_the_ledger_still_rebuilds(
) {
    let limits = AuditLimits {
        max_file_bytes: 8_000,
        max_files: 3,
    };
    let fixture = Fixture::start(Options {
        volume_db: -60.0,
        audit_limits: Some(limits),
        ..Options::default()
    })
    .await;
    let operator = fixture.operator_token();
    let room = living_room();

    // The Operator raises the volume from -60 dB to -45 dB: the ledger counts it.
    let body = serde_json::to_vec(
        &json!({ "intent": { "kind": "volume", "level": { "half_steps": -90 } } }),
    )
    .unwrap();
    let made = request(
        fixture.operator_socket(),
        "POST",
        &path::submit(&room),
        Some(&operator),
        Some(&body),
    )
    .await;
    assert_eq!(made.status, 200, "{}", made.text());
    let id = made.json()["id"].as_u64().unwrap();
    let done = get(
        fixture.operator_socket(),
        &path::operation(id, Some(5_000)),
        &operator,
    )
    .await;
    assert_eq!(done.json()["status"], "completed");
    let dispatching = |text: &str| text.matches("\"kind\":\"dispatching\"").count();
    assert_eq!(dispatching(&all_audit_text(&fixture.audit_directory)), 1);

    // A flood: many requests with no credential, and agents of many labels probing.
    for _ in 0..1_500 {
        request(fixture.agent_socket(), "GET", "/v1/health", None, None).await;
    }
    for n in 0..60 {
        let token = fixture.agent_token(&format!("flood-{n}")).await;
        request(
            fixture.agent_socket(),
            "GET",
            "/v1/policy",
            Some(&token),
            None,
        )
        .await;
    }

    // At most thirty refusals were written, and the Operator's record is still there.
    let records = audit_records(&fixture).await;
    let written = refusals(&records).len();
    assert!(
        (1..=30).contains(&written),
        "{written} refusals were written"
    );
    assert_eq!(
        dispatching(&all_audit_text(&fixture.audit_directory)),
        1,
        "the flood rotated the budget's record away"
    );

    // A new service over the same log rebuilds the ledger from it: the Operator's
    // rise from -60 dB still counts, so an agent's half-decibel rise from -45 dB is
    // held, being 15.5 dB above the lowest level of the window.
    let connector = FakeConnector::new(-45.0);
    let config = locked(&fixture.config.0).clone();
    let restarted = ControlService::start(
        connector.clone(),
        Arc::new(FakeConfig(std::sync::Mutex::new(config))),
        Arc::new(FakeDiscovery::new(Vec::new())),
        ServiceConfig::default(),
        AgentPath {
            policy: Arc::new(YamlPolicySource::new(fixture.root.0.join("policy.yaml"))),
            audit: Arc::new(JsonlAuditLog::new(&fixture.audit_directory).with_limits(limits)),
            clock: Arc::new(SystemClock),
            limits: AgentLimits::default(),
            tokens: None,
        },
    )
    .await;
    let agent = restarted
        .handle(Principal::Agent(AgentLabel::new("openclaw").unwrap()))
        .unwrap();
    let submission = denon_avr_application::OperationSubmission::new(
        denon_avr_domain::ReceiverIntent::Volume(master(-44.5)),
    );
    let first = agent.submit(&room, submission).await.unwrap();
    let ended = agent
        .operation(first.id, Some(std::time::Duration::from_secs(5)))
        .await
        .unwrap();
    assert_eq!(ended.status.as_str(), "approval_unavailable", "{ended:?}");
    assert_eq!(connector.operate_calls(), 0);
    fixture.shutdown().await;
}

/// All the audit files' text, the rotated ones included.
fn all_audit_text(directory: &std::path::Path) -> String {
    let mut text = String::new();
    if let Ok(entries) = std::fs::read_dir(directory) {
        for entry in entries.flatten() {
            text.push_str(&std::fs::read_to_string(entry.path()).unwrap_or_default());
        }
    }
    text
}
