//! YAML configuration adapter retaining the v1 receiver shape.

use denon_avr_application::ports::{
    AsyncConfigRepository, BoxFuture, ConfigRepository, OperationError, OperationErrorKind,
};
use denon_avr_domain::{
    ConfiguredReceivers, ReceiverIdentity, SoundModeCategory, SoundModeFavorite,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone)]
pub struct YamlConfigRepository {
    path: PathBuf,
}
impl YamlConfigRepository {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}
impl Default for YamlConfigRepository {
    fn default() -> Self {
        Self::new(default_config_path())
    }
}

#[cfg(target_os = "macos")]
fn default_config_path() -> PathBuf {
    env_path("HOME")
        .unwrap_or_else(|| PathBuf::from("~"))
        .join("Library")
        .join("Application Support")
        .join("Denon AVR Remote")
        .join("denon-avr-remote.yaml")
}

#[cfg(not(target_os = "macos"))]
fn default_config_path() -> PathBuf {
    PathBuf::from("config/denon-avr-remote.yaml")
}

#[cfg(target_os = "macos")]
fn env_path(variable: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    receiver: ReceiverRecord,
    #[serde(default, skip_serializing_if = "SoundModeFavoritesFile::is_empty")]
    sound_mode_favorites: SoundModeFavoritesFile,
}

/// Older files keyed a favorite only by its detailed mode. As that format did
/// not retain the originating table row, migration assigns it to Movie (the
/// table's first group); the next save writes the explicit paired form.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum SoundModeFavoritesFile {
    Paired(BTreeMap<String, BTreeMap<String, BTreeSet<String>>>),
    Legacy(BTreeMap<String, BTreeSet<String>>),
}

impl Default for SoundModeFavoritesFile {
    fn default() -> Self {
        Self::Paired(BTreeMap::new())
    }
}

impl SoundModeFavoritesFile {
    fn is_empty(&self) -> bool {
        match self {
            Self::Paired(favorites) => favorites.is_empty(),
            Self::Legacy(favorites) => favorites.is_empty(),
        }
    }

    fn into_paired(self) -> BTreeMap<String, BTreeMap<String, BTreeSet<String>>> {
        match self {
            Self::Paired(favorites) => favorites,
            Self::Legacy(favorites) => favorites
                .into_iter()
                .map(|(receiver, modes)| (receiver, BTreeMap::from([("movie".into(), modes)])))
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
fn create_operation_error(
    kind: OperationErrorKind,
    context: &'static str,
    error: impl ToString,
) -> OperationError {
    OperationError::new(kind, context, error.to_string())
}
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn create_unique_path(path: &Path, suffix: &str) -> PathBuf {
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    path.with_extension(format!("yaml.{suffix}.{}.{}", std::process::id(), counter))
}

fn create_temporary_path(path: &Path) -> PathBuf {
    create_unique_path(path, "tmp")
}

#[cfg(windows)]
fn backup_path(path: &Path) -> PathBuf {
    create_unique_path(path, "bak")
}

fn install_config_sync(temporary: PathBuf, path: &Path) -> std::io::Result<()> {
    #[cfg(not(windows))]
    {
        let result = fs::rename(&temporary, path);
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    #[cfg(windows)]
    {
        let backup = backup_path(path);
        let had_existing = match fs::metadata(path) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                return Err(error);
            }
        };
        if had_existing {
            fs::rename(path, &backup)?;
        }
        match fs::rename(&temporary, path) {
            Ok(()) => {
                if had_existing {
                    let _ = fs::remove_file(backup);
                }
                Ok(())
            }
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                if had_existing {
                    if let Err(restore_error) = fs::rename(&backup, path) {
                        return Err(std::io::Error::new(
                            error.kind(),
                            format!("{error}; could not restore backup: {restore_error}"),
                        ));
                    }
                }
                Err(error)
            }
        }
    }
}

async fn install_config_async(temporary: PathBuf, path: &Path) -> std::io::Result<()> {
    #[cfg(not(windows))]
    {
        let result = tokio::fs::rename(&temporary, path).await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&temporary).await;
        }
        result
    }

    #[cfg(windows)]
    {
        let backup = backup_path(path);
        let had_existing = match tokio::fs::metadata(path).await {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                let _ = tokio::fs::remove_file(&temporary).await;
                return Err(error);
            }
        };
        if had_existing {
            tokio::fs::rename(path, &backup).await?;
        }
        match tokio::fs::rename(&temporary, path).await {
            Ok(()) => {
                if had_existing {
                    let _ = tokio::fs::remove_file(backup).await;
                }
                Ok(())
            }
            Err(error) => {
                let _ = tokio::fs::remove_file(&temporary).await;
                if had_existing {
                    if let Err(restore_error) = tokio::fs::rename(&backup, path).await {
                        return Err(std::io::Error::new(
                            error.kind(),
                            format!("{error}; could not restore backup: {restore_error}"),
                        ));
                    }
                }
                Err(error)
            }
        }
    }
}

fn optional_text(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty())
}

fn decode_config_file(file: ConfigFile) -> ConfiguredReceivers {
    let identity = ReceiverIdentity {
        host: file.receiver.host,
        model: optional_text(file.receiver.model),
        friendly_name: optional_text(file.receiver.friendly_name),
    };
    let name = identity
        .friendly_name
        .clone()
        .unwrap_or_else(|| "default".into());
    ConfiguredReceivers {
        current: Some(name.clone()),
        receivers: BTreeMap::from([(name, identity)]),
        sound_mode_favorites: file
            .sound_mode_favorites
            .into_paired()
            .into_iter()
            .map(|(receiver, categories)| {
                let favorites = categories
                    .into_iter()
                    .filter_map(|(category, modes)| {
                        SoundModeCategory::from_key(&category).map(|category| {
                            modes
                                .into_iter()
                                .filter_map(move |mode| SoundModeFavorite::new(category, mode).ok())
                        })
                    })
                    .flatten()
                    .collect();
                (receiver, favorites)
            })
            .collect(),
    }
}
fn encode_config_file(config: &ConfiguredReceivers) -> Result<ConfigFile, OperationError> {
    config.validate().map_err(|e| {
        create_operation_error(
            OperationErrorKind::Configuration,
            "validating configuration",
            e,
        )
    })?;
    if config.receivers.len() > 1 {
        return Err(create_operation_error(
            OperationErrorKind::Configuration,
            "encoding configuration",
            "legacy YAML supports exactly one receiver",
        ));
    }
    let identity = config
        .current()
        .map(|(_, identity)| identity)
        .or_else(|| config.receivers.values().next())
        .ok_or_else(|| {
            create_operation_error(
                OperationErrorKind::Configuration,
                "encoding configuration",
                "at least one receiver is required",
            )
        })?;
    // The legacy file can encode one receiver only. Its persisted name is
    // derived from the receiver record, so write that same key for its
    // favorites instead of retaining an in-memory alias.
    let receiver_name = identity
        .friendly_name
        .clone()
        .unwrap_or_else(|| "default".into());
    let paired_favorites = config
        .sound_mode_favorites
        .values()
        .next()
        .into_iter()
        .map(|favorites| {
            let categories = favorites.iter().fold(
                BTreeMap::<String, BTreeSet<String>>::new(),
                |mut categories, favorite| {
                    categories
                        .entry(favorite.category.key().into())
                        .or_default()
                        .insert(favorite.mode.clone());
                    categories
                },
            );
            (receiver_name.clone(), categories)
        })
        .collect();
    Ok(ConfigFile {
        receiver: ReceiverRecord {
            host: identity.host.clone(),
            model: optional_text(identity.model.clone()),
            friendly_name: optional_text(identity.friendly_name.clone()),
        },
        sound_mode_favorites: SoundModeFavoritesFile::Paired(paired_favorites),
    })
}
fn read_config(path: &Path) -> Result<ConfiguredReceivers, OperationError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ConfiguredReceivers::default())
        }
        Err(error) => {
            return Err(create_operation_error(
                OperationErrorKind::Configuration,
                "reading configuration",
                error,
            ))
        }
    };
    let file: ConfigFile = serde_yaml::from_str(&text).map_err(|e| {
        create_operation_error(
            OperationErrorKind::Configuration,
            "parsing configuration",
            e,
        )
    })?;
    let config = decode_config_file(file);
    config.validate().map_err(|e| {
        create_operation_error(
            OperationErrorKind::Configuration,
            "validating configuration",
            e,
        )
    })?;
    Ok(config)
}
fn write_config(path: &Path, config: &ConfiguredReceivers) -> Result<(), OperationError> {
    let file = encode_config_file(config)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            create_operation_error(
                OperationErrorKind::Configuration,
                "creating configuration directory",
                e,
            )
        })?;
    }
    let text = serde_yaml::to_string(&file).map_err(|e| {
        create_operation_error(
            OperationErrorKind::Configuration,
            "encoding configuration",
            e,
        )
    })?;
    let temporary = create_temporary_path(path);
    fs::write(&temporary, text).map_err(|e| {
        let _ = fs::remove_file(&temporary);
        create_operation_error(
            OperationErrorKind::Configuration,
            "writing temporary configuration",
            e,
        )
    })?;
    install_config_sync(temporary, path).map_err(|e| {
        create_operation_error(
            OperationErrorKind::Configuration,
            "installing configuration",
            e,
        )
    })
}
impl ConfigRepository for YamlConfigRepository {
    fn load(&self) -> Result<ConfiguredReceivers, OperationError> {
        read_config(&self.path)
    }
    fn save(&self, config: &ConfiguredReceivers) -> Result<(), OperationError> {
        write_config(&self.path, config)
    }
}
impl AsyncConfigRepository for YamlConfigRepository {
    fn load(&self) -> BoxFuture<'_, Result<ConfiguredReceivers, OperationError>> {
        Box::pin(async move {
            let text = match tokio::fs::read_to_string(&self.path).await {
                Ok(text) => text,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(ConfiguredReceivers::default())
                }
                Err(error) => {
                    return Err(create_operation_error(
                        OperationErrorKind::Configuration,
                        "reading configuration",
                        error,
                    ))
                }
            };
            let file: ConfigFile = serde_yaml::from_str(&text).map_err(|e| {
                create_operation_error(
                    OperationErrorKind::Configuration,
                    "parsing configuration",
                    e,
                )
            })?;
            let config = decode_config_file(file);
            config.validate().map_err(|e| {
                create_operation_error(
                    OperationErrorKind::Configuration,
                    "validating configuration",
                    e,
                )
            })?;
            Ok(config)
        })
    }
    fn save<'a>(
        &'a self,
        config: &'a ConfiguredReceivers,
    ) -> BoxFuture<'a, Result<(), OperationError>> {
        Box::pin(async move {
            let file = encode_config_file(config)?;
            if let Some(parent) = self.path.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(|e| {
                    create_operation_error(
                        OperationErrorKind::Configuration,
                        "creating configuration directory",
                        e,
                    )
                })?;
            }
            let text = serde_yaml::to_string(&file).map_err(|e| {
                create_operation_error(
                    OperationErrorKind::Configuration,
                    "encoding configuration",
                    e,
                )
            })?;
            let temporary = create_temporary_path(&self.path);
            tokio::fs::write(&temporary, text).await.map_err(|e| {
                let _ = std::fs::remove_file(&temporary);
                create_operation_error(
                    OperationErrorKind::Configuration,
                    "writing temporary configuration",
                    e,
                )
            })?;
            install_config_async(temporary, &self.path)
                .await
                .map_err(|e| {
                    create_operation_error(
                        OperationErrorKind::Configuration,
                        "installing configuration",
                        e,
                    )
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn path(suffix: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("denon-config-{stamp}-{suffix}.yaml"))
    }

    #[test]
    fn legacy_yaml_rejects_multiple_receivers_on_save() {
        let config = ConfiguredReceivers {
            current: Some("one".into()),
            receivers: BTreeMap::from([
                ("one".into(), ReceiverIdentity::ad_hoc("192.0.2.1")),
                ("two".into(), ReceiverIdentity::ad_hoc("192.0.2.2")),
            ]),
            ..ConfiguredReceivers::default()
        };
        let error =
            ConfigRepository::save(&YamlConfigRepository::new(path("multi")), &config).unwrap_err();
        assert!(error.to_string().contains("exactly one receiver"));
    }

    #[tokio::test]
    async fn async_load_rejects_the_same_invalid_identity_as_sync_load() {
        let path = path("invalid");
        fs::write(&path, "receiver:\n  host: \"\"\n").unwrap();
        let repository = YamlConfigRepository::new(&path);
        assert!(ConfigRepository::load(&repository).is_err());
        assert!(AsyncConfigRepository::load(&repository).await.is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sound_mode_favorites_round_trip_by_category_and_mode() {
        let path = path("sound-mode-favorites");
        let config = ConfiguredReceivers {
            current: Some("living-room".into()),
            receivers: BTreeMap::from([(
                "living-room".into(),
                ReceiverIdentity {
                    host: "192.0.2.10".into(),
                    model: Some("AVR-X3800H".into()),
                    friendly_name: Some("living-room".into()),
                },
            )]),
            sound_mode_favorites: BTreeMap::from([(
                "living-room".into(),
                BTreeSet::from([
                    SoundModeFavorite::new(SoundModeCategory::Movie, "DTS NEURAL:X").unwrap(),
                    SoundModeFavorite::new(SoundModeCategory::Music, "DTS NEURAL:X").unwrap(),
                ]),
            )]),
        };
        let repository = YamlConfigRepository::new(&path);

        ConfigRepository::save(&repository, &config).unwrap();
        let stored = fs::read_to_string(&path).unwrap();
        assert!(stored.contains("movie:\n    - DTS NEURAL:X"));
        assert!(stored.contains("music:\n    - DTS NEURAL:X"));
        assert_eq!(ConfigRepository::load(&repository).unwrap(), config);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn legacy_unpaired_favorites_migrate_without_preventing_startup() {
        let path = path("legacy-sound-mode-favorites");
        fs::write(
            &path,
            "receiver:\n  host: 192.0.2.10\nsound_mode_favorites:\n  default:\n    - DTS NEURAL:X\n",
        )
        .unwrap();

        let config = ConfigRepository::load(&YamlConfigRepository::new(&path)).unwrap();
        assert!(config.is_sound_mode_favorite(SoundModeCategory::Movie, "DTS NEURAL:X"));
        assert!(!config.is_sound_mode_favorite(SoundModeCategory::Music, "DTS NEURAL:X"));

        let _ = fs::remove_file(path);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn default_repository_keeps_repository_relative_development_path() {
        assert_eq!(
            YamlConfigRepository::default().path(),
            Path::new("config/denon-avr-remote.yaml")
        );
    }
}
