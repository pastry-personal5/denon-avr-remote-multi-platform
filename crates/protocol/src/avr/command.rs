//! AVR command validation and framing.

use denon_avr_domain::{MainZoneControl, MainZoneField, MuteState, PowerState};
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

/// Read-only context queries used to determine which listening modes are valid.
/// `SD?` reports the source input mode and `DC?` reports the digital decoder mode.
pub fn audio_context_query_commands() -> [AvrCommand; 5] {
    [
        AvrCommand("SI?".into()),
        AvrCommand("SD?".into()),
        AvrCommand("DC?".into()),
        AvrCommand("MS?".into()),
        AvrCommand("CV?".into()),
    ]
}

pub fn is_read_only_audio_context_query(command: &str) -> bool {
    matches!(command, "SI?" | "SD?" | "DC?" | "MS?" | "CV?")
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

pub fn encode_native_volume(code: u16) -> Result<AvrCommand, AvrProtocolError> {
    if code > 985 || !code.is_multiple_of(5) {
        return Err(AvrProtocolError::InvalidVolume(
            "volume code is outside AVR code range",
        ));
    }
    let command = if code % 10 == 5 {
        format!("MV{:02}5", code / 10)
    } else {
        format!("MV{:02}", code / 10)
    };
    AvrCommand::new(command)
}

pub fn encode_control(control: &MainZoneControl) -> Result<AvrCommand, AvrProtocolError> {
    match control {
        MainZoneControl::Power(PowerState::On) => AvrCommand::new("PWON"),
        MainZoneControl::Power(PowerState::Standby) => AvrCommand::new("PWSTANDBY"),
        MainZoneControl::Input(value) => AvrCommand::new(format!("SI{}", value.as_str())),
        MainZoneControl::Volume(value) => encode_native_volume(value.to_native_code()),
        MainZoneControl::Mute(MuteState::On) => AvrCommand::new("MUON"),
        MainZoneControl::Mute(MuteState::Off) => AvrCommand::new("MUOFF"),
        MainZoneControl::SurroundMode(value) => AvrCommand::new(format!("MS{}", value.as_str())),
        MainZoneControl::ListeningModeGroup(value) => {
            AvrCommand::new(format!("MS{}", value.command_suffix()))
        }
    }
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
    use crate::avr::{parse_main_zone_event, parse_main_zone_response};
    use denon_avr_domain::{MainZoneEvent, MainZoneValue, PowerState};

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
        let parsed_volume = |response| {
            let MainZoneValue::Volume(volume) =
                parse_main_zone_response(MainZoneField::Volume, response).unwrap()
            else {
                panic!("expected volume");
            };
            volume.db_tenths()
        };
        assert_eq!(parsed_volume("MV800"), 0);
        assert_eq!(parsed_volume("MV980"), 180);
        assert_eq!(parsed_volume("MV80"), 0);
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

    #[test]
    fn encodes_typed_main_zone_controls() {
        use denon_avr_domain::{
            Input, MainZoneControl, MuteState, PowerState, SurroundMode, VolumeLevel,
        };
        assert_eq!(
            encode_control(&MainZoneControl::Power(PowerState::On))
                .unwrap()
                .as_str(),
            "PWON"
        );
        assert_eq!(
            encode_control(&MainZoneControl::Power(PowerState::Standby))
                .unwrap()
                .as_str(),
            "PWSTANDBY"
        );
        assert_eq!(
            encode_control(&MainZoneControl::Input(Input::new("CD").unwrap()))
                .unwrap()
                .as_str(),
            "SICD"
        );
        assert_eq!(
            encode_control(&MainZoneControl::Volume(VolumeLevel::new(500).unwrap()))
                .unwrap()
                .as_str(),
            "MV495"
        );
        assert_eq!(
            encode_control(&MainZoneControl::Mute(MuteState::Off))
                .unwrap()
                .as_str(),
            "MUOFF"
        );
        assert_eq!(
            encode_control(&MainZoneControl::SurroundMode(
                SurroundMode::new("STEREO").unwrap()
            ))
            .unwrap()
            .as_str(),
            "MSSTEREO"
        );
    }

    #[test]
    fn command_rejects_line_breaks_and_empty_values() {
        assert!(matches!(
            AvrCommand::new("SI?\r"),
            Err(AvrProtocolError::InvalidCommand(_))
        ));
        assert!(matches!(
            AvrCommand::new(""),
            Err(AvrProtocolError::InvalidCommand(_))
        ));
    }

    #[test]
    fn query_commands_cover_each_main_zone_field() {
        assert_eq!(query_command(MainZoneField::Input).as_str(), "SI?");
        assert_eq!(query_command(MainZoneField::Volume).as_str(), "MV?");
        assert_eq!(query_command(MainZoneField::Mute).as_str(), "MU?");
        assert_eq!(query_command(MainZoneField::SurroundMode).as_str(), "MS?");
    }

    #[test]
    fn encodes_remembered_listening_mode_groups_and_context_queries() {
        use denon_avr_domain::ListeningModeGroup;
        assert_eq!(
            encode_control(&MainZoneControl::ListeningModeGroup(
                ListeningModeGroup::Movie
            ))
            .unwrap()
            .as_str(),
            "MSMOVIE"
        );
        assert_eq!(
            encode_control(&MainZoneControl::ListeningModeGroup(
                ListeningModeGroup::Music
            ))
            .unwrap()
            .as_str(),
            "MSMUSIC"
        );
        assert_eq!(
            encode_control(&MainZoneControl::ListeningModeGroup(
                ListeningModeGroup::Game
            ))
            .unwrap()
            .as_str(),
            "MSGAME"
        );
        let queries = audio_context_query_commands();
        assert_eq!(queries[0].as_str(), "SI?");
        assert_eq!(queries[1].as_str(), "SD?");
        assert_eq!(queries[2].as_str(), "DC?");
        assert_eq!(queries[3].as_str(), "MS?");
        assert_eq!(queries[4].as_str(), "CV?");
        assert!(queries
            .iter()
            .all(|query| is_read_only_audio_context_query(query.as_str())));
        assert!(!is_read_only_audio_context_query("PWON"));
    }
}
