//! Minimal persisted receiver identity configuration.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiverIdentity {
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub friendly_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub receiver: ReceiverIdentity,
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(String),
    Invalid(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "configuration I/O failed: {error}"),
            Self::Parse(message) => write!(f, "configuration YAML is invalid: {message}"),
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
    validate(config)?;
    let text = serde_yaml::to_string(config).map_err(|e| ConfigError::Parse(e.to_string()))?;
    let temporary = path.with_extension("yaml.tmp");
    fs::write(&temporary, text)?;
    // Rename is atomic on the target filesystem. Windows cannot replace an
    // existing file with rename, so remove only this exact destination first.
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)?;
    Ok(())
}

pub fn parse(text: &str) -> Result<Config, ConfigError> {
    let config: Config =
        serde_yaml::from_str(text).map_err(|e| ConfigError::Parse(e.to_string()))?;
    validate(&config)?;
    Ok(config)
}

fn validate(config: &Config) -> Result<(), ConfigError> {
    if config.receiver.host.trim().is_empty() {
        return Err(ConfigError::Invalid("receiver.host is required".into()));
    }
    if config.receiver.host.contains(['\r', '\n']) {
        return Err(ConfigError::Invalid(
            "receiver.host contains a line break".into(),
        ));
    }
    Ok(())
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

    #[test]
    fn rejects_unknown_fields_and_missing_host() {
        assert!(matches!(
            parse("receiver:\n  host: 192.0.2.10\n  port: 23\n"),
            Err(ConfigError::Parse(_))
        ));
        assert!(matches!(
            parse("receiver:\n  model: AVR-X3800H\n"),
            Err(ConfigError::Invalid(_)) | Err(ConfigError::Parse(_))
        ));
    }

    #[test]
    fn serde_round_trip_preserves_optional_identity_fields() {
        let config = Config {
            receiver: ReceiverIdentity {
                host: "192.0.2.10".into(),
                model: Some("AVR-X3800H".into()),
                friendly_name: Some("Living room".into()),
            },
        };
        let encoded = serde_yaml::to_string(&config).unwrap();
        assert_eq!(parse(&encoded).unwrap(), config);
    }
}
