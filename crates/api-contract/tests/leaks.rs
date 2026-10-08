//! What an agent can never be told. The session's state and the source catalog
//! carry free text that can name an address: the message of a failed query, a raw
//! frame, the receiver's own status text. The Agent views are types with no field
//! to put it in, and these tests seed every place it can be.

mod common;

use common::*;
use denon_avr_api_contract::{
    AgentSourcesView, AgentStateView, CapabilitiesDto, ConnectionDto, OperatorSourcesView,
    OperatorStateView, ReceiverSummaryDto,
};
use denon_avr_domain::receiver_state::ReceiverEvidence;
use denon_avr_domain::{FieldIssue, ReceiverFieldValidity};

/// The sample state with the address in the issue of every one of the seven
/// fields, in the receiver's status text, and in the diagnostics.
fn state_with_text_everywhere() -> denon_avr_domain::ReceiverState {
    let mut state = sample_state();
    let issue = || {
        Some(FieldIssue {
            message: ADDRESS_TEXT.into(),
        })
    };
    state.system_power.last_issue = issue();
    state.main_zone.power.last_issue = issue();
    state.main_zone.source.last_issue = issue();
    state.main_zone.volume.last_issue = issue();
    state.main_zone.mute.last_issue = issue();
    state.main_zone.sound_mode.last_issue = issue();
    state.zone2_power.last_issue = issue();
    state.main_zone.volume.validity = ReceiverFieldValidity::Unavailable {
        evidence: ReceiverEvidence::UnavailableStatus(format!("{ADDRESS_TEXT} {RAW_FRAME}")),
    };
    state
}

const FORBIDDEN: [&str; 6] = ["192.168", "connecting", "1.20", ":23", "MVMAX", "malformed"];

fn assert_clean(what: &str, text: &str) {
    for needle in FORBIDDEN {
        assert!(!text.contains(needle), "{what} carries {needle:?}: {text}");
    }
}

#[test]
fn an_address_seeded_into_every_free_text_field_never_reaches_an_agent_view() {
    let state = state_with_text_everywhere();
    let agent = serde_json::to_string(&AgentStateView::from(&state)).unwrap();
    assert_clean("the agent's state view", &agent);

    let catalog = catalog_with_text();
    let agent_sources = serde_json::to_string(&AgentSourcesView::from(&catalog)).unwrap();
    assert_clean("the agent's source view", &agent_sources);
    // The receiver's own label for a source is what the agent is given to name it.
    assert!(agent_sources.contains("Console"));

    // The same seeds reach the Operator, so the check above can fail.
    let operator = serde_json::to_string(&OperatorStateView::from(&state)).unwrap();
    assert!(operator.contains("192.168.1.20"), "{operator}");
    assert!(operator.contains("MVMAX"));
    let operator_sources = serde_json::to_string(&OperatorSourcesView::from(&catalog)).unwrap();
    assert!(operator_sources.contains("192.168.1.20") && operator_sources.contains("MVMAX"));
}

#[test]
fn the_agent_views_have_no_text_field() {
    let state = state_with_text_everywhere();
    let mut found = keys(&serde_json::to_value(AgentStateView::from(&state)).unwrap());
    found.extend(keys(
        &serde_json::to_value(AgentSourcesView::from(&catalog_with_text())).unwrap(),
    ));
    for text in [
        "issue",
        "evidence",
        "raw",
        "diagnostics",
        "raw_response",
        "error",
    ] {
        assert!(!found.contains(text), "{text} is a field of an agent view");
    }
    let listing = found.into_iter().collect::<Vec<_>>().join("\n");
    golden("agent_view_keys.txt", &listing);
}

#[test]
fn the_operator_view_keeps_the_issue_text_the_gui_shows() {
    let state = state_with_text_everywhere();
    let json = serde_json::to_string(&OperatorStateView::from(&state)).unwrap();
    let rebuilt = serde_json::from_str::<OperatorStateView>(&json)
        .unwrap()
        .into_state()
        .unwrap();
    let message = |field: &Option<FieldIssue>| field.as_ref().map(|i| i.message.clone());
    assert_eq!(
        message(&rebuilt.main_zone.mute.last_issue).as_deref(),
        Some(ADDRESS_TEXT)
    );
    assert_eq!(
        message(&rebuilt.zone2_power.last_issue).as_deref(),
        Some(ADDRESS_TEXT)
    );
    assert_eq!(rebuilt.diagnostics, state.diagnostics);
    // The receiver's own status text and its name for the sound mode too.
    assert!(matches!(
        &rebuilt.main_zone.volume.validity,
        ReceiverFieldValidity::Unavailable { evidence: ReceiverEvidence::UnavailableStatus(text) }
            if text.contains(RAW_FRAME)
    ));
    assert_eq!(
        rebuilt.main_zone.sound_mode.last_good.unwrap().value.raw,
        state.main_zone.sound_mode.last_good.unwrap().value.raw
    );
}

#[test]
fn a_receiver_summary_has_no_address_field() {
    let summary = ReceiverSummaryDto {
        id: "living-room".into(),
        model: Some("AVR-X3800H".into()),
        capabilities: CapabilitiesDto {
            writable: true,
            zone2_power: true,
            source_catalog_read: true,
            inputs: vec!["BD".into(), "GAME".into()],
            surround_modes: vec!["DIRECT".into()],
        },
        connection: ConnectionDto::Released,
    };
    let found = keys(&serde_json::to_value(&summary).unwrap());
    let allowed = [
        "id",
        "model",
        "capabilities",
        "writable",
        "zone2_power",
        "source_catalog_read",
        "inputs",
        "surround_modes",
        "connection",
    ];
    for key in &found {
        assert!(allowed.contains(&key.as_str()), "unexpected field {key}");
    }
    for address in [
        "host", "address", "ip", "port", "location", "endpoint", "url",
    ] {
        assert!(!found.contains(address), "{address}");
    }
}
