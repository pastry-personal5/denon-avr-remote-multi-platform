//! Every route answers what the port answers, errors reach the wire as the
//! contract documents them, and a request is read strictly.

mod support;

use denon_avr_api_contract::admin::{
    AdHocResponse, AuditPageDto, ConfigDto, DiscoveredListDto, HttpInformationDto, IssuedTokenDto,
    PolicyDto, QuickSelectNamesDto, ReadinessDto, TokenDto, TokenListDto,
};
use denon_avr_api_contract::paths::request as path;
use denon_avr_api_contract::requests::{AgentDryRun, OperatorDryRun};
use denon_avr_api_contract::routes::{served_to, TABLE};
use denon_avr_api_contract::{
    AgentSourcesView, AgentStateView, OperationDto, OperatorSourcesView, OperatorStateView,
    ReceiversDto, RouteId,
};
use denon_avr_api_server::Limits;
use denon_avr_application::{
    AgentLabel, ControlError, OperationControl, OperatorAdmin, Principal, ReceiverReads,
};
use denon_avr_domain::{
    ConfiguredReceivers, DiscoveredReceiver, MuteState, ReceiverEndpoint, ReceiverId,
    ReceiverIdentity, ReceiverIntent,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;
use support::*;

async fn post_json(socket: &std::path::Path, path: &str, token: &str, body: &Value) -> Reply {
    request(
        socket,
        "POST",
        path,
        Some(token),
        Some(serde_json::to_vec(body).unwrap().as_slice()),
    )
    .await
}

fn mute_on() -> Value {
    json!({ "intent": { "kind": "mute", "value": "on" } })
}

#[tokio::test]
async fn each_both_resource_returns_what_the_port_returns() {
    let fixture = Fixture::start(Options::default()).await;
    let operator_token = fixture.operator_token();
    let agent_token = fixture.agent_token("openclaw").await;
    let room = living_room();
    let operator = fixture.service.handle(Principal::Operator).unwrap();
    let agent = fixture
        .service
        .handle(Principal::Agent(AgentLabel::new("openclaw").unwrap()))
        .unwrap();

    for (socket, token, is_operator) in [
        (fixture.operator_socket().to_owned(), operator_token, true),
        (fixture.agent_socket().to_owned(), agent_token, false),
    ] {
        let handle = if is_operator { &operator } else { &agent };

        // receivers
        let reply = get(&socket, &path::receivers(), &token).await;
        assert_eq!(reply.status, 200);
        let expected = handle.receivers().await.unwrap();
        let expected: Vec<_> = expected
            .iter()
            .map(denon_avr_api_contract::ReceiverSummaryDto::from)
            .collect();
        assert_eq!(
            reply.json(),
            json!(ReceiversDto {
                receivers: expected
            })
        );

        // state: the view that endpoint serves, of the state the port holds
        let reply = get(&socket, &path::state(&room), &token).await;
        assert_eq!(reply.status, 200);
        let latest = handle.state(&room).await.unwrap().latest();
        let expected = if is_operator {
            json!(OperatorStateView::from(&latest))
        } else {
            json!(AgentStateView::from(&latest))
        };
        assert_eq!(reply.json(), expected);

        // sources
        let reply = get(&socket, &path::sources(&room), &token).await;
        assert_eq!(reply.status, 200);
        let catalog = handle.source_catalog(&room).await.unwrap();
        let expected = if is_operator {
            json!(OperatorSourcesView::from(&catalog))
        } else {
            json!(AgentSourcesView::from(&catalog))
        };
        assert_eq!(reply.json(), expected);

        // dry run
        let intent = ReceiverIntent::Mute(MuteState::On);
        let reply = post_json(&socket, &path::dry_run(&room), &token, &mute_on()).await;
        assert_eq!(reply.status, 200, "{}", reply.text());
        let expected = handle.dry_run(&room, intent).await.unwrap();
        let expected = if is_operator {
            json!(OperatorDryRun::from(&expected))
        } else {
            json!(AgentDryRun::from(&expected))
        };
        assert_eq!(reply.json(), expected);

        // submit, then read it, then try to cancel it
        let reply = post_json(&socket, &path::submit(&room), &token, &mute_on()).await;
        assert_eq!(reply.status, 200, "{}", reply.text());
        let submitted: OperationDto = serde_json::from_value(reply.json()).unwrap();
        assert_eq!(submitted.receiver, "living-room");
        let reply = get(&socket, &path::operation(submitted.id, Some(5_000)), &token).await;
        assert_eq!(reply.status, 200);
        let finished: OperationDto = serde_json::from_value(reply.json()).unwrap();
        assert_eq!(finished.id, submitted.id);
        assert_eq!(json!(finished.status), "completed");
        let port = handle
            .operation(denon_avr_domain::OperationId(submitted.id), None)
            .await
            .unwrap();
        assert_eq!(finished, OperationDto::from(&port));

        // Too late: the operation has finished.
        let reply = post_json(&socket, &path::cancel(submitted.id), &token, &json!({})).await;
        assert_eq!(reply.status, 409);
        assert_eq!(reply.code().as_deref(), Some("too_late"));
        assert_eq!(reply.json()["error"]["operation"]["status"], "completed");
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn each_operator_resource_returns_what_the_port_returns() {
    let fixture = Fixture::start(Options::default()).await;
    let token = fixture.operator_token();
    let socket = fixture.operator_socket().to_owned();
    let room = living_room();
    let operator = fixture.service.handle(Principal::Operator).unwrap();

    // configuration
    let reply = get(&socket, &path::config(), &token).await;
    assert_eq!(reply.status, 200);
    let config = operator.configuration().await.unwrap();
    assert_eq!(reply.json(), json!(ConfigDto::from(&config)));
    assert!(reply
        .header("etag")
        .is_some_and(|tag| tag.starts_with('"') && tag.len() == 66));

    // the inspection reads
    let reply = get(&socket, &path::quick_select_names(&room), &token).await;
    assert_eq!(reply.status, 200);
    let names = operator.quick_select_names(&room).await.unwrap();
    assert_eq!(reply.json(), json!(QuickSelectNamesDto::from(&names)));
    let reply = get(&socket, &path::http_information(&room), &token).await;
    assert_eq!(reply.status, 200);
    let info = operator.http_information(&room).await.unwrap();
    assert_eq!(reply.json(), json!(HttpInformationDto::from(&info)));
    let reply = post_json(&socket, &path::refresh(&room), &token, &json!({})).await;
    assert_eq!(reply.status, 200);
    let readiness: ReadinessDto = serde_json::from_value(reply.json()).unwrap();
    assert!(readiness.ready);

    // the policy
    let reply = get(&socket, &path::policy(), &token).await;
    assert_eq!(reply.status, 200);
    let view = operator.policy().await.unwrap();
    assert_eq!(reply.json(), json!(PolicyDto::from(&view)));
    let reply = post_json(&socket, &path::policy_reload(), &token, &json!({})).await;
    assert_eq!(reply.status, 200);
    let reloaded: PolicyDto = serde_json::from_value(reply.json()).unwrap();
    assert_eq!(reloaded.digest, PolicyDto::from(&view).digest);

    // the audit log
    let reply = get(&socket, &path::audit(50, None), &token).await;
    assert_eq!(reply.status, 200);
    let page: AuditPageDto = serde_json::from_value(reply.json()).unwrap();
    assert!(!page.entries.is_empty());

    // ad hoc receivers
    let ad_hoc = json!({ "host": "192.0.2.50", "friendly_name": "Den" });
    let reply = post_json(&socket, &path::ad_hoc(), &token, &ad_hoc).await;
    assert_eq!(reply.status, 200);
    let made: AdHocResponse = serde_json::from_value(reply.json()).unwrap();
    assert_eq!(made.id, "adhoc:192.0.2.50");
    let again = post_json(&socket, &path::ad_hoc(), &token, &json!({ "host": "" })).await;
    assert_eq!(again.status, 400);
    fixture.shutdown().await;
}

#[tokio::test]
async fn register_ad_hoc_returns_an_ad_hoc_id_the_state_route_can_use() {
    let fixture = Fixture::start(Options::default()).await;
    let token = fixture.operator_token();
    let socket = fixture.operator_socket().to_owned();
    let reply = post_json(
        &socket,
        &path::ad_hoc(),
        &token,
        &json!({ "host": "192.0.2.50" }),
    )
    .await;
    let made: AdHocResponse = serde_json::from_value(reply.json()).unwrap();
    let id = denon_avr_api_contract::parse_receiver_id(&made.id).unwrap();
    assert!(id.is_ad_hoc());
    let reply = get(&socket, &path::state(&id), &token).await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    assert_eq!(reply.json()["receiver"], "adhoc:192.0.2.50");
    fixture.shutdown().await;
}

#[tokio::test]
async fn every_control_error_reaches_the_wire_as_documented() {
    let fixture = Fixture::start(Options::default()).await;
    let token = fixture.operator_token();
    let socket = fixture.operator_socket().to_owned();
    let room = living_room();

    // NotFound, with the word that says what.
    let reply = get(
        &socket,
        &path::state(&ReceiverId::new("nowhere").unwrap()),
        &token,
    )
    .await;
    assert_eq!(
        (reply.status, reply.code().as_deref()),
        (404, Some("not_found"))
    );
    assert_eq!(reply.json()["error"]["what"], "receiver");
    let reply = get(&socket, &path::operation(99_999, None), &token).await;
    assert_eq!(
        (reply.status, reply.json()["error"]["what"].clone()),
        (404, json!("operation"))
    );
    let reply = request(
        &socket,
        "DELETE",
        "/v1/tokens/t-0000ffff",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(
        (reply.status, reply.json()["error"]["what"].clone()),
        (404, json!("token"))
    );

    // InvalidRequest, from the port's own validation.
    let bad_source = json!({ "intent": { "kind": "source", "id": "" } });
    let reply = post_json(&socket, &path::submit(&room), &token, &bad_source).await;
    assert_eq!(
        (reply.status, reply.code().as_deref()),
        (400, Some("invalid_request"))
    );
    let reply = request(&socket, "GET", "/v1/operations/seven", Some(&token), None).await;
    assert_eq!(reply.status, 400);

    // Unavailable, when the receiver cannot be reached. The Operator is told why;
    // the text is the connector's.
    *locked(&fixture.connector.fail) = Some("connecting to 192.0.2.10:23 was refused".into());
    let reply = get(&socket, &path::state(&room), &token).await;
    assert_eq!(
        (reply.status, reply.code().as_deref()),
        (503, Some("unavailable"))
    );
    *locked(&fixture.connector.fail) = None;

    // RateLimited, with Retry-After in whole seconds. Identical writes in flight are
    // one operation, so each is waited for before the next is made.
    let agent = fixture.agent_token("openclaw").await;
    let mut limited = None;
    for i in 0..40 {
        let value = if i % 2 == 0 { "on" } else { "off" };
        let body = json!({ "intent": { "kind": "mute", "value": value } });
        let reply = post_json(fixture.agent_socket(), &path::submit(&room), &agent, &body).await;
        if reply.status == 429 {
            limited = Some(reply);
            break;
        }
        assert_eq!(reply.status, 200, "{}", reply.text());
        let id = reply.json()["id"].as_u64().unwrap();
        get(
            fixture.agent_socket(),
            &path::operation(id, Some(5_000)),
            &agent,
        )
        .await;
    }
    let limited = limited.expect("the write cap is reached within forty writes");
    assert_eq!(limited.code().as_deref(), Some("rate_limited"));
    let header: u64 = limited
        .header("retry-after")
        .expect("Retry-After")
        .parse()
        .unwrap();
    assert!((1..=60).contains(&header), "{header}");
    assert_eq!(limited.json()["error"]["retry_after_secs"], header);
    fixture.shutdown().await;
}

#[tokio::test]
async fn receiver_ids_with_slash_space_percent_question_mark_and_unicode_reach_the_port_intact() {
    let fixture = Fixture::start(Options::default()).await;
    let names = [
        "a/b",
        "living room",
        "100%",
        "what?",
        "Büro #2",
        "a%2Fb",
        "..",
        "x y/z?w%v",
        // Named like a route's own segment: the router must fall back to the id.
        "discover",
        "ad-hoc",
        "events",
        "health",
    ];
    {
        let mut config = locked(&fixture.config.0);
        for name in names {
            config
                .receivers
                .insert(name.to_owned(), ReceiverIdentity::ad_hoc("192.0.2.77"));
        }
    }
    let token = fixture.operator_token();
    for name in names {
        let id = ReceiverId::new(name).unwrap();
        let reply = get(fixture.operator_socket(), &path::state(&id), &token).await;
        assert_eq!(reply.status, 200, "{name}: {}", reply.text());
        assert_eq!(reply.json()["receiver"], name, "{name}");
        // And it is the right receiver for a write.
        let reply = post_json(
            fixture.operator_socket(),
            &path::submit(&id),
            &token,
            &mute_on(),
        )
        .await;
        assert_eq!(reply.status, 200, "{name}");
        assert_eq!(reply.json()["receiver"], name);
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn an_id_that_is_not_utf8_or_that_receiver_id_refuses_is_a_bad_request() {
    let fixture = Fixture::start(Options::default()).await;
    let token = fixture.operator_token();
    for bad in [
        "/v1/receivers/%FF/state",
        "/v1/receivers/%00/state",
        "/v1/receivers/adhoc:/state",
        "/v1/receivers/%0A/state",
    ] {
        let reply = get(fixture.operator_socket(), bad, &token).await;
        assert_eq!(reply.status, 400, "{bad}: {}", reply.text());
        assert_eq!(reply.code().as_deref(), Some("invalid_request"), "{bad}");
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn routing_matches_what_it_should_and_nothing_else() {
    let fixture = Fixture::start(Options::default()).await;
    let token = fixture.operator_token();
    let socket = fixture.operator_socket().to_owned();
    for (method, target, status) in [
        ("GET", "/v1/receivers/", 404),
        // An empty id reaches the handler, which refuses it.
        ("GET", "/v1/receivers//state", 400),
        ("GET", "/v1/receivers/living-room/state/", 404),
        ("GET", "/v1/receivers/../policy", 404),
        ("GET", "/v1//health", 404),
        ("GET", "/health", 404),
        ("GET", "/v2/health", 404),
        ("PATCH", "/v1/health", 405),
        ("DELETE", "/v1/health", 405),
        ("GET", "/v1/receivers/living-room/state?whatever=1", 200),
        ("GET", "/v1/health?x=%20", 200),
    ] {
        let reply = request(&socket, method, target, Some(&token), None).await;
        assert_eq!(reply.status, status, "{method} {target}: {}", reply.text());
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn every_row_of_the_table_has_a_handler_and_no_route_exists_outside_it() {
    let fixture = Fixture::start(Options::default()).await;
    let operator = fixture.operator_token();
    let agent = fixture.agent_token("openclaw").await;
    // The event streams are step 7's.
    let served = |route: &&denon_avr_api_contract::Route| {
        !matches!(route.id, RouteId::StateEvents | RouteId::OperationEvents)
    };
    for (socket, token, is_operator) in [
        (fixture.operator_socket().to_owned(), operator.clone(), true),
        (fixture.agent_socket().to_owned(), agent, false),
    ] {
        for route in served_to(is_operator).filter(served) {
            let reply = request(
                &socket,
                route.method.as_str(),
                &sample_path(route.pattern),
                Some(&token),
                (route.method.as_str() != "GET" && route.method.as_str() != "DELETE")
                    .then_some(b"{}".as_slice()),
            )
            .await;
            // It exists: whatever it says, it did not say "no such resource", and
            // it did not say the method is wrong.
            assert!(
                reply.status != 404 || reply.json()["error"]["what"] != Value::Null,
                "{} {} is not served: {}",
                route.method.as_str(),
                route.pattern,
                reply.text()
            );
            assert_ne!(
                reply.status,
                405,
                "{} {}",
                route.method.as_str(),
                route.pattern
            );
        }
    }
    // And a path outside the table is not served, with any method.
    for method in ["GET", "POST", "PUT", "DELETE"] {
        let reply = request(
            fixture.operator_socket(),
            method,
            "/v1/not-in-the-table",
            Some(&operator),
            None,
        )
        .await;
        assert_eq!(reply.status, 404, "{method}");
    }
    // The table's methods are the routes' methods: each other method is a 405.
    for route in served_to(true).filter(served) {
        let other = if route.method.as_str() == "GET" {
            "DELETE"
        } else {
            "GET"
        };
        let taken = TABLE
            .iter()
            .any(|row| row.pattern == route.pattern && row.method.as_str() == other);
        if !taken {
            let reply = request(
                fixture.operator_socket(),
                other,
                &sample_path(route.pattern),
                Some(&operator),
                None,
            )
            .await;
            assert_eq!(reply.status, 405, "{other} {}", route.pattern);
        }
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_mistyped_field_in_a_submit_is_400_and_nothing_is_submitted() {
    let fixture = Fixture::start(Options::default()).await;
    let token = fixture.agent_token("openclaw").await;
    let room = living_room();
    for stray in [
        json!({ "intent": { "kind": "mute", "value": "on" }, "dry_run": true }),
        json!({ "intent": { "kind": "mute", "value": "on" }, "dryrun": true }),
        json!({ "intent": { "kind": "mute", "value": "on", "extra": 1 } }),
        json!({ "intent": { "kind": "mute", "value": "on" }, "idempotency-key": "x" }),
    ] {
        let reply = post_json(fixture.agent_socket(), &path::submit(&room), &token, &stray).await;
        assert_eq!(reply.status, 400, "{stray}");
        assert_eq!(reply.code().as_deref(), Some("invalid_request"));
    }
    assert_eq!(
        fixture.connector.operate_calls(),
        0,
        "nothing reached the receiver"
    );
    // No operation was made: the first id is still free.
    let reply = get(fixture.agent_socket(), &path::operation(1, None), &token).await;
    assert_eq!(reply.status, 404);
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_body_naming_dry_run_on_the_submit_route_is_400_and_does_not_write() {
    let fixture = Fixture::start(Options::default()).await;
    let token = fixture.operator_token();
    let body = json!({ "intent": { "kind": "mute", "value": "on" }, "dry_run": true });
    let reply = post_json(
        fixture.operator_socket(),
        &path::submit(&living_room()),
        &token,
        &body,
    )
    .await;
    assert_eq!(reply.status, 400);
    assert!(reply.json()["error"]["message"]
        .as_str()
        .unwrap()
        .contains("dry_run"));
    assert_eq!(fixture.connector.operate_calls(), 0);
    fixture.shutdown().await;
}

#[tokio::test]
async fn the_dry_run_route_never_submits() {
    let fixture = Fixture::start(Options::default()).await;
    let token = fixture.agent_token("openclaw").await;
    let room = living_room();
    let before = locked(&fixture.connector.sessions).len();
    for body in [
        mute_on(),
        json!({ "intent": { "kind": "volume", "level": { "half_steps": -50 } } }),
        json!({ "intent": { "kind": "volume", "level": { "half_steps": -30 } } }),
    ] {
        let reply = post_json(fixture.agent_socket(), &path::dry_run(&room), &token, &body).await;
        assert_eq!(reply.status, 200, "{}", reply.text());
    }
    assert_eq!(fixture.connector.operate_calls(), 0);
    assert!(locked(&fixture.connector.sessions).len() >= before);
    // No operation exists, and nothing about it is in the audit log.
    assert_eq!(
        get(fixture.agent_socket(), &path::operation(1, None), &token)
            .await
            .status,
        404
    );
    let text = audit_text(&fixture.audit_directory);
    assert!(
        !text.contains("\"kind\":\"decided\"") || !text.contains("openclaw"),
        "{text}"
    );
    assert!(!text.contains("\"kind\":\"dispatching\""));
    fixture.shutdown().await;
}

#[tokio::test]
async fn put_config_needs_if_match_and_a_stale_one_is_412() {
    let fixture = Fixture::start(Options::default()).await;
    let token = fixture.operator_token();
    let socket = fixture.operator_socket().to_owned();
    let read = get(&socket, &path::config(), &token).await;
    let tag = read.header("etag").unwrap().to_owned();
    let mut config = read.json();
    config["receivers"]["living-room"]["friendly_name"] = "Lounge".into();
    let body = serde_json::to_vec(&config).unwrap();

    let put = |if_match: Option<String>| {
        let socket = socket.clone();
        let token = token.clone();
        let body = body.clone();
        async move {
            let mut head = format!(
                "PUT /v1/config HTTP/1.1\r\nHost: dar\r\nConnection: close\r\nAuthorization: Bearer {token}\r\n\
                 Content-Type: application/json\r\nContent-Length: {}\r\n",
                body.len()
            );
            if let Some(tag) = if_match {
                head.push_str(&format!("If-Match: {tag}\r\n"));
            }
            head.push_str("\r\n");
            let mut bytes = head.into_bytes();
            bytes.extend_from_slice(&body);
            parse_reply(&send_raw(&socket, &bytes).await).unwrap()
        }
    };
    assert_eq!(put(None).await.status, 428);
    assert_eq!(put(Some("\"00\"".into())).await.status, 412);
    assert_eq!(put(Some("*".into())).await.status, 412);
    // Neither changed anything.
    assert_eq!(
        locked(&fixture.config.0).receivers["living-room"].friendly_name,
        None
    );

    let saved = put(Some(tag.clone())).await;
    assert_eq!(saved.status, 200, "{}", saved.text());
    assert_eq!(
        locked(&fixture.config.0).receivers["living-room"]
            .friendly_name
            .as_deref(),
        Some("Lounge")
    );
    let new_tag = saved.header("etag").unwrap();
    assert_ne!(new_tag, tag, "the tag changes with an edit");
    // The old tag is stale now.
    assert_eq!(put(Some(tag)).await.status, 412);
    fixture.shutdown().await;
}

#[tokio::test]
async fn two_puts_with_one_etag_apply_once() {
    let fixture = Fixture::start(Options::default()).await;
    let token = fixture.operator_token();
    let socket = fixture.operator_socket().to_owned();
    let read = get(&socket, &path::config(), &token).await;
    let tag = read.header("etag").unwrap().to_owned();
    let send = |name: &'static str| {
        let (socket, token, tag) = (
            socket.clone(),
            token.clone(),
            read.header("etag").unwrap().to_owned(),
        );
        let mut config = read.json();
        config["receivers"]["living-room"]["friendly_name"] = name.into();
        async move {
            let body = serde_json::to_vec(&config).unwrap();
            let head = format!(
                "PUT /v1/config HTTP/1.1\r\nHost: dar\r\nConnection: close\r\nAuthorization: Bearer {token}\r\n\
                 Content-Type: application/json\r\nContent-Length: {}\r\nIf-Match: {tag}\r\n\r\n",
                body.len()
            );
            let mut bytes = head.into_bytes();
            bytes.extend_from_slice(&body);
            parse_reply(&send_raw(&socket, &bytes).await)
                .unwrap()
                .status
        }
    };
    let (first, second) = tokio::join!(send("First"), send("Second"));
    let mut statuses = [first, second];
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 412], "exactly one wins");
    let _ = tag;
    fixture.shutdown().await;
}

#[tokio::test]
async fn the_etag_changes_with_any_edit_and_not_with_key_order() {
    let fixture = Fixture::start(Options::default()).await;
    let token = fixture.operator_token();
    let socket = fixture.operator_socket().to_owned();
    {
        let mut config = locked(&fixture.config.0);
        config
            .receivers
            .insert("den".into(), ReceiverIdentity::ad_hoc("192.0.2.11"));
    }
    let first = get(&socket, &path::config(), &token)
        .await
        .header("etag")
        .unwrap()
        .to_owned();
    // The same configuration, inserted in another order, has the same tag.
    {
        let mut config = locked(&fixture.config.0);
        let kept = std::mem::take(&mut config.receivers);
        let reversed: BTreeMap<_, _> = kept.into_iter().rev().collect();
        config.receivers = reversed;
    }
    let again = get(&socket, &path::config(), &token)
        .await
        .header("etag")
        .unwrap()
        .to_owned();
    assert_eq!(first, again);
    // Any edit changes it.
    for edit in [
        |c: &mut ConfiguredReceivers| c.current = None,
        |c: &mut ConfiguredReceivers| {
            c.receivers.get_mut("den").unwrap().host = "192.0.2.12".into()
        },
        |c: &mut ConfiguredReceivers| c.receivers.get_mut("den").unwrap().model = Some("X".into()),
        |c: &mut ConfiguredReceivers| {
            c.receivers.remove("den");
        },
    ] {
        let before = get(&socket, &path::config(), &token)
            .await
            .header("etag")
            .unwrap()
            .to_owned();
        edit(&mut locked(&fixture.config.0));
        let after = get(&socket, &path::config(), &token)
            .await
            .header("etag")
            .unwrap()
            .to_owned();
        assert_ne!(before, after);
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn policy_reload_and_audit_paging_work_through_the_server() {
    let fixture = Fixture::start(Options::default()).await;
    let token = fixture.operator_token();
    let socket = fixture.operator_socket().to_owned();
    // A few records: each reload writes one.
    for _ in 0..5 {
        assert_eq!(
            post_json(&socket, &path::policy_reload(), &token, &json!({}))
                .await
                .status,
            200
        );
    }
    let all: AuditPageDto =
        serde_json::from_value(get(&socket, &path::audit(500, None), &token).await.json()).unwrap();
    assert!(all.entries.len() >= 6, "{}", all.entries.len());

    // Walk the log two at a time: no entry twice, none missed, newest first.
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let page: AuditPageDto =
            serde_json::from_value(get(&socket, &path::audit(2, cursor), &token).await.json())
                .unwrap();
        assert!(page.entries.len() <= 2);
        seen.extend(page.entries.iter().map(|entry| entry.seq));
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    let expected: Vec<u64> = all.entries.iter().map(|entry| entry.seq).collect();
    assert_eq!(seen, expected);
    assert!(
        seen.windows(2).all(|pair| pair[0] > pair[1]),
        "newest first"
    );
    // A limit of zero is one; a huge limit is the cap.
    let page: AuditPageDto =
        serde_json::from_value(get(&socket, "/v1/audit?limit=0", &token).await.json()).unwrap();
    assert_eq!(page.entries.len(), 1);
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_token_is_issued_listed_and_revoked_and_its_secret_is_shown_once() {
    let fixture = Fixture::start(Options::default()).await;
    let operator = fixture.operator_token();
    let socket = fixture.operator_socket().to_owned();

    let reply = post_json(
        &socket,
        &path::tokens(),
        &operator,
        &json!({ "label": "claude-code" }),
    )
    .await;
    assert_eq!(reply.status, 201, "{}", reply.text());
    let issued: IssuedTokenDto = serde_json::from_value(reply.json()).unwrap();
    assert!(issued.secret.starts_with("dara_"));
    assert_eq!(issued.token.label, "claude-code");

    // The token works on the Agent endpoint.
    assert_eq!(
        get(fixture.agent_socket(), "/v1/health", &issued.secret)
            .await
            .status,
        200
    );

    // The listing has no secret.
    let reply = get(&socket, &path::tokens(), &operator).await;
    assert!(!reply.text().contains(&issued.secret));
    let listing: TokenListDto = serde_json::from_value(reply.json()).unwrap();
    assert_eq!(listing.tokens.len(), 1);
    assert_eq!(listing.tokens[0], issued.token);

    // One active token for a label; a label that fails the rule.
    let again = post_json(
        &socket,
        &path::tokens(),
        &operator,
        &json!({ "label": "claude-code" }),
    )
    .await;
    assert_eq!(again.status, 400);
    let bad = post_json(
        &socket,
        &path::tokens(),
        &operator,
        &json!({ "label": "Claude-Code" }),
    )
    .await;
    assert_eq!(
        bad.status, 400,
        "a label typed with the wrong case is refused"
    );
    let extra = post_json(
        &socket,
        &path::tokens(),
        &operator,
        &json!({ "label": "x", "admin": true }),
    )
    .await;
    assert_eq!(extra.status, 400);

    // Revoking ends the token at once, and is not an error twice.
    let reply = request(
        &socket,
        "DELETE",
        &path::token(&issued.token.id),
        Some(&operator),
        None,
    )
    .await;
    assert_eq!(reply.status, 200);
    let revoked: TokenDto = serde_json::from_value(reply.json()).unwrap();
    assert!(revoked.revoked_ms.is_some());
    assert_eq!(
        get(fixture.agent_socket(), "/v1/health", &issued.secret)
            .await
            .status,
        401
    );
    let twice = request(
        &socket,
        "DELETE",
        &path::token(&issued.token.id),
        Some(&operator),
        None,
    )
    .await;
    assert_eq!(twice.status, 200);

    // And the label can be issued again.
    let reply = post_json(
        &socket,
        &path::tokens(),
        &operator,
        &json!({ "label": "claude-code" }),
    )
    .await;
    assert_eq!(reply.status, 201);
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_discovery_timeout_over_the_cap_is_clamped_and_not_refused() {
    let fixture = Fixture::start(Options::default()).await;
    let token = fixture.operator_token();
    let socket = fixture.operator_socket().to_owned();
    let asked = |fixture: &Fixture| *locked(&fixture.discovery.1);

    let reply = post_json(
        &socket,
        &path::discover(),
        &token,
        &json!({ "timeout_ms": 86_400_000_u64 }),
    )
    .await;
    assert_eq!(reply.status, 200);
    assert_eq!(asked(&fixture), Some(Duration::from_secs(30)));
    let reply = post_json(
        &socket,
        &path::discover(),
        &token,
        &json!({ "timeout_ms": 250 }),
    )
    .await;
    assert_eq!(reply.status, 200);
    assert_eq!(asked(&fixture), Some(Duration::from_millis(250)));
    // No body at all is the default.
    let reply = request(&socket, "POST", &path::discover(), Some(&token), None).await;
    assert_eq!(reply.status, 200);
    assert_eq!(asked(&fixture), Some(Duration::from_secs(5)));
    let found: DiscoveredListDto = serde_json::from_value(reply.json()).unwrap();
    assert!(found.receivers.is_empty());
    fixture.shutdown().await;

    let _ = (
        DiscoveredReceiver {
            address: ReceiverEndpoint {
                host: String::new(),
                port: 0,
            },
            location: None,
            server: None,
            model: None,
            search_target: None,
            unique_service_name: None,
        },
        ControlError::Forbidden,
    );
}

#[tokio::test]
async fn a_validation_error_names_the_field_and_never_echoes_the_body() {
    let fixture = Fixture::start(Options {
        limits: Limits {
            body_timeout: Duration::from_millis(300),
            ..Limits::default()
        },
        ..Options::default()
    })
    .await;
    let token = fixture.operator_token();
    let socket = fixture.operator_socket().to_owned();
    let room = living_room();
    let secret = "SECRET-TEXT-123";

    let cases: Vec<(String, &str)> = vec![
        (
            format!(r#"{{"intent":{{"kind":"mute","value":"on"}},"surprise":"{secret}"}}"#),
            "surprise",
        ),
        (
            format!(r#"{{"intent":{{"kind":"mute","value":"{secret}"}}}}"#),
            "",
        ),
        (format!(r#"{{"intent":{{"kind":"{secret}"}}}}"#), ""),
        (format!(r#"{{"intent": {secret}"#), ""),
        (
            format!(
                r#"{{"intent":{{"kind":"mute","value":"on"}},"idempotency_key":"{secret}\n"}}"#
            ),
            "",
        ),
        (
            format!(
                r#"{{"intent":{{"kind":"mute","value":"on"}},"{}":1}}"#,
                "x".repeat(10_000)
            ),
            "unknown field",
        ),
    ];
    for (body, names) in cases {
        let reply = request(
            &socket,
            "POST",
            &path::submit(&room),
            Some(&token),
            Some(body.as_bytes()),
        )
        .await;
        assert_eq!(reply.status, 400, "{body}: {}", reply.text());
        let text = reply.text();
        assert!(!text.contains(secret), "{body} was echoed: {text}");
        assert!(
            text.len() < 400,
            "a long field name is cut short: {} bytes",
            text.len()
        );
        if !names.is_empty() {
            assert!(text.contains(names), "{text}");
        }
    }

    // A body that is not JSON by its type.
    let bytes = format!(
        "POST {} HTTP/1.1\r\nHost: dar\r\nConnection: close\r\nAuthorization: Bearer {token}\r\n\
         Content-Type: text/plain\r\nContent-Length: 2\r\n\r\n{{}}",
        path::submit(&room)
    );
    assert_eq!(
        parse_reply(&send_raw(&socket, bytes.as_bytes()).await)
            .unwrap()
            .status,
        415
    );

    // A body that never finishes arriving.
    let bytes = format!(
        "POST {} HTTP/1.1\r\nHost: dar\r\nConnection: close\r\nAuthorization: Bearer {token}\r\n\
         Content-Type: application/json\r\nContent-Length: 50\r\n\r\n{{\"intent\"",
        path::submit(&room)
    );
    let reply = parse_reply(&send_raw(&socket, bytes.as_bytes()).await).unwrap();
    assert_eq!(reply.status, 408, "a_slow_body_is_408");
    assert_eq!(reply.code().as_deref(), Some("request_timeout"));
    fixture.shutdown().await;
}
