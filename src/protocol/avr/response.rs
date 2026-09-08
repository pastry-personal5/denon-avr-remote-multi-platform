//! AVR response correlation, parsing, and event reduction.

use super::command::AvrProtocolError;
use crate::domain::{
    AudioContextField, AudioContextValue, Input, MainZoneEvent, MainZoneField, MainZoneValue,
    MuteState, PowerState, SurroundMode, Volume,
};

/// CV is a channel trim/configuration response, never an active channel map.
pub fn parse_channel_volume_response(response: &str) -> Result<String, AvrProtocolError> {
    response
        .strip_prefix("CV")
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .ok_or(AvrProtocolError::MalformedResponse("invalid CV response"))
}

pub fn parse_audio_context_response(
    field: AudioContextField,
    response: &str,
) -> Result<AudioContextValue, AvrProtocolError> {
    let prefix = match field {
        AudioContextField::InputMode => "SD",
        AudioContextField::DigitalMode => "DC",
    };
    response
        .strip_prefix(prefix)
        .filter(|value| !value.is_empty())
        .and_then(|value| AudioContextValue::new(value).ok())
        .ok_or(AvrProtocolError::MalformedResponse(
            "invalid audio context response",
        ))
}

pub fn get_command_family(command: &str) -> &str {
    command
        .split_once('?')
        .map_or(command, |(family, _)| family)
        .trim_end()
}

pub fn response_matches(family: &str, response: &str) -> bool {
    let Some(suffix) = response.strip_prefix(family) else {
        return false;
    };
    family != "MV"
        || suffix == "---"
        || (!suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()))
}

pub fn parse_main_zone_response(
    field: MainZoneField,
    response: &str,
) -> Result<MainZoneValue, AvrProtocolError> {
    match field {
        MainZoneField::Power => match response {
            "PWON" => Ok(MainZoneValue::Power(PowerState::On)),
            "PWSTANDBY" => Ok(MainZoneValue::Power(PowerState::Standby)),
            _ => Err(unexpected(field, response)),
        },
        MainZoneField::Input => prefixed(response, "SI", field)
            .and_then(|value| Input::new(value).map_err(AvrProtocolError::MalformedResponse))
            .map(MainZoneValue::Input),
        MainZoneField::Volume => parse_volume(response).map(MainZoneValue::Volume),
        MainZoneField::Mute => match response {
            "MUON" => Ok(MainZoneValue::Mute(MuteState::On)),
            "MUOFF" => Ok(MainZoneValue::Mute(MuteState::Off)),
            _ => Err(unexpected(field, response)),
        },
        MainZoneField::SurroundMode => prefixed(response, "MS", field)
            .and_then(|value| SurroundMode::new(value).map_err(AvrProtocolError::MalformedResponse))
            .map(MainZoneValue::SurroundMode),
    }
}

pub fn parse_main_zone_event(line: &str) -> MainZoneEvent {
    for field in MainZoneField::ALL {
        if let Ok(value) = parse_main_zone_response(field, line) {
            return MainZoneEvent::Changed(value);
        }
    }
    MainZoneEvent::Unknown(line.to_owned())
}

fn prefixed<'a>(
    response: &'a str,
    prefix: &str,
    field: MainZoneField,
) -> Result<&'a str, AvrProtocolError> {
    response
        .strip_prefix(prefix)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| unexpected(field, response))
}

fn parse_volume(response: &str) -> Result<Volume, AvrProtocolError> {
    let code = response
        .strip_prefix("MV")
        .ok_or_else(|| unexpected(MainZoneField::Volume, response))?;
    if code == "---" {
        return Err(AvrProtocolError::Unavailable("volume is unavailable"));
    }
    let half = code.ends_with('5');
    let base_text = if half { &code[..code.len() - 1] } else { code };
    let base = base_text
        .parse::<i16>()
        .map_err(|_| AvrProtocolError::InvalidVolume("volume code is not numeric"))?;
    if !(0..=98).contains(&base) || (half && code.len() != 3) || (!half && code.len() != 2) {
        return Err(AvrProtocolError::InvalidVolume(
            "volume code is outside the supported format",
        ));
    }
    Ok(Volume::from_parts(
        code,
        (base - 80) * 10 + i16::from(half) * 5,
    ))
}

fn unexpected(field: MainZoneField, response: &str) -> AvrProtocolError {
    AvrProtocolError::UnexpectedResponse {
        field,
        response: response.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_family_is_the_command_prefix_before_query_marker() {
        assert_eq!(get_command_family("SI?"), "SI");
        assert_eq!(get_command_family("Z2?"), "Z2");
        assert_eq!(get_command_family("PSMULTEQ: ?"), "PSMULTEQ:");
        assert_eq!(get_command_family("PSDYNEQ ?"), "PSDYNEQ");
        assert_eq!(get_command_family("CUSTOM"), "CUSTOM");
    }

    #[test]
    fn response_matching_rejects_wrong_or_malformed_families() {
        assert!(response_matches("SI", "SICD"));
        assert!(!response_matches("SI", "MSSTEREO"));
        assert!(response_matches("MV", "MV---"));
        assert!(response_matches("MV", "MV805"));
        assert!(!response_matches("MV", "MV80.5"));
        assert!(!response_matches("MV", "MVMAX 615"));
        assert!(response_matches("MSQUICK1", "MSQUICK1"));
        assert!(!response_matches("MSQUICK1", "MSQUICKX"));
    }

    #[test]
    fn parses_audio_context_query_responses() {
        assert_eq!(
            parse_audio_context_response(crate::domain::AudioContextField::InputMode, "SDHDMI")
                .unwrap()
                .as_str(),
            "HDMI"
        );
        assert_eq!(
            parse_audio_context_response(crate::domain::AudioContextField::DigitalMode, "DCAUTO")
                .unwrap()
                .as_str(),
            "AUTO"
        );
        assert!(parse_audio_context_response(
            crate::domain::AudioContextField::DigitalMode,
            "MSSTEREO"
        )
        .is_err());
    }
}
