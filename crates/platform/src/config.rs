use denon_avr_core::{
    AsyncConfigRepository, BoxFuture, ConfigRepository, ConfiguredReceivers, OperationError,
    OperationErrorKind, ReceiverIdentity,
};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct TomlConfigRepository {
    path: PathBuf,
}

impl TomlConfigRepository {
    pub fn platform_default() -> Result<Self, OperationError> {
        let directories = ProjectDirs::from("", "", "denon-avr-remote").ok_or_else(|| {
            OperationError::new(
                OperationErrorKind::Configuration,
                "resolving configuration path",
                "the operating system did not provide a configuration directory",
            )
        })?;
        Ok(Self::new(directories.config_dir().join("config.toml")))
    }

    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn decode(text: &str) -> Result<ConfiguredReceivers, OperationError> {
        let file: ConfigFile = toml::from_str(text).map_err(|error| {
            OperationError::new(
                OperationErrorKind::Configuration,
                "parsing configuration",
                error.to_string(),
            )
        })?;
        let config = file.into_domain();
        config.validate().map_err(|message| {
            OperationError::new(
                OperationErrorKind::Configuration,
                "validating configuration",
                message,
            )
        })?;
        Ok(config)
    }

    fn encode(config: &ConfiguredReceivers) -> Result<String, OperationError> {
        config.validate().map_err(|message| {
            OperationError::new(
                OperationErrorKind::Configuration,
                "validating configuration",
                message,
            )
        })?;
        toml::to_string_pretty(&ConfigFile::from_domain(config)).map_err(|error| {
            OperationError::new(
                OperationErrorKind::Configuration,
                "encoding configuration",
                error.to_string(),
            )
        })
    }
}

impl ConfigRepository for TomlConfigRepository {
    fn load(&self) -> Result<ConfiguredReceivers, OperationError> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ConfiguredReceivers::default())
            }
            Err(error) => return Err(io_error("reading configuration", error)),
        };
        Self::decode(&text)
    }

    fn save(&self, config: &ConfiguredReceivers) -> Result<(), OperationError> {
        let text = Self::encode(config)?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| io_error("creating configuration directory", error))?;
        }
        let temporary = temporary_path(&self.path);
        fs::write(&temporary, text)
            .map_err(|error| io_error("writing temporary configuration", error))?;
        replace_file(&temporary, &self.path)
    }
}

impl AsyncConfigRepository for TomlConfigRepository {
    fn load(&self) -> BoxFuture<'_, Result<ConfiguredReceivers, OperationError>> {
        Box::pin(async move {
            let text = match tokio::fs::read_to_string(&self.path).await {
                Ok(text) => text,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(ConfiguredReceivers::default())
                }
                Err(error) => return Err(io_error("reading configuration", error)),
            };
            Self::decode(&text)
        })
    }

    fn save<'a>(
        &'a self,
        config: &'a ConfiguredReceivers,
    ) -> BoxFuture<'a, Result<(), OperationError>> {
        Box::pin(async move {
            let text = Self::encode(config)?;
            if let Some(parent) = self.path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|error| io_error("creating configuration directory", error))?;
            }
            let temporary = temporary_path(&self.path);
            tokio::fs::write(&temporary, text)
                .await
                .map_err(|error| io_error("writing temporary configuration", error))?;
            if tokio::fs::try_exists(&self.path)
                .await
                .map_err(|error| io_error("checking configuration destination", error))?
            {
                tokio::fs::remove_file(&self.path)
                    .await
                    .map_err(|error| io_error("replacing configuration", error))?;
            }
            tokio::fs::rename(&temporary, &self.path)
                .await
                .map_err(|error| io_error("installing configuration", error))
        })
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    path.with_extension("toml.tmp")
}

fn replace_file(temporary: &Path, destination: &Path) -> Result<(), OperationError> {
    if destination.exists() {
        fs::remove_file(destination).map_err(|error| io_error("replacing configuration", error))?;
    }
    fs::rename(temporary, destination).map_err(|error| io_error("installing configuration", error))
}

fn io_error(context: &'static str, error: std::io::Error) -> OperationError {
    OperationError::new(
        OperationErrorKind::Configuration,
        context,
        error.to_string(),
    )
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    current: Option<String>,
    #[serde(default)]
    receivers: BTreeMap<String, ReceiverRecord>,
}

impl ConfigFile {
    fn from_domain(config: &ConfiguredReceivers) -> Self {
        Self {
            current: config.current.clone(),
            receivers: config
                .receivers
                .iter()
                .map(|(name, receiver)| (name.clone(), ReceiverRecord::from(receiver)))
                .collect(),
        }
    }

    fn into_domain(self) -> ConfiguredReceivers {
        ConfiguredReceivers {
            current: self.current,
            receivers: self
                .receivers
                .into_iter()
                .map(|(name, receiver)| (name, receiver.into()))
                .collect(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiverRecord {
    host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    friendly_name: Option<String>,
}

impl From<&ReceiverIdentity> for ReceiverRecord {
    fn from(value: &ReceiverIdentity) -> Self {
        Self {
            host: value.host.clone(),
            model: value.model.clone(),
            friendly_name: value.friendly_name.clone(),
        }
    }
}

impl From<ReceiverRecord> for ReceiverIdentity {
    fn from(value: ReceiverRecord) -> Self {
        Self {
            host: value.host,
            model: value.model,
            friendly_name: value.friendly_name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ConfiguredReceivers {
        ConfiguredReceivers {
            current: Some("living-room".into()),
            receivers: BTreeMap::from([(
                "living-room".into(),
                ReceiverIdentity {
                    host: "192.0.2.10".into(),
                    model: Some("AVR-X3800H".into()),
                    friendly_name: Some("Living room".into()),
                },
            )]),
        }
    }

    #[test]
    fn toml_round_trip_preserves_multiple_receiver_shape() {
        let encoded = TomlConfigRepository::encode(&sample()).unwrap();
        assert_eq!(TomlConfigRepository::decode(&encoded).unwrap(), sample());
    }

    #[tokio::test]
    async fn sync_and_async_repositories_have_feature_parity() {
        let path = std::env::temp_dir().join(format!(
            "denon-avr-config-{}-{}.toml",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let repository = TomlConfigRepository::new(&path);
        AsyncConfigRepository::save(&repository, &sample())
            .await
            .unwrap();
        assert_eq!(ConfigRepository::load(&repository).unwrap(), sample());
        let _ = fs::remove_file(path);
    }
}
