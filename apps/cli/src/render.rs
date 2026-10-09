//! Text a command prints: the report, the target and readiness lines, a state
//! resource, an operation outcome, and the discovered receiver list.

use crate::args::Resource;
use denon_avr_application::OperationSnapshot;
use denon_avr_domain::{
    CoreField, DiscoveredReceiver, FieldBaseline, ReceiverIdentity, ReceiverState,
};

/// What a command produced. Text goes to standard output and warnings to
/// standard error, so a failure to remember the receiver never hides a result.
#[derive(Debug, Default)]
pub(super) struct Report {
    pub(super) text: String,
    pub(super) warnings: Vec<String>,
}

pub(super) fn readiness_line(state: &ReceiverState) -> String {
    const CORE: [CoreField; 7] = [
        CoreField::SystemPower,
        CoreField::MainZonePower,
        CoreField::Zone2Power,
        CoreField::Source,
        CoreField::Volume,
        CoreField::Mute,
        CoreField::SoundMode,
    ];
    let usable = CORE.iter().all(|field| {
        matches!(
            FieldBaseline::capture(state, *field),
            FieldBaseline::Value(_)
        )
    });
    if usable {
        "Readiness: ready\n".into()
    } else {
        "Readiness: degraded (one or more core fields could not be observed)\n".into()
    }
}

/// An outcome in the design's terms: what the service decided, whether anything
/// was dispatched, and whether receiver evidence confirms the requested value.
/// A completed write is not a confirmation on its own.
pub(super) fn outcome_text(snapshot: &OperationSnapshot) -> String {
    let mut text = format!("Outcome: {}", snapshot.status.as_str());
    if let denon_avr_application::OperationStatus::Superseded { by } = &snapshot.status {
        text += &format!(" (by operation {})", by.0);
    }
    text += &format!(
        "\nDispatch: {}\nConfirmed: {}\n",
        snapshot.dispatch.as_str(),
        snapshot.confirmed
    );
    if let Some(observation) = &snapshot.observation {
        text += &format!("Observation: {observation}\n");
    }
    if let Some(reason) = &snapshot.reason {
        text += &format!("Reason: {reason}\n");
    }
    text
}

pub(super) fn target_line(identity: &ReceiverIdentity) -> String {
    format!(
        "Receiver: {} ({})\n",
        identity
            .friendly_name
            .as_deref()
            .or(identity.model.as_deref())
            .unwrap_or("receiver"),
        identity.host
    )
}
pub(super) fn render_resource(resource: Resource, state: &ReceiverState) -> String {
    match resource {
        Resource::Status => format!("{state:#?}\n"),
        Resource::Power => format!("Main Zone power: {:?}\n", state.main_zone.power),
        Resource::Input => format!("Source: {:?}\n", state.main_zone.source),
        Resource::Volume => format!("Volume: {:?}\n", state.main_zone.volume),
        Resource::Mute => format!("Mute: {:?}\n", state.main_zone.mute),
        Resource::Surround => format!("Sound mode: {:?}\n", state.main_zone.sound_mode),
    }
}
pub(super) fn receivers_text(receivers: &[DiscoveredReceiver]) -> String {
    if receivers.is_empty() {
        return "No receivers found.\n".into();
    }
    receivers
        .iter()
        .enumerate()
        .map(|(index, receiver)| {
            format!(
                "{}. {} ({})\n",
                index + 1,
                receiver
                    .model
                    .as_deref()
                    .or(receiver.server.as_deref())
                    .unwrap_or("Denon receiver"),
                receiver.address.host
            )
        })
        .collect()
}
