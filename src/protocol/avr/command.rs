//! AVR command validation and framing.

use crate::domain::MainZoneField;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AvrProtocolError {
    InvalidCommand(&'static str),
    InvalidUtf8,
    EmptyLine,
    InvalidVolume(&'static str),
    MalformedResponse(&'static str),
    Unavailable(&'static str),
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
            | Self::MalformedResponse(message)
            | Self::Unavailable(message) => f.write_str(message),
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
    use crate::domain::{MainZoneEvent, MainZoneValue, PowerState};
    use crate::protocol::avr::{parse_main_zone_event, parse_main_zone_response};

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
    fn mv_unknown_is_typed_unavailable() {
        assert!(matches!(
            parse_main_zone_response(MainZoneField::Volume, "MV---"),
            Err(AvrProtocolError::Unavailable("volume is unavailable"))
        ));
    }

    #[test]
    fn preserves_unknown_events() {
        assert_eq!(
            parse_main_zone_event("MVMAX 615"),
            MainZoneEvent::Unknown("MVMAX 615".into())
        );
    }

    #[test]
    fn encodes_volume_in_half_db_steps() {
        assert_eq!(encode_volume(5).unwrap().as_str(), "MV805");
        assert!(encode_volume(3).is_err());
    }
}
