//! The Operator's resources and the requests: configuration, discovery, the
//! inspection reads, the policy, the audit log, tokens, and dry runs.

mod common;

use common::*;
use denon_avr_api_contract::admin::{
    AdHocRequest, AuditPageDto, ConfigDto, ConfigRequest, DiscoverRequest, DiscoveredDto,
    DiscoveredListDto, HttpInformationDto, IssuedTokenDto, PolicyDto, QuickSelectNamesDto,
    ReadinessDto, TokenDto, TokenIssueRequest, TokenListDto,
};
use denon_avr_api_contract::requests::{
    AgentDryRun, DryRunRequest, OperatorDryRun, OperatorDryRunRequest, SubmitRequest,
};
use denon_avr_application::{
    AgentLabel, AuditCursor, AuditDecision, AuditEntry, AuditEvent, AuditPage, AuditRecord, DryRun,
    DryRunDecision, EndpointKind, IdempotencyKey, IssuedToken, OperationSubmission, PolicyDigest,
    PolicyView, Principal, Readiness, RefusalReason, TokenId, TokenRecord, TokenSecret,
};
use denon_avr_domain::{
    AudioInformation, AudysseyInformation, ChannelSlot, ChannelSlotState, ConfiguredReceivers,
    CoreField, DiscoveredReceiver, DispatchCertainty, FieldError, FieldErrorKind, FieldStatus,
    Freshness, HttpInformationSnapshot, MuteState, OperationId, QuickSelectName,
    QuickSelectNameObservation, QuickSelectNameResponseEvidence, ReceiverEndpoint,
    ReceiverIdentity, ReceiverIntent, SoundModeFavorite, VideoInformation, WallTime,
};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, UNIX_EPOCH};

fn digest() -> PolicyDigest {
    PolicyDigest::from_bytes([0xab; 32])
}

fn via_json<T: serde::Serialize + serde::de::DeserializeOwned>(value: &T) -> T {
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

#[test]
fn a_submission_carries_its_intent_and_its_key_and_refuses_a_stray_field() {
    let submission = OperationSubmission::new(ReceiverIntent::Mute(MuteState::On))
        .with_idempotency_key(IdempotencyKey::new("retry-1").unwrap());
    let request = via_json(&SubmitRequest::from(&submission));
    assert_eq!(request.into_submission().unwrap(), submission);

    let without = SubmitRequest::from(&OperationSubmission::new(ReceiverIntent::Mute(
        MuteState::Off,
    )));
    assert_eq!(without.idempotency_key, None);
    assert!(!serde_json::to_string(&without)
        .unwrap()
        .contains("idempotency"));

    for bad in [
        r#"{"intent":{"kind":"mute","value":"on"},"dry_run":true}"#,
        r#"{"intent":{"kind":"mute","value":"on"},"idempotencykey":"x"}"#,
        r#"{"intent":{"kind":"mute","value":"on"},"idempotency_key":"new\nline"}"#,
    ] {
        let result: Result<_, String> = serde_json::from_str::<SubmitRequest>(bad)
            .map_err(|why| why.to_string())
            .and_then(SubmitRequest::into_submission);
        assert!(result.is_err(), "{bad}");
    }
}

#[test]
fn a_dry_run_for_an_agent_has_no_as_agent_and_no_rule_ids() {
    let intent = r#"{"kind":"mute","value":"on"}"#;
    // An agent's request has no `as_agent`, so it is refused by name.
    let agent = format!(r#"{{"intent":{intent},"as_agent":"openclaw"}}"#);
    let why = serde_json::from_str::<DryRunRequest>(&agent)
        .unwrap_err()
        .to_string();
    assert!(why.contains("as_agent"), "{why}");
    // The Operator's may name one.
    let operator: OperatorDryRunRequest = serde_json::from_str(&agent).unwrap();
    assert_eq!(
        operator.agent().unwrap(),
        Some(AgentLabel::new("openclaw").unwrap())
    );
    assert!(serde_json::from_str::<OperatorDryRunRequest>(&format!(
        r#"{{"intent":{intent},"as_agent":"new\nline"}}"#
    ))
    .unwrap()
    .agent()
    .is_err());

    // The answer an agent gets has no rule ids, whatever the port handed over.
    let decided = DryRun {
        decision: DryRunDecision::Deny {
            reasons: vec!["the volume target is above the limit".into()],
            rules: vec!["volume-hard-limit".into()],
        },
        policy: Some(digest()),
    };
    let agent_json = serde_json::to_string(&AgentDryRun::from(&decided)).unwrap();
    assert!(!agent_json.contains("volume-hard-limit") && !agent_json.contains("rules"));
    let operator_json = serde_json::to_string(&OperatorDryRun::from(&decided)).unwrap();
    assert!(operator_json.contains("volume-hard-limit"));

    // And every decision reads back.
    for decision in [
        DryRunDecision::Allow,
        DryRunDecision::RequireApproval {
            reasons: vec!["r".into()],
            rules: vec!["a".into(), "b".into()],
        },
        DryRunDecision::Deny {
            reasons: vec![],
            rules: vec!["a".into()],
        },
        DryRunDecision::Unavailable {
            reason: "policy unavailable".into(),
        },
    ] {
        let dry = DryRun {
            decision,
            policy: None,
        };
        assert_eq!(
            via_json(&OperatorDryRun::from(&dry))
                .into_dry_run()
                .unwrap(),
            dry
        );
        // An agent's copy reads back without the rule ids.
        let back = via_json(&AgentDryRun::from(&dry)).into_dry_run().unwrap();
        match (&dry.decision, &back.decision) {
            (
                DryRunDecision::RequireApproval { reasons, .. },
                DryRunDecision::RequireApproval {
                    reasons: got,
                    rules,
                },
            ) => {
                assert_eq!((reasons, rules.len()), (got, 0));
            }
            (
                DryRunDecision::Deny { reasons, .. },
                DryRunDecision::Deny {
                    reasons: got,
                    rules,
                },
            ) => {
                assert_eq!((reasons, rules.len()), (got, 0));
            }
            (a, b) => assert_eq!(a, b),
        }
    }
    let mut bad = serde_json::to_value(OperatorDryRun::from(&decided)).unwrap();
    bad["policy_digest"] = "not hex".into();
    assert!(serde_json::from_value::<OperatorDryRun>(bad)
        .unwrap()
        .into_dry_run()
        .is_err());
}

fn configuration() -> ConfiguredReceivers {
    ConfiguredReceivers {
        current: Some("living-room".into()),
        receivers: BTreeMap::from([
            (
                "living-room".into(),
                ReceiverIdentity {
                    host: "192.0.2.10".into(),
                    model: Some("AVR-X3800H".into()),
                    friendly_name: Some("Living Room".into()),
                },
            ),
            ("den".into(), ReceiverIdentity::ad_hoc("192.0.2.11")),
        ]),
        sound_mode_favorites: BTreeMap::from([(
            "living-room".into(),
            BTreeSet::from([
                SoundModeFavorite::new("DOLBY ATMOS").unwrap(),
                SoundModeFavorite::new("STEREO").unwrap(),
            ]),
        )]),
    }
}

#[test]
fn config_round_trips_with_favorites_and_current() {
    let config = configuration();
    let back: ConfiguredReceivers = via_json(&ConfigDto::from(&config)).try_into().unwrap();
    assert_eq!(back, config);
    let empty: ConfiguredReceivers = via_json(&ConfigDto::from(&ConfiguredReceivers::default()))
        .try_into()
        .unwrap();
    assert_eq!(empty, ConfiguredReceivers::default());

    // The request is the same shape, and strict.
    let request: ConfiguredReceivers = via_json(&ConfigRequest::from(&config)).try_into().unwrap();
    assert_eq!(request, config);
    let mut stray = serde_json::to_value(ConfigRequest::from(&config)).unwrap();
    stray["receivers"]["den"]["adress"] = "x".into();
    let why = serde_json::from_value::<ConfigRequest>(stray)
        .unwrap_err()
        .to_string();
    assert!(why.contains("adress"), "{why}");
    // The answer is lenient.
    let mut grown = serde_json::to_value(ConfigDto::from(&config)).unwrap();
    grown["receivers"]["den"]["added_later"] = true.into();
    assert!(serde_json::from_value::<ConfigDto>(grown).is_ok());

    // A favorite that is not a name is refused.
    let mut bad = ConfigDto::from(&config);
    bad.sound_mode_favorites
        .insert("den".into(), vec!["  ".into()]);
    assert!(ConfiguredReceivers::try_from(bad).is_err());
}

#[test]
fn policy_and_discovery_views_round_trip() {
    let view = PolicyView {
        digest: Some(digest()),
        loaded_at: Some(WallTime(1_700_000_000_000)),
        text: Some("unclassified: require_approval\n".into()),
        error: None,
    };
    assert_eq!(
        PolicyView::try_from(via_json(&PolicyDto::from(&view))).unwrap(),
        view
    );
    let failed = PolicyView {
        digest: None,
        loaded_at: None,
        text: None,
        error: Some("line 3: unknown key".into()),
    };
    assert_eq!(
        PolicyView::try_from(via_json(&PolicyDto::from(&failed))).unwrap(),
        failed
    );
    let mut bad = PolicyDto::from(&view);
    bad.digest = Some("xyz".into());
    assert!(PolicyView::try_from(bad).is_err());

    let found = DiscoveredReceiver {
        address: ReceiverEndpoint {
            host: "192.0.2.10".into(),
            port: 80,
        },
        location: Some("http://192.0.2.10:8080/desc.xml".into()),
        server: Some("Denon/1.0".into()),
        model: Some("AVR-X3800H".into()),
        search_target: Some("urn:schemas-denon-com:device:AiosDevice:1".into()),
        unique_service_name: Some("uuid:abc".into()),
    };
    assert_eq!(
        DiscoveredReceiver::from(via_json(&DiscoveredDto::from(&found))),
        found
    );
    let bare = DiscoveredReceiver {
        address: ReceiverEndpoint {
            host: "h".into(),
            port: 1,
        },
        location: None,
        server: None,
        model: None,
        search_target: None,
        unique_service_name: None,
    };
    assert_eq!(
        DiscoveredReceiver::from(via_json(&DiscoveredDto::from(&bare))),
        bare
    );

    assert_eq!(
        via_json(&DiscoverRequest::default()),
        DiscoverRequest::default()
    );
    assert!(serde_json::from_str::<DiscoverRequest>(r#"{"timeout":5}"#).is_err());

    let identity = ReceiverIdentity {
        host: "192.0.2.50".into(),
        model: None,
        friendly_name: Some("Den".into()),
    };
    assert_eq!(
        ReceiverIdentity::from(via_json(&AdHocRequest::from(&identity))),
        identity
    );
    assert!(serde_json::from_str::<AdHocRequest>(r#"{"host":"h","port":1}"#).is_err());

    let ready = Readiness {
        ready: true,
        degraded: true,
        detail: "volume could not be read".into(),
    };
    assert_eq!(
        Readiness::from(via_json(&ReadinessDto::from(&ready))),
        ready
    );
}

fn quick_select() -> QuickSelectNameObservation {
    QuickSelectNameObservation {
        names: [
            Some(QuickSelectName::new("Movies").unwrap()),
            None,
            Some(QuickSelectName::new("Music").unwrap()),
            None,
        ],
        sources: [Some("BD".into()), None, Some("TUNER".into()), None],
        freshness: Freshness::Live,
        generation: 3,
        observed_at: Some(UNIX_EPOCH + Duration::from_millis(1_700_000_000_000)),
        error: Some("one slot did not answer".into()),
        raw_response: "<names/>".into(),
        response_evidence: QuickSelectNameResponseEvidence::Partial,
    }
}

fn http_information() -> HttpInformationSnapshot {
    let text = |value: &str| FieldStatus::Value(value.to_owned());
    HttpInformationSnapshot {
        input_slots: FieldStatus::Value(vec![
            ChannelSlot {
                label: "FL".into(),
                state: ChannelSlotState::Active,
            },
            ChannelSlot {
                label: "FR".into(),
                state: ChannelSlotState::Available,
            },
        ]),
        output_slots: FieldStatus::Unavailable(FieldError {
            kind: FieldErrorKind::Timeout,
            message: "no answer from 192.0.2.10".into(),
        }),
        audio: AudioInformation {
            input_mode: text("Auto"),
            output: text("FL FR"),
            signal: text("PCM"),
            sound: text("Stereo"),
            sample_rate: FieldStatus::Unavailable(FieldError {
                kind: FieldErrorKind::Unsupported,
                message: "not supported".into(),
            }),
        },
        video: VideoInformation {
            monitor: text("1"),
            hdmi_input: text("4K"),
            hdmi_output: text("4K"),
        },
        audyssey: AudysseyInformation {
            multeq: text("Reference"),
            dynamic_eq: text("Off"),
            dynamic_volume: text("Off"),
        },
        freshness: Freshness::Partial,
        generation: 9,
        observed_at: Some(UNIX_EPOCH + Duration::from_millis(1_700_000_000_500)),
        error: None,
    }
}

#[test]
fn the_inspection_reads_round_trip() {
    let names = quick_select();
    assert_eq!(
        QuickSelectNameObservation::try_from(via_json(&QuickSelectNamesDto::from(&names))).unwrap(),
        names
    );
    let mut short = QuickSelectNamesDto::from(&names);
    short.names.pop();
    assert!(QuickSelectNameObservation::try_from(short).is_err());
    let mut long_name = QuickSelectNamesDto::from(&names);
    long_name.names[1] = Some("a name well past sixteen characters".into());
    assert!(QuickSelectNameObservation::try_from(long_name).is_err());

    let info = http_information();
    assert_eq!(
        HttpInformationSnapshot::from(via_json(&HttpInformationDto::from(&info))),
        info
    );
    let default = HttpInformationSnapshot::default();
    assert_eq!(
        HttpInformationSnapshot::from(via_json(&HttpInformationDto::from(&default))),
        default
    );
}

fn every_event() -> Vec<AuditRecord> {
    let record = |event: AuditEvent| AuditRecord {
        schema: 1,
        run: WallTime(1_000),
        at: WallTime(2_000),
        operation: Some(OperationId(4)),
        principal: Some(Principal::Agent(AgentLabel::new("openclaw").unwrap())),
        receiver: Some(room()),
        event,
    };
    let id = TokenId::new("t-0000002a").unwrap();
    let label = AgentLabel::new("claude-code").unwrap();
    vec![
        record(AuditEvent::Decided {
            intent: "volume -35.0 dB".into(),
            decision: AuditDecision::RequireApproval,
            reasons: vec!["above the limit".into()],
            rules: vec!["volume-ceiling".into()],
            baseline: vec![CoreField::Volume, CoreField::Mute],
            policy: Some(digest()),
        }),
        record(AuditEvent::Decided {
            intent: "mute on".into(),
            decision: AuditDecision::Allow,
            reasons: vec![],
            rules: vec![],
            baseline: vec![],
            policy: None,
        }),
        record(AuditEvent::Dispatching {
            intent: "volume -35.0 dB".into(),
            before: Some(-72),
            target: Some(-70),
            precondition_fields: vec![CoreField::Volume],
        }),
        record(AuditEvent::Finished {
            status: "completed".into(),
            dispatch: DispatchCertainty::CompleteWrite,
            confirmed: true,
            reason: None,
        }),
        record(AuditEvent::Finished {
            status: "rejected".into(),
            dispatch: DispatchCertainty::NotDispatched,
            confirmed: false,
            reason: Some("policy unavailable".into()),
        }),
        record(AuditEvent::PolicyLoaded { digest: digest() }),
        record(AuditEvent::PolicyLoadFailed {
            error: "line 3: unknown key".into(),
        }),
        AuditRecord {
            operation: None,
            principal: None,
            receiver: None,
            ..record(AuditEvent::AccessRefused {
                endpoint: EndpointKind::Agent,
                reason: RefusalReason::NoCredential,
                peer_uid: Some(502),
                resource: Some("/v1/policy".into()),
                suppressed: 17,
            })
        },
        AuditRecord {
            principal: Some(Principal::Operator),
            operation: None,
            receiver: None,
            ..record(AuditEvent::TokenIssued {
                id: id.clone(),
                label: label.clone(),
            })
        },
        AuditRecord {
            principal: Some(Principal::Operator),
            operation: None,
            receiver: None,
            ..record(AuditEvent::TokenRevoked { id, label })
        },
    ]
}

#[test]
fn audit_pages_round_trip_with_cursors_and_every_event_kind() {
    let entries: Vec<AuditEntry> = every_event()
        .into_iter()
        .enumerate()
        .rev()
        .map(|(index, record)| AuditEntry {
            seq: index as u64 + 1,
            record,
        })
        .collect();
    assert_eq!(entries.len(), 10);
    for next in [None, Some(AuditCursor::from_seq(41))] {
        let page = AuditPage {
            entries: entries.clone(),
            next,
        };
        let back = AuditPage::try_from(via_json(&AuditPageDto::from(&page))).unwrap();
        assert_eq!(back, page);
    }
    let empty = AuditPage {
        entries: vec![],
        next: None,
    };
    assert_eq!(
        AuditPage::try_from(via_json(&AuditPageDto::from(&empty))).unwrap(),
        empty
    );

    // An entry naming a refusal reason this code does not know is not guessed.
    let mut value = serde_json::to_value(AuditPageDto::from(&AuditPage {
        entries,
        next: None,
    }))
    .unwrap();
    let refused = value["entries"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["event"]["kind"] == "access_refused")
        .unwrap();
    refused["event"]["reason"] = "from_the_future".into();
    assert!(AuditPage::try_from(serde_json::from_value::<AuditPageDto>(value).unwrap()).is_err());
}

#[test]
fn a_token_listing_has_no_secret_or_digest_field() {
    let record = TokenRecord {
        id: TokenId::new("t-0000002a").unwrap(),
        label: AgentLabel::new("claude-code").unwrap(),
        created: WallTime(1_700_000_000_000),
        revoked: Some(WallTime(1_700_000_100_000)),
    };
    let json = serde_json::to_value(TokenDto::from(&record)).unwrap();
    let found = keys(&json);
    for forbidden in ["secret", "token", "digest", "hash"] {
        assert!(!found.contains(forbidden), "{forbidden}");
    }
    assert_eq!(
        TokenRecord::try_from(via_json(&TokenDto::from(&record))).unwrap(),
        record
    );
    let active = TokenRecord {
        revoked: None,
        ..record
    };
    assert_eq!(
        TokenRecord::try_from(via_json(&TokenDto::from(&active))).unwrap(),
        active
    );
}

#[test]
fn only_the_issue_response_holds_a_secret() {
    let issued = IssuedToken {
        record: TokenRecord {
            id: TokenId::new("t-0000002a").unwrap(),
            label: AgentLabel::new("claude-code").unwrap(),
            created: WallTime(5),
            revoked: None,
        },
        secret: TokenSecret::new("dara_the-one-and-only-copy"),
    };
    let dto = IssuedTokenDto::from(&issued);
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("dara_the-one-and-only-copy"));
    let back = IssuedToken::try_from(via_json(&dto)).unwrap();
    assert_eq!(back.record, issued.record);
    assert_eq!(back.secret.expose(), issued.secret.expose());
    // The record inside it, listed later, has no secret.
    assert!(!serde_json::to_string(&TokenDto::from(&issued.record))
        .unwrap()
        .contains("dara_"));

    let request = TokenIssueRequest {
        label: "claude-code".into(),
    };
    assert_eq!(via_json(&request), request);
    assert!(serde_json::from_str::<TokenIssueRequest>(r#"{"label":"a","admin":true}"#).is_err());
}

#[test]
fn golden_json_for_the_requests_and_the_operator_resources_is_unchanged() {
    golden(
        "submit_request.json",
        &pretty(&SubmitRequest::from(
            &OperationSubmission::new(ReceiverIntent::Mute(MuteState::On))
                .with_idempotency_key(IdempotencyKey::new("retry-1").unwrap()),
        )),
    );
    let dry = DryRun {
        decision: DryRunDecision::RequireApproval {
            reasons: vec!["the volume target -25.0 dB is above the limit -30.0 dB".into()],
            rules: vec!["volume-ceiling".into()],
        },
        policy: Some(digest()),
    };
    golden("dry_run_agent.json", &pretty(&AgentDryRun::from(&dry)));
    golden(
        "dry_run_operator.json",
        &pretty(&OperatorDryRun::from(&dry)),
    );
    golden("config.json", &pretty(&ConfigDto::from(&configuration())));
    golden(
        "policy.json",
        &pretty(&PolicyDto::from(&PolicyView {
            digest: Some(digest()),
            loaded_at: Some(WallTime(1_700_000_000_000)),
            text: Some("unclassified: require_approval\n".into()),
            error: None,
        })),
    );
    golden(
        "audit_page.json",
        &pretty(&AuditPageDto::from(&AuditPage {
            entries: every_event()
                .into_iter()
                .enumerate()
                .rev()
                .map(|(index, record)| AuditEntry {
                    seq: index as u64 + 1,
                    record,
                })
                .collect(),
            next: Some(AuditCursor::from_seq(0)),
        })),
    );
    golden(
        "token_issued.json",
        &pretty(&IssuedTokenDto::from(&IssuedToken {
            record: TokenRecord {
                id: TokenId::new("t-0000002a").unwrap(),
                label: AgentLabel::new("claude-code").unwrap(),
                created: WallTime(1_700_000_000_000),
                revoked: None,
            },
            secret: TokenSecret::new("dara_EXAMPLE-ONLY-NOT-A-REAL-TOKEN-0123456789abcdef"),
        })),
    );
    golden(
        "quick_select_names.json",
        &pretty(&QuickSelectNamesDto::from(&quick_select())),
    );
    golden(
        "http_information.json",
        &pretty(&HttpInformationDto::from(&http_information())),
    );
    golden(
        "token_list.json",
        &pretty(&TokenListDto {
            tokens: vec![TokenDto {
                id: "t-0000002a".into(),
                label: "claude-code".into(),
                created_ms: 1_700_000_000_000,
                revoked_ms: None,
            }],
        }),
    );
    golden(
        "discovered_list.json",
        &pretty(&DiscoveredListDto {
            receivers: vec![DiscoveredDto::from(&DiscoveredReceiver {
                address: ReceiverEndpoint {
                    host: "192.0.2.10".into(),
                    port: 80,
                },
                location: None,
                server: None,
                model: Some("AVR-X3800H".into()),
                search_target: None,
                unique_service_name: None,
            })],
        }),
    );
    golden(
        "receivers_list.json",
        &pretty(&denon_avr_api_contract::ReceiversDto {
            receivers: vec![denon_avr_api_contract::ReceiverSummaryDto {
                id: "living-room".into(),
                model: Some("AVR-X3800H".into()),
                capabilities: denon_avr_api_contract::CapabilitiesDto {
                    writable: true,
                    zone2_power: true,
                    source_catalog_read: true,
                    inputs: vec!["BD".into()],
                    surround_modes: vec!["DIRECT".into()],
                },
                connection: denon_avr_api_contract::ConnectionDto::Released,
            }],
        }),
    );
}
