//! What goes onto the wire comes back as the same thing: errors, states,
//! catalogs, intents, and operations.

mod common;

use common::*;
use denon_avr_api_contract::error::{kind_name, ALL_KINDS};
use denon_avr_api_contract::{
    AgentSourcesView, AgentStateView, ApiError, Diagnostics, ErrorBody, FieldText, IntentDto,
    OperationDto, OperatorSourcesView, OperatorStateView, StateView, VolumeDto,
};
use denon_avr_application::ports::{OperationError, OperationErrorKind};
use denon_avr_application::{ControlError, OperationSnapshot, OperationStatus};
use denon_avr_domain::{
    CatalogResponseEvidence, DispatchCertainty, FieldBaseline, Freshness, MasterVolume, MuteState,
    OperationId, ReceiverFieldValidity, ReceiverIntent, SoundModeIntent, SourceCatalog,
    SourceCatalogObservation, SourceEntry, SourceId, SourceVisibility, SystemPower, ZonePower,
};
use std::time::{Duration, UNIX_EPOCH};

fn snapshot(status: OperationStatus) -> OperationSnapshot {
    OperationSnapshot {
        id: OperationId(7),
        receiver: room(),
        intent: ReceiverIntent::Mute(MuteState::On),
        status,
        dispatch: DispatchCertainty::NotDispatched,
        confirmed: false,
        reason: Some("a reason".into()),
        observation: None,
    }
}

/// An error through the wire: serialized, parsed, and read back by a client.
fn across_the_wire(error: &ControlError) -> (u16, ControlError) {
    let api = ApiError::from(error);
    let status = api.status;
    let json = serde_json::to_string(&api.body).unwrap();
    let body: ErrorBody = serde_json::from_str(&json).unwrap();
    (status, ApiError { status, body }.into_control_error())
}

#[test]
fn every_control_error_maps_to_its_status_and_code_and_back() {
    let table: Vec<(ControlError, u16, &str)> = vec![
        (ControlError::NotFound("receiver"), 404, "not_found"),
        (ControlError::NotFound("operation"), 404, "not_found"),
        (ControlError::NotFound("token"), 404, "not_found"),
        (ControlError::Forbidden, 403, "forbidden"),
        (
            ControlError::InvalidRequest("no".into()),
            400,
            "invalid_request",
        ),
        (ControlError::Unavailable("down".into()), 503, "unavailable"),
        (
            ControlError::TooLate(Box::new(snapshot(OperationStatus::InSession))),
            409,
            "too_late",
        ),
        (
            ControlError::RateLimited { retry_after: None },
            429,
            "rate_limited",
        ),
    ];
    for (error, status, code) in table {
        let api = ApiError::from(&error);
        assert_eq!((api.status, api.code()), (status, code), "{error:?}");
        let (_, back) = across_the_wire(&error);
        assert_eq!(back, error, "{error:?}");
    }

    // Retry-After is whole seconds, rounded up.
    for (given, secs) in [
        (Duration::from_secs(3), 3),
        (Duration::from_millis(1_500), 2),
        (Duration::from_millis(1), 1),
        (Duration::ZERO, 0),
    ] {
        let error = ControlError::RateLimited {
            retry_after: Some(given),
        };
        let api = ApiError::from(&error);
        assert_eq!(api.retry_after_secs(), Some(secs));
        assert_eq!(
            across_the_wire(&error).1,
            ControlError::RateLimited {
                retry_after: Some(Duration::from_secs(secs))
            }
        );
    }

    // Each kind of session error survives, with its message.
    for kind in ALL_KINDS {
        let inner = OperationError::new(kind, "using AVR session", "connecting to the receiver");
        let error = ControlError::Receiver(inner.clone());
        let api = ApiError::from(&error);
        assert_eq!((api.status, api.code()), (502, "receiver_error"));
        assert_eq!(api.body.error.kind.as_deref(), Some(kind_name(kind)));
        let (_, back) = across_the_wire(&error);
        let ControlError::Receiver(read) = back else {
            panic!("not a receiver error");
        };
        assert_eq!(read.kind, kind);
        assert_eq!(read.message, inner.to_string());
    }
    assert_eq!(ALL_KINDS.len(), 11);
}

#[test]
fn a_too_late_error_carries_the_operation_as_it_is() {
    let error = ControlError::TooLate(Box::new(snapshot(OperationStatus::Superseded {
        by: OperationId(9),
    })));
    let (status, back) = across_the_wire(&error);
    assert_eq!(status, 409);
    assert_eq!(back, error);
}

#[test]
fn a_not_found_word_outside_the_closed_set_becomes_resource() {
    let mut api = ApiError::from(&ControlError::NotFound("receiver"));
    api.body.error.what = Some("widget".into());
    assert_eq!(api.into_control_error(), ControlError::NotFound("resource"));
    // A route the server does not have is a not-found too, by its status.
    let route = ApiError::server(404, "no_such_route", "no such resource");
    assert_eq!(
        route.into_control_error(),
        ControlError::NotFound("resource")
    );
}

#[test]
fn an_error_the_server_adds_reads_as_the_port_would() {
    let cases = [
        (401, "unauthenticated", ControlError::Forbidden),
        (
            413,
            "payload_too_large",
            ControlError::InvalidRequest("m".into()),
        ),
        (
            412,
            "precondition_failed",
            ControlError::InvalidRequest("m".into()),
        ),
        (
            408,
            "request_timeout",
            ControlError::InvalidRequest("m".into()),
        ),
        (503, "shutting_down", ControlError::Unavailable("m".into())),
    ];
    for (status, code, expected) in cases {
        assert_eq!(
            ApiError::server(status, code, "m").into_control_error(),
            expected,
            "{code}"
        );
    }
    assert_eq!(
        ApiError::unauthenticated().into_control_error(),
        ControlError::Forbidden
    );
    let limited = ApiError::server(429, "too_many_streams", "four at most");
    assert_eq!(
        limited.into_control_error(),
        ControlError::RateLimited { retry_after: None }
    );
}

#[test]
fn an_operator_state_view_rebuilds_an_equal_state_except_the_stamps() {
    for seed in 0..64 {
        let state = seeded_state(seed);
        let view = OperatorStateView::from(&state);
        let json = serde_json::to_string(&view).unwrap();
        let read: OperatorStateView = serde_json::from_str(&json).unwrap();
        let rebuilt = read.into_state().unwrap();
        assert_eq!(
            normalize(&rebuilt, true),
            normalize(&state, true),
            "seed {seed}"
        );

        // The stamps are placeholders, and nothing was invented about progress.
        for observed in [
            rebuilt
                .system_power
                .last_good
                .as_ref()
                .map(|o| (o.frame_seq.0, o.observed_at.0)),
            rebuilt
                .main_zone
                .volume
                .last_good
                .as_ref()
                .map(|o| (o.frame_seq.0, o.observed_at.0)),
        ]
        .into_iter()
        .flatten()
        {
            assert_eq!(observed, (0, 0), "seed {seed}");
        }
        if let ReceiverFieldValidity::Current { valid_until } = rebuilt.main_zone.mute.validity {
            assert_eq!(valid_until.0, u64::MAX);
        }
    }
}

#[test]
fn an_agent_state_view_rebuilds_the_same_state_without_the_text() {
    for seed in 0..64 {
        let state = seeded_state(seed);
        let json = serde_json::to_string(&AgentStateView::from(&state)).unwrap();
        let rebuilt = serde_json::from_str::<AgentStateView>(&json)
            .unwrap()
            .into_state()
            .unwrap();
        assert_eq!(
            normalize(&rebuilt, false),
            normalize(&state, false),
            "seed {seed}"
        );
        assert!(rebuilt.diagnostics.is_empty());
        if let Some(mode) = &rebuilt.main_zone.sound_mode.last_good {
            assert_eq!(
                mode.value.raw, mode.value.id,
                "the receiver's own text is not sent"
            );
        }
        assert!(rebuilt.main_zone.mute.last_issue.is_none());
    }
}

#[test]
fn field_baseline_capture_agrees_before_and_after_the_round_trip() {
    for seed in 0..64 {
        let state = seeded_state(seed);
        for view in [
            serde_json::to_string(&OperatorStateView::from(&state)).unwrap(),
            serde_json::to_string(&AgentStateView::from(&state)).unwrap(),
        ] {
            let rebuilt = serde_json::from_str::<StateView<FieldText, Diagnostics>>(&view)
                .map(StateView::into_state)
                .or_else(|_| {
                    serde_json::from_str::<AgentStateView>(&view).map(StateView::into_state)
                })
                .unwrap()
                .unwrap();
            for field in FIELDS {
                assert_eq!(
                    FieldBaseline::capture(&rebuilt, field),
                    FieldBaseline::capture(&state, field),
                    "seed {seed}, {field:?}"
                );
            }
        }
    }
}

#[test]
fn a_field_that_is_stale_without_a_reason_is_not_guessed() {
    let state = sample_state();
    let mut json: serde_json::Value =
        serde_json::to_value(OperatorStateView::from(&state)).unwrap();
    json["main_zone"]["mute"]
        .as_object_mut()
        .unwrap()
        .remove("reason");
    let view: OperatorStateView = serde_json::from_value(json).unwrap();
    assert!(view.into_state().is_err());
}

fn catalog() -> SourceCatalogObservation {
    SourceCatalogObservation {
        catalog: SourceCatalog {
            entries: vec![
                SourceEntry {
                    id: SourceId::new("GAME").unwrap(),
                    display_name: Some("Console".into()),
                    visibility: SourceVisibility::Shown,
                },
                SourceEntry {
                    id: SourceId::new("AUX2").unwrap(),
                    display_name: None,
                    visibility: SourceVisibility::Hidden,
                },
                SourceEntry {
                    id: SourceId::new("TUNER").unwrap(),
                    display_name: Some("Radio".into()),
                    visibility: SourceVisibility::Unknown,
                },
            ],
            freshness: Freshness::Partial,
            generation: 4,
            observed_at: Some(UNIX_EPOCH + Duration::from_millis(1_700_000_000_123)),
            error: Some(ADDRESS_TEXT.into()),
        },
        raw_response: format!("<list>{RAW_FRAME}</list>"),
        response_evidence: CatalogResponseEvidence::Partial,
    }
}

#[test]
fn a_source_catalog_round_trips_and_an_agents_copy_drops_the_text() {
    let observation = catalog();
    let json = serde_json::to_string(&OperatorSourcesView::from(&observation)).unwrap();
    let operator = serde_json::from_str::<OperatorSourcesView>(&json)
        .unwrap()
        .into_observation()
        .unwrap();
    assert_eq!(operator, observation);

    let json = serde_json::to_string(&AgentSourcesView::from(&observation)).unwrap();
    let agent = serde_json::from_str::<AgentSourcesView>(&json)
        .unwrap()
        .into_observation()
        .unwrap();
    assert_eq!(agent.catalog.entries, observation.catalog.entries);
    assert_eq!(agent.catalog.generation, 4);
    assert_eq!(agent.response_evidence, CatalogResponseEvidence::Partial);
    assert_eq!(agent.raw_response, "");
    assert_eq!(agent.catalog.error, None);
}

fn every_intent() -> Vec<ReceiverIntent> {
    vec![
        ReceiverIntent::SystemPower(SystemPower::On),
        ReceiverIntent::SystemPower(SystemPower::Standby),
        ReceiverIntent::MainZonePower(ZonePower::On),
        ReceiverIntent::MainZonePower(ZonePower::Off),
        ReceiverIntent::Zone2Power(ZonePower::On),
        ReceiverIntent::Zone2Power(ZonePower::Off),
        ReceiverIntent::Source(SourceId::new("TV AUDIO").unwrap()),
        ReceiverIntent::Volume(MasterVolume::Minimum),
        ReceiverIntent::Volume(volume(-35.5)),
        ReceiverIntent::Mute(MuteState::On),
        ReceiverIntent::Mute(MuteState::Off),
        ReceiverIntent::SoundMode(SoundModeIntent::Auto),
        ReceiverIntent::SoundMode(SoundModeIntent::Direct),
        ReceiverIntent::SoundMode(SoundModeIntent::PureDirect),
        ReceiverIntent::SoundMode(SoundModeIntent::Stereo),
        ReceiverIntent::SoundMode(SoundModeIntent::RecallMovie),
        ReceiverIntent::SoundMode(SoundModeIntent::RecallMusic),
        ReceiverIntent::SoundMode(SoundModeIntent::RecallGame),
        ReceiverIntent::SoundMode(SoundModeIntent::Select("DOLBY ATMOS".into())),
    ]
}

#[test]
fn every_receiver_intent_has_a_wire_form_and_back() {
    for intent in every_intent() {
        let json = serde_json::to_string(&IntentDto::from(&intent)).unwrap();
        let back: ReceiverIntent = serde_json::from_str::<IntentDto>(&json)
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(back, intent, "{json}");
        // An exhaustive match here fails to compile when an intent is added, so
        // the list above cannot fall behind.
        match intent {
            ReceiverIntent::SystemPower(_)
            | ReceiverIntent::MainZonePower(_)
            | ReceiverIntent::Zone2Power(_)
            | ReceiverIntent::Source(_)
            | ReceiverIntent::Volume(_)
            | ReceiverIntent::Mute(_)
            | ReceiverIntent::SoundMode(_) => {}
        }
    }
}

#[test]
fn volume_wire_forms_cover_both_ends_and_refuse_off_scale_values() {
    let read = |json: &str| -> Result<ReceiverIntent, String> {
        serde_json::from_str::<IntentDto>(json)
            .map_err(|why| why.to_string())?
            .try_into()
    };
    assert_eq!(
        read(r#"{"kind":"volume","level":{"half_steps":-159}}"#).unwrap(),
        ReceiverIntent::Volume(volume(-79.5))
    );
    assert_eq!(
        read(r#"{"kind":"volume","level":{"half_steps":36}}"#).unwrap(),
        ReceiverIntent::Volume(volume(18.0))
    );
    assert_eq!(
        read(r#"{"kind":"volume","level":"minimum"}"#).unwrap(),
        ReceiverIntent::Volume(MasterVolume::Minimum)
    );
    for bad in [
        r#"{"kind":"volume","level":{"half_steps":-160}}"#,
        r#"{"kind":"volume","level":{"half_steps":37}}"#,
        r#"{"kind":"volume","level":{"half_steps":99999}}"#,
        r#"{"kind":"volume","level":{"half_steps":-71},"extra":1}"#,
        r#"{"kind":"volume","level":"loud"}"#,
        r#"{"kind":"volume"}"#,
    ] {
        assert!(read(bad).is_err(), "{bad}");
    }
    assert_eq!(VolumeDto::from(MasterVolume::Minimum), VolumeDto::Minimum);
}

#[test]
fn an_operation_snapshot_round_trips_for_every_status() {
    let statuses = [
        OperationStatus::Submitted,
        OperationStatus::Allowed,
        OperationStatus::AwaitingApproval,
        OperationStatus::Approved,
        OperationStatus::InSession,
        OperationStatus::Denied,
        OperationStatus::ApprovalUnavailable,
        OperationStatus::ApprovalRejected,
        OperationStatus::Expired,
        OperationStatus::Cancelled,
        OperationStatus::Completed,
        OperationStatus::AlreadyInState,
        OperationStatus::Rejected,
        OperationStatus::Superseded {
            by: OperationId(12),
        },
        OperationStatus::Indeterminate,
    ];
    assert_eq!(statuses.len(), 15);
    for status in statuses {
        let original = snapshot(status.clone());
        let json = serde_json::to_string(&OperationDto::from(&original)).unwrap();
        let dto: OperationDto = serde_json::from_str(&json).unwrap();
        // The wire's status name is the port's stable one.
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["status"], status.as_str(), "{json}");
        let back = OperationSnapshot::try_from(dto).unwrap();
        assert_eq!(back, original);
    }
    // A superseded operation that does not say by what is not guessed.
    let mut value =
        serde_json::to_value(OperationDto::from(&snapshot(OperationStatus::Superseded {
            by: OperationId(1),
        })))
        .unwrap();
    value.as_object_mut().unwrap().remove("superseded_by");
    let dto: OperationDto = serde_json::from_value(value).unwrap();
    assert!(OperationSnapshot::try_from(dto).is_err());
}

#[test]
fn dispatch_and_confirmed_round_trip() {
    for dispatch in [
        DispatchCertainty::NotDispatched,
        DispatchCertainty::PossiblyDispatched,
        DispatchCertainty::CompleteWrite,
        DispatchCertainty::Unknown,
    ] {
        for confirmed in [false, true] {
            let mut original = snapshot(OperationStatus::Completed);
            original.dispatch = dispatch.clone();
            original.confirmed = confirmed;
            let json = serde_json::to_string(&OperationDto::from(&original)).unwrap();
            let value: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(value["dispatch"], dispatch.as_str());
            let back =
                OperationSnapshot::try_from(serde_json::from_str::<OperationDto>(&json).unwrap())
                    .unwrap();
            assert_eq!(back, original);
        }
    }
}

#[test]
fn an_ad_hoc_receiver_id_and_a_saved_one_each_read_back() {
    use denon_avr_api_contract::parse_receiver_id;
    assert_eq!(
        parse_receiver_id("living-room").unwrap().as_str(),
        "living-room"
    );
    let ad_hoc = denon_avr_domain::ReceiverId::ad_hoc("192.0.2.50").unwrap();
    assert_eq!(parse_receiver_id(ad_hoc.as_str()).unwrap(), ad_hoc);
    assert!(parse_receiver_id("").is_err());
    assert!(parse_receiver_id("adhoc:").is_err());
    assert!(parse_receiver_id("a\nb").is_err());
    let _ = OperationErrorKind::Conflict;
}
