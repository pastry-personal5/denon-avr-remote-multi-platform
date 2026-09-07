use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeosCommand(String);

impl HeosCommand {
    pub fn new(command: impl Into<String>) -> Result<Self, HeosProtocolError> {
        let command = command.into();
        if !command.starts_with("heos://") {
            return Err(HeosProtocolError::InvalidCommand(
                "HEOS command must start with heos://",
            ));
        }
        if command.contains(['\r', '\n']) {
            return Err(HeosProtocolError::InvalidCommand(
                "HEOS command contains a line break",
            ));
        }
        Ok(Self(command))
    }

    pub fn as_str(&self) -> &str {
        &self.0
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
}

pub fn parse_line(line: &[u8]) -> Result<HeosLine, HeosProtocolError> {
    let text = std::str::from_utf8(line).map_err(|_| HeosProtocolError::InvalidUtf8)?;
    let text = text.strip_suffix("\r\n").unwrap_or(text);
    if text.is_empty() {
        return Err(HeosProtocolError::EmptyLine);
    }
    Ok(HeosLine::Json(text.into()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeosProtocolError {
    InvalidCommand(&'static str),
    InvalidUtf8,
    EmptyLine,
}

impl fmt::Display for HeosProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCommand(message) => f.write_str(message),
            Self::InvalidUtf8 => f.write_str("HEOS line is not valid UTF-8"),
            Self::EmptyLine => f.write_str("HEOS line is empty"),
        }
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
}
