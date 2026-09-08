//! HEOS CLI framing primitives.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeosCommand(String);

impl HeosCommand {
    pub fn new(command: impl Into<String>) -> Result<Self, HeosProtocolError> {
        let command = command.into();
        if command.is_empty()
            || command
                .chars()
                .any(|character| matches!(character, '\r' | '\n'))
        {
            return Err(HeosProtocolError::InvalidCommand);
        }
        if !command.starts_with("heos://") {
            return Err(HeosProtocolError::InvalidCommand);
        }
        Ok(Self(command))
    }

    pub fn as_bytes(&self) -> Vec<u8> {
        let mut bytes = self.0.as_bytes().to_vec();
        bytes.extend_from_slice(b"\r\n");
        bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeosLine {
    Json(String),
    Raw(String),
}

pub fn parse_heos_line(line: &[u8]) -> Result<HeosLine, HeosProtocolError> {
    let text = std::str::from_utf8(line).map_err(|_| HeosProtocolError::InvalidUtf8)?;
    let text = text.strip_suffix("\r\n").unwrap_or(text);
    if text.is_empty() {
        return Err(HeosProtocolError::EmptyLine);
    }
    if text.starts_with('{') {
        Ok(HeosLine::Json(text.to_owned()))
    } else {
        Ok(HeosLine::Raw(text.to_owned()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeosProtocolError {
    InvalidCommand,
    InvalidUtf8,
    EmptyLine,
}

impl fmt::Display for HeosProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidCommand => "invalid HEOS command",
            Self::InvalidUtf8 => "HEOS line is not valid UTF-8",
            Self::EmptyLine => "HEOS line is empty",
        })
    }
}

impl std::error::Error for HeosProtocolError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_is_crlf_terminated() {
        let command = HeosCommand::new("heos://player/get_players").unwrap();
        assert_eq!(command.as_bytes(), b"heos://player/get_players\r\n");
    }

    #[test]
    fn command_requires_heos_scheme() {
        assert!(HeosCommand::new("player/get_players").is_err());
    }

    #[test]
    fn command_rejects_line_breaks() {
        assert!(matches!(
            HeosCommand::new("heos://player/get_players\n"),
            Err(HeosProtocolError::InvalidCommand)
        ));
    }

    #[test]
    fn parser_preserves_json_and_raw_lines() {
        assert_eq!(
            parse_heos_line(b"{\"heos\":{\"result\":\"success\"}}\r\n").unwrap(),
            HeosLine::Json("{\"heos\":{\"result\":\"success\"}}".into())
        );
        assert_eq!(
            parse_heos_line(b"event/player_state_changed\r\n").unwrap(),
            HeosLine::Raw("event/player_state_changed".into())
        );
    }

    #[test]
    fn parser_rejects_invalid_utf8_and_empty_lines() {
        assert!(matches!(
            parse_heos_line(&[0xff]),
            Err(HeosProtocolError::InvalidUtf8)
        ));
        assert!(matches!(
            parse_heos_line(b"\r\n"),
            Err(HeosProtocolError::EmptyLine)
        ));
    }
}
