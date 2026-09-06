//! Minimal persisted receiver identity configuration.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverIdentity {
    pub host: String,
    pub model: Option<String>,
    pub friendly_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub receiver: ReceiverIdentity,
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Invalid(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "configuration I/O failed: {error}"),
            Self::Invalid(message) => write!(f, "invalid configuration: {message}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn default_path() -> PathBuf {
    PathBuf::from("config/denon-avr-remote.yaml")
}

pub fn load(path: &Path) -> Result<Option<Config>, ConfigError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    parse(&text).map(Some)
}

pub fn save(path: &Path, config: &Config) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut text = format!("receiver:\n  host: {}\n", config.receiver.host);
    if let Some(model) = &config.receiver.model {
        text.push_str(&format!("  model: {}\n", model));
    }
    if let Some(name) = &config.receiver.friendly_name {
        text.push_str(&format!("  friendly_name: {}\n", name));
    }
    fs::write(path, text)?;
    Ok(())
}

pub fn parse(text: &str) -> Result<Config, ConfigError> {
    let mut host = None;
    let mut model = None;
    let mut friendly_name = None;
    let mut in_receiver = false;
    for (line_number, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "receiver:" {
            in_receiver = true;
            continue;
        }
        if !in_receiver {
            return Err(ConfigError::Invalid(format!(
                "line {} is outside receiver",
                line_number + 1
            )));
        }
        let Some((key, value)) = line.split_once(':') else {
            return Err(ConfigError::Invalid(format!(
                "line {} is not key/value",
                line_number + 1
            )));
        };
        let value = value.trim().trim_matches('"').trim_matches('\'');
        match key.trim() {
            "host" => host = Some(value.to_owned()),
            "model" => model = Some(value.to_owned()),
            "friendly_name" => friendly_name = Some(value.to_owned()),
            key => {
                return Err(ConfigError::Invalid(format!(
                    "unknown receiver field {key}"
                )))
            }
        }
    }
    let host = host
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ConfigError::Invalid("receiver.host is required".to_owned()))?;
    Ok(Config {
        receiver: ReceiverIdentity {
            host,
            model,
            friendly_name,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_documented_shape() {
        let config = parse(
            "receiver:\n  host: 192.0.2.10\n  model: AVR-X3800H\n  friendly_name: Living room\n",
        )
        .unwrap();
        assert_eq!(config.receiver.host, "192.0.2.10");
        assert_eq!(config.receiver.model.as_deref(), Some("AVR-X3800H"));
        assert_eq!(
            config.receiver.friendly_name.as_deref(),
            Some("Living room")
        );
    }
}
