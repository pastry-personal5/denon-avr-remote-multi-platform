use crate::domain::{
    Input, MainZoneEvent, MainZoneField, MainZoneValue, MuteState, PowerState, SurroundMode, Volume,
};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvrCommand(String);

impl AvrCommand {
    pub fn new(command: impl Into<String>) -> Result<Self, AvrProtocolError> {
        let command = command.into();
        if command.is_empty() {
            return Err(AvrProtocolError::InvalidCommand("command is empty"));
        }
        if command.contains(['\r', '\n']) {
            return Err(AvrProtocolError::InvalidCommand(
                "command contains a line break",
            ));
        }
        Ok(Self(command))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn as_bytes(&self) -> Vec<u8> {
        let mut bytes = self.0.as_bytes().to_vec();
        bytes.push(b'\r');
        bytes
    }
}

pub fn query_command(field: MainZoneField) -> AvrCommand {
    let command = match field {
        MainZoneField::Power => "PW?",
        MainZoneField::Input => "SI?",
        MainZoneField::Volume => "MV?",
        MainZoneField::Mute => "MU?",
        MainZoneField::SurroundMode => "MS?",
    };
    AvrCommand(command.into())
}

pub fn command_family(command: &str) -> &str {
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
        || (!suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_digit()))
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

pub fn parse_event(line: &str) -> MainZoneEvent {
    for field in MainZoneField::ALL {
        if let Ok(value) = parse_main_zone_response(field, line) {
            return MainZoneEvent::Changed(value);
        }
    }
    MainZoneEvent::Unknown(line.into())
}

pub fn encode_volume(db_tenths: i16) -> Result<AvrCommand, AvrProtocolError> {
    if db_tenths % 5 != 0 {
        return Err(AvrProtocolError::InvalidVolume(
            "volume must use 0.5 dB steps",
        ));
    }
    let whole_db = db_tenths.div_euclid(10);
    let base = 80i16 + whole_db;
    if !(0..=98).contains(&base) {
        return Err(AvrProtocolError::InvalidVolume(
            "volume is outside AVR code range",
        ));
    }
    let code = if db_tenths % 10 == 0 {
        format!("{base:02}")
    } else {
        format!("{base:02}5")
    };
    AvrCommand::new(format!("MV{code}"))
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
        return Err(AvrProtocolError::MalformedResponse("volume is unknown"));
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
        response: response.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AvrProtocolError {
    InvalidCommand(&'static str),
    InvalidUtf8,
    EmptyLine,
    InvalidVolume(&'static str),
    MalformedResponse(&'static str),
    UnexpectedResponse {
        field: MainZoneField,
        response: String,
    },
}

impl fmt::Display for AvrProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCommand(message)
            | Self::InvalidVolume(message)
            | Self::MalformedResponse(message) => f.write_str(message),
            Self::InvalidUtf8 => f.write_str("AVR line is not valid UTF-8"),
            Self::EmptyLine => f.write_str("AVR line is empty"),
            Self::UnexpectedResponse { field, response } => {
                write!(f, "unexpected {} response {response}", field.name())
            }
        }
    }
}

impl std::error::Error for AvrProtocolError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_queries_and_parses_typed_values() {
        assert_eq!(query_command(MainZoneField::Power).as_bytes(), b"PW?\r");
        assert_eq!(
            parse_main_zone_response(MainZoneField::Power, "PWON").unwrap(),
            MainZoneValue::Power(PowerState::On)
        );
        let MainZoneValue::Volume(volume) =
            parse_main_zone_response(MainZoneField::Volume, "MV795").unwrap()
        else {
            panic!("expected volume");
        };
        assert_eq!(volume.db_tenths(), -5);
    }

    #[test]
    fn preserves_unknown_events() {
        assert_eq!(
            parse_event("MVMAX 615"),
            MainZoneEvent::Unknown("MVMAX 615".into())
        );
    }

    #[test]
    fn encodes_volume_in_half_db_steps() {
        assert_eq!(encode_volume(5).unwrap().as_str(), "MV805");
        assert!(encode_volume(3).is_err());
    }
}
