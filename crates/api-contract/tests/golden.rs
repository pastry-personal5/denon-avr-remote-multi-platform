//! The contract as a reviewer reads it: golden files for each wire type and the
//! route table, the routing facts a client and a server rely on, and the rule that
//! a request is strict and a response lenient.

mod common;

use common::*;
use denon_avr_api_contract::paths::request;
use denon_avr_api_contract::routes::{served_to, TABLE};
use denon_avr_api_contract::{
    encode_segment, AgentEndpointDto, AgentSourcesView, AgentStateView, ApiError, Audience,
    CapabilitiesDto, ConnectionDto, EndpointPaths, HealthDto, IntentDto, OperationDto,
    OperatorSourcesView, OperatorStateView, ReceiverSummaryDto, ServerHealthDto, SoundModeDto,
    TokenStoreDto, VolumeDto,
};
use denon_avr_application::{
    ApprovalHealth, AuditHealth, ControlError, OperationSnapshot, OperationStatus, PolicyHealth,
    ServiceHealth,
};
use denon_avr_domain::{DispatchCertainty, MuteState, OperationId, ReceiverIntent};
use std::path::Path;
use std::time::Duration;

#[test]
fn the_route_table_lists_exactly_the_documented_resources() {
    let listing: Vec<String> = TABLE
        .iter()
        .map(|route| {
            format!(
                "{} {} {}",
                route.method.as_str(),
                route.pattern,
                match route.audience {
                    Audience::Both => "both",
                    Audience::Operator => "operator",
                }
            )
        })
        .collect();
    golden("routes.txt", &listing.join("\n"));
}

#[test]
fn no_two_rows_share_a_method_and_pattern_or_an_id() {
    for (index, route) in TABLE.iter().enumerate() {
        for other in &TABLE[index + 1..] {
            assert!(
                (route.method, route.pattern) != (other.method, other.pattern),
                "{} {}",
                route.method.as_str(),
                route.pattern
            );
            assert_ne!(route.id, other.id, "{}", route.pattern);
        }
    }
}

#[test]
fn the_agent_endpoint_is_served_only_the_shared_rows() {
    let agent: Vec<_> = served_to(false).collect();
    assert!(agent.iter().all(|route| route.audience == Audience::Both));
    assert_eq!(agent.len(), 10);
    assert_eq!(served_to(true).count(), TABLE.len());
    for operator_only in ["/v1/policy", "/v1/audit", "/v1/tokens", "/v1/config"] {
        assert!(agent
            .iter()
            .all(|route| !route.pattern.starts_with(operator_only)));
    }
}

/// Whether `path` fits `pattern`, a `{id}` standing for one whole segment.
fn fits(pattern: &str, path: &str) -> bool {
    let path = path.split('?').next().unwrap();
    let (p, q): (Vec<_>, Vec<_>) = (pattern.split('/').collect(), path.split('/').collect());
    p.len() == q.len()
        && p.iter()
            .zip(&q)
            .all(|(p, q)| (p.starts_with('{') && !q.is_empty()) || p == q)
}

#[test]
fn every_path_a_client_builds_fits_a_row_of_the_table() {
    let awkward = denon_avr_domain::ReceiverId::new("a/b c?d%e#f é").unwrap();
    let ad_hoc = denon_avr_domain::ReceiverId::ad_hoc("192.0.2.50").unwrap();
    for receiver in [&awkward, &ad_hoc, &common::room()] {
        let paths = [
            request::state(receiver),
            request::sources(receiver),
            request::state_events(receiver),
            request::submit(receiver),
            request::dry_run(receiver),
            request::quick_select_names(receiver),
            request::http_information(receiver),
            request::refresh(receiver),
        ];
        for path in paths {
            assert!(path.starts_with("/v1/"), "{path}");
            assert!(!path.contains(' ') && !path.contains('#'), "{path}");
            let fitting = TABLE
                .iter()
                .filter(|route| fits(route.pattern, &path))
                .count();
            assert!(fitting >= 1, "{path} fits no row");
        }
    }
    let others = [
        request::health(),
        request::receivers(),
        request::operation_events(),
        request::operation(7, None),
        request::operation(7, Some(30_000)),
        request::cancel(7),
        request::discover(),
        request::ad_hoc(),
        request::config(),
        request::policy(),
        request::policy_reload(),
        request::audit(50, None),
        request::audit(50, Some(12)),
        request::tokens(),
        request::token("t-0000002a"),
    ];
    for path in others {
        assert!(
            TABLE.iter().any(|route| fits(route.pattern, &path)),
            "{path}"
        );
    }
}

#[test]
fn an_encoded_id_stays_in_its_segment() {
    let id = denon_avr_domain::ReceiverId::new("a/b").unwrap();
    let path = request::state(&id);
    assert_eq!(path, "/v1/receivers/a%2Fb/state");
    assert_eq!(path.split('/').count(), 5, "{path}");
}

#[test]
fn encode_segment_leaves_unreserved_characters_and_encodes_the_rest() {
    assert_eq!(encode_segment("living-room_2.~"), "living-room_2.~");
    assert_eq!(encode_segment("a b"), "a%20b");
    assert_eq!(encode_segment("a/b"), "a%2Fb");
    assert_eq!(encode_segment("100%"), "100%25");
    assert_eq!(encode_segment("a?b#c"), "a%3Fb%23c");
    assert_eq!(encode_segment("adhoc:192.0.2.50"), "adhoc%3A192.0.2.50");
    assert_eq!(encode_segment("é"), "%C3%A9");
    assert_eq!(encode_segment(""), "");
    // Every encoded byte decodes to what it was.
    let original = "Büro/2 & ½?";
    let encoded = encode_segment(original);
    assert!(encoded
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-._~%".contains(&b)));
    let mut decoded = Vec::new();
    let bytes = encoded.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            decoded.push(u8::from_str_radix(&encoded[index + 1..index + 3], 16).unwrap());
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    assert_eq!(String::from_utf8(decoded).unwrap(), original);
}

#[test]
fn endpoint_paths_are_the_servers_own_subdirectories() {
    let paths = EndpointPaths::under(Path::new("/data"));
    assert_eq!(paths.run_directory, Path::new("/data/run"));
    assert_eq!(paths.operator_socket, Path::new("/data/run/operator.sock"));
    assert_eq!(paths.lock_file, Path::new("/data/run/server.lock"));
    assert_eq!(paths.credentials_directory, Path::new("/data/credentials"));
    assert_eq!(
        paths.operator_token,
        Path::new("/data/credentials/operator.token")
    );
    assert_eq!(
        paths.agent_tokens,
        Path::new("/data/credentials/agent-tokens.json")
    );
}

#[test]
fn golden_json_for_each_type_is_unchanged() {
    golden(
        "intent_volume.json",
        &pretty(&IntentDto::Volume {
            level: VolumeDto::HalfSteps(-71),
        }),
    );
    golden(
        "intent_volume_minimum.json",
        &pretty(&IntentDto::Volume {
            level: VolumeDto::Minimum,
        }),
    );
    golden(
        "intent_sound_mode_select.json",
        &pretty(&IntentDto::SoundMode {
            mode: SoundModeDto::Select("DOLBY ATMOS".into()),
        }),
    );
    let snapshot = OperationSnapshot {
        id: OperationId(7),
        receiver: room(),
        intent: ReceiverIntent::Mute(MuteState::On),
        status: OperationStatus::Completed,
        dispatch: DispatchCertainty::CompleteWrite,
        confirmed: true,
        reason: None,
        observation: Some("mute on".into()),
    };
    golden(
        "operation_completed.json",
        &pretty(&OperationDto::from(&snapshot)),
    );
    let mut late = snapshot.clone();
    late.status = OperationStatus::InSession;
    late.confirmed = false;
    golden(
        "error_too_late.json",
        &pretty(&ApiError::from(&ControlError::TooLate(Box::new(late))).body),
    );
    golden(
        "error_rate_limited.json",
        &pretty(
            &ApiError::from(&ControlError::RateLimited {
                retry_after: Some(Duration::from_millis(1_500)),
            })
            .body,
        ),
    );
    golden(
        "error_unauthenticated.json",
        &pretty(&ApiError::unauthenticated().body),
    );
    golden(
        "receiver_summary.json",
        &pretty(&ReceiverSummaryDto {
            id: "living-room".into(),
            model: Some("AVR-X3800H".into()),
            capabilities: CapabilitiesDto {
                writable: true,
                zone2_power: true,
                source_catalog_read: true,
                inputs: vec!["BD".into(), "GAME".into()],
                surround_modes: vec!["DIRECT".into(), "STEREO".into()],
            },
            connection: ConnectionDto::Connected,
        }),
    );
    let service = ServiceHealth {
        policy: PolicyHealth::Active,
        audit: AuditHealth::Ok,
        ledger_ready: true,
        approval: ApprovalHealth::Unavailable,
    };
    golden(
        "health_agent.json",
        &pretty(&HealthDto::new(&service, None)),
    );
    golden(
        "health_operator.json",
        &pretty(&HealthDto::new(
            &service,
            Some(ServerHealthDto {
                agent_endpoint: AgentEndpointDto::Off {
                    reason: "no agent endpoint is configured".into(),
                },
                token_store: TokenStoreDto::Ok,
            }),
        )),
    );
    let state = sample_state();
    golden("state_agent.json", &pretty(&AgentStateView::from(&state)));
    golden(
        "state_operator.json",
        &pretty(&OperatorStateView::from(&state)),
    );
    let catalog = catalog_with_text();
    golden(
        "sources_agent.json",
        &pretty(&AgentSourcesView::from(&catalog)),
    );
    golden(
        "sources_operator.json",
        &pretty(&OperatorSourcesView::from(&catalog)),
    );
}

#[test]
fn a_response_with_an_extra_field_is_read_and_a_request_with_one_is_refused_by_name() {
    // A response: the contract grows by adding fields, and a reader skips them.
    let mut response = serde_json::to_value(OperationDto::from(&OperationSnapshot {
        id: OperationId(1),
        receiver: room(),
        intent: ReceiverIntent::Mute(MuteState::On),
        status: OperationStatus::Completed,
        dispatch: DispatchCertainty::CompleteWrite,
        confirmed: true,
        reason: None,
        observation: None,
    }))
    .unwrap();
    response["a_field_from_the_future"] = serde_json::json!({ "nested": [1, 2] });
    assert!(serde_json::from_value::<OperationDto>(response).is_ok());

    let mut state = serde_json::to_value(OperatorStateView::from(&sample_state())).unwrap();
    state["main_zone"]["volume"]["a_field_from_the_future"] = serde_json::json!(true);
    state["something_new"] = serde_json::json!("x");
    assert!(serde_json::from_value::<OperatorStateView>(state).is_ok());

    // A request: a mistyped field is refused, and the message names it.
    for (json, field) in [
        (r#"{"kind":"mute","value":"on","dry_run":true}"#, "dry_run"),
        (
            r#"{"kind":"volume","level":"minimum","dryrun":true}"#,
            "dryrun",
        ),
        (r#"{"kind":"source","id":"BD","idd":"x"}"#, "idd"),
    ] {
        let why = serde_json::from_str::<IntentDto>(json)
            .unwrap_err()
            .to_string();
        assert!(why.contains(field), "{json}: {why}");
    }
}

#[test]
fn a_response_naming_an_unknown_status_is_not_guessed() {
    let mut value = serde_json::to_value(OperationDto::from(&OperationSnapshot {
        id: OperationId(1),
        receiver: room(),
        intent: ReceiverIntent::Mute(MuteState::On),
        status: OperationStatus::Completed,
        dispatch: DispatchCertainty::CompleteWrite,
        confirmed: true,
        reason: None,
        observation: None,
    }))
    .unwrap();
    value["status"] = "vaporized".into();
    assert!(serde_json::from_value::<OperationDto>(value.clone()).is_err());
    value["status"] = "completed".into();
    value["dispatch"] = "teleported".into();
    assert!(serde_json::from_value::<OperationDto>(value).is_err());
}
