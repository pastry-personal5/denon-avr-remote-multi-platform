//! AVR command validation and framing.

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AvrProtocolError {
    InvalidCommand(&'static str),
    InvalidVolume(&'static str),
    MalformedResponse(&'static str),
}

impl fmt::Display for AvrProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCommand(message)
            | Self::InvalidVolume(message)
            | Self::MalformedResponse(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for AvrProtocolError {}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_command_is_framed_with_one_carriage_return() {
        assert_eq!(AvrCommand::new("ZM?").unwrap().as_bytes(), b"ZM?\r");
    }
}
