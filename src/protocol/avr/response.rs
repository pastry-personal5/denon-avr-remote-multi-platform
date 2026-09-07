//! AVR response correlation, parsing, and event reduction.

use super::command::AvrProtocolError;
use crate::domain::{
    Input, MainZoneEvent, MainZoneField, MainZoneValue, MuteState, PowerState, SurroundMode, Volume,
};

pub fn get_command_family(command: &str) -> &str {
    command
        .split_once('?')
        .map_or(command, |(family, _)| family)
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
