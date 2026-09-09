//! Translation of X3800H AppCommand replies into presentation-safe facts.

use crate::app_command::{AppCommandParameter, AppCommandResponse};
use denon_avr_domain::{
    AudioInformation, AudysseyInformation, ChannelSlot, ChannelSlotState, FieldError,
    FieldErrorKind, FieldStatus, Freshness, HttpInformationSnapshot, VideoInformation,
};
use std::time::SystemTime;

const INPUT: &str = "GetInputSignal";
const OUTPUT: &str = "GetActiveSpeaker";
const AUDIO: &str = "GetAudioInfo";
const VIDEO: &str = "GetVideoInfo";
const AUDYSSEY: &str = "GetAudyssyInfo";

pub const INFORMATION_COMMANDS: [&str; 5] = [INPUT, OUTPUT, VIDEO, AUDIO, AUDYSSEY];

pub fn missing_information_commands(response: &AppCommandResponse) -> Vec<&'static str> {
    INFORMATION_COMMANDS
        .into_iter()
        .filter(|name| response.command(name).is_none())
        .collect()
}

pub fn parse_http_information(
    response: &AppCommandResponse,
    generation: u64,
) -> HttpInformationSnapshot {
    let mut snapshot = HttpInformationSnapshot {
        generation,
        observed_at: Some(SystemTime::now()),
        freshness: Freshness::Live,
        ..Default::default()
    };
    snapshot.input_slots = slots(response, INPUT);
    snapshot.output_slots = slots(response, OUTPUT);
    snapshot.audio = AudioInformation {
        input_mode: text(response, AUDIO, "inputmode"),
        output: text(response, AUDIO, "output"),
        signal: text(response, AUDIO, "signal"),
        sound: text(response, AUDIO, "sound"),
        sample_rate: text(response, AUDIO, "fs"),
    };
    snapshot.video = VideoInformation {
        monitor: text(response, VIDEO, "videooutput"),
        hdmi_input: text(response, VIDEO, "hdmisigin"),
        hdmi_output: text(response, VIDEO, "hdmisigout"),
    };
    snapshot.audyssey = AudysseyInformation {
        multeq: text(response, AUDYSSEY, "eqvalue"),
        dynamic_eq: text(response, AUDYSSEY, "dynamiceq"),
        dynamic_volume: text(response, AUDYSSEY, "dynamicvol"),
    };
    if !missing_information_commands(response).is_empty() {
        snapshot.freshness = Freshness::Partial;
    }
    snapshot
}

fn unavailable(message: impl Into<String>) -> FieldStatus<String> {
    FieldStatus::Unavailable(FieldError {
        kind: FieldErrorKind::Unavailable,
        message: message.into(),
    })
}
fn text(response: &AppCommandResponse, command: &str, parameter: &str) -> FieldStatus<String> {
    let Some(command) = response.command(command) else {
        return unavailable("receiver did not return this information");
    };
    let Some(parameter) = command.parameter(parameter) else {
        return unavailable("receiver did not return this value");
    };
    let value = parameter.trimmed_value();
    if value.is_empty() {
        unavailable("receiver returned an empty value")
    } else {
        FieldStatus::Value(value.to_owned())
    }
}
fn slots(response: &AppCommandResponse, name: &str) -> FieldStatus<Vec<ChannelSlot>> {
    let Some(command) = response.command(name) else {
        return FieldStatus::Unavailable(FieldError {
            kind: FieldErrorKind::Unavailable,
            message: "receiver did not return channel layout".into(),
        });
    };
    let slots = command
        .parameters
        .iter()
        .filter_map(slot)
        .collect::<Vec<_>>();
    if slots.is_empty() {
        FieldStatus::Unavailable(FieldError {
            kind: FieldErrorKind::Unavailable,
            message: "receiver returned no channel slots".into(),
        })
    } else {
        FieldStatus::Value(slots)
    }
}
fn slot(parameter: &AppCommandParameter) -> Option<ChannelSlot> {
    let label = parameter.trimmed_value();
    if label.is_empty() {
        return None;
    }
    Some(ChannelSlot {
        label: label.to_ascii_uppercase(),
        state: match parameter.control_code() {
            Some(2) => ChannelSlotState::Active,
            Some(1) => ChannelSlotState::Available,
            Some(0) => ChannelSlotState::Absent,
            _ => ChannelSlotState::Unknown,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_command::parse_app_command_response;
    #[test]
    fn maps_slot_controls_without_exposing_the_codes() {
        let reply = parse_app_command_response("<rx><cmd><name>GetInputSignal</name><param name=\"a\" control=\"2\">FL</param><param name=\"b\" control=\"1\">LFE</param><param name=\"c\" control=\"9\">ZZ</param></cmd></rx>").unwrap();
        let info = parse_http_information(&reply, 2);
        let FieldStatus::Value(slots) = info.input_slots else {
            panic!()
        };
        assert_eq!(slots[0].state, ChannelSlotState::Active);
        assert_eq!(slots[1].state, ChannelSlotState::Available);
        assert_eq!(slots[2].state, ChannelSlotState::Unknown);
        assert_eq!(info.freshness, Freshness::Partial);
    }
}
