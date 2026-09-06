//! Denon AVR ASCII protocol primitives.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvrCommand(String);

impl AvrCommand {
    pub fn new(command: impl Into<String>) -> Result<Self, AvrProtocolError> {
        let command = command.into();
        if command
            .chars()
            .any(|character| matches!(character, '\r' | '\n'))
        {
            return Err(AvrProtocolError::InvalidCommand(
                "command contains a line break",
            ));
        }
        if command.is_empty() {
            return Err(AvrProtocolError::InvalidCommand("command is empty"));
        }
        Ok(Self(command))
    }

    pub fn as_bytes(&self) -> Vec<u8> {
        let mut bytes = self.0.as_bytes().to_vec();
        bytes.push(b'\r');
        bytes
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AvrLine {
    Response(AvrResponse),
    Event(AvrEvent),
    Raw(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvrResponse(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvrEvent(pub String);

/// Classify one CR-delimited AVR line.
///
/// The wire protocol uses the same shape for responses and events. Without a
/// pending request or a model-specific event table, the safe representation is
/// a raw response/event candidate; callers can correlate it with request state.
pub fn parse_line(line: &[u8]) -> Result<AvrLine, AvrProtocolError> {
    let text = std::str::from_utf8(line).map_err(|_| AvrProtocolError::InvalidUtf8)?;
    let text = text.strip_suffix('\r').unwrap_or(text);
    if text.is_empty() {
        return Err(AvrProtocolError::EmptyLine);
    }
    Ok(AvrLine::Raw(text.to_owned()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeCode(String);

impl VolumeCode {
    pub fn from_db_tenths(db_tenths: i16) -> Result<Self, AvrProtocolError> {
        if db_tenths % 5 != 0 {
            return Err(AvrProtocolError::InvalidVolume(
                "volume must use 0.5 dB steps",
            ));
        }
        // Reference Denon encoding: 80 == 0 dB. Whole dB values use two
        // characters; half-dB values append 5 to the two-character base.
        let whole_db = db_tenths.div_euclid(10);
        let base = 80i16 + whole_db;
        if !(0..=98).contains(&base) {
            return Err(AvrProtocolError::InvalidVolume(
                "volume is outside AVR code range",
            ));
        }
        let encoded = if db_tenths % 10 == 0 {
            format!("{base:02}")
        } else {
            format!("{base:02}5")
        };
        Ok(Self(encoded))
    }

    pub fn whole_db(code: u8) -> Result<Self, AvrProtocolError> {
        if code > 98 {
            return Err(AvrProtocolError::InvalidVolume(
                "AVR volume code must be 00..98",
            ));
        }
        Ok(Self(format!("{code:02}")))
    }

    pub fn code(&self) -> &str {
        &self.0
    }

    pub fn command(&self) -> String {
        format!("MV{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AvrProtocolError {
    InvalidCommand(&'static str),
    InvalidUtf8,
    EmptyLine,
    InvalidVolume(&'static str),
}

impl fmt::Display for AvrProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCommand(message) | Self::InvalidVolume(message) => f.write_str(message),
            Self::InvalidUtf8 => f.write_str("AVR line is not valid UTF-8"),
            Self::EmptyLine => f.write_str("AVR line is empty"),
        }
    }
}

impl std::error::Error for AvrProtocolError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_is_carriage_return_terminated() {
        let command = AvrCommand::new("SICD").unwrap();
        assert_eq!(command.as_bytes(), b"SICD\r");
    }

    #[test]
    fn command_rejects_embedded_line_breaks() {
        assert!(AvrCommand::new("SI\rCD").is_err());
    }

    #[test]
    fn volume_code_uses_reference_zero_db_encoding() {
        assert_eq!(VolumeCode::from_db_tenths(0).unwrap().command(), "MV80");
        assert_eq!(VolumeCode::from_db_tenths(5).unwrap().command(), "MV805");
        assert_eq!(VolumeCode::from_db_tenths(-5).unwrap().command(), "MV795");
    }
}
