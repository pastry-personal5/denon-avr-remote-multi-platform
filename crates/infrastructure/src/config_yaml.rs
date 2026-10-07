//! YAML configuration adapter.
//!
//! The file holds several receivers by name, with a version field. Entry names
//! are receiver ids. The single-receiver file written by earlier releases is
//! still read, and is copied once to a backup before the first rewrite in the
//! new schema, which earlier releases cannot read.

use denon_avr_application::ports::{
    AsyncConfigRepository, BoxFuture, ConfigRepository, OperationError, OperationErrorKind,
};
use denon_avr_domain::{ConfiguredReceivers, ReceiverIdentity, SoundModeFavorite};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{debug, warn};

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

/// The schema version this release writes.
const CURRENT_VERSION: u32 = 2;

/// The single-receiver file written by earlier releases.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyConfigFile {
    receiver: ReceiverRecord,
    #[serde(default, skip_serializing_if = "SoundModeFavoritesFile::is_empty")]
    sound_mode_favorites: SoundModeFavoritesFile,
}

/// The multi-receiver file. Map keys are the receiver ids.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    current: Option<String>,
    receivers: BTreeMap<String, ReceiverRecord>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    sound_mode_favorites: BTreeMap<String, BTreeSet<String>>,
}

/// The canonical form is one detailed-mode list per receiver. Category-keyed
/// files written by the prior UI are accepted so no favorite is lost during
/// migration; their category labels are deliberately discarded.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum SoundModeFavoritesFile {
    CategoryKeyed(BTreeMap<String, BTreeMap<String, BTreeSet<String>>>),
    Simple(BTreeMap<String, BTreeSet<String>>),
}

impl Default for SoundModeFavoritesFile {
    fn default() -> Self {
        Self::Simple(BTreeMap::new())
    }
}

impl SoundModeFavoritesFile {
    fn is_empty(&self) -> bool {
        match self {
            Self::CategoryKeyed(favorites) => favorites.is_empty(),
            Self::Simple(favorites) => favorites.is_empty(),
        }
    }

    fn into_modes(self) -> BTreeMap<String, BTreeSet<String>> {
        match self {
            Self::CategoryKeyed(favorites) => favorites
                .into_iter()
                .map(|(receiver, categories)| {
                    (receiver, categories.into_values().flatten().collect())
                })
                .collect(),
            Self::Simple(favorites) => favorites,
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

fn configuration_error(context: &'static str, error: impl ToString) -> OperationError {
    create_operation_error(OperationErrorKind::Configuration, context, error)
}

fn identity_of(record: ReceiverRecord) -> ReceiverIdentity {
    ReceiverIdentity {
        host: record.host,
        model: optional_text(record.model),
        friendly_name: optional_text(record.friendly_name),
    }
}

fn favorites_of(
    favorites: BTreeMap<String, BTreeSet<String>>,
) -> BTreeMap<String, BTreeSet<SoundModeFavorite>> {
    favorites
        .into_iter()
        .map(|(receiver, modes)| {
            let favorites = modes
                .into_iter()
                .filter_map(|mode| SoundModeFavorite::new(mode).ok())
                .collect();
            (receiver, favorites)
        })
        .collect()
}

/// Which schema a file is in, told by its top-level key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Schema {
    /// One receiver, under `receiver`.
    SingleReceiver,
    /// Several receivers, with a `version`.
    MultiReceiver,
}

fn schema_of(text: &str) -> Result<Schema, OperationError> {
    let value: serde_yaml::Value = serde_yaml::from_str(text)
        .map_err(|error| configuration_error("parsing configuration", error))?;
    let mapping = value.as_mapping().ok_or_else(|| {
        configuration_error("parsing configuration", "the file is not a YAML mapping")
    })?;
    match (
        mapping.contains_key("version"),
        mapping.contains_key("receiver"),
    ) {
        (true, false) => Ok(Schema::MultiReceiver),
        (false, true) => Ok(Schema::SingleReceiver),
        (true, true) => Err(configuration_error(
            "parsing configuration",
            "the file has both `version` and `receiver`",
        )),
        (false, false) => Err(configuration_error(
            "parsing configuration",
            "the file has neither `version` nor `receiver`",
        )),
    }
}

/// Decode either schema. The single-receiver file becomes one entry named by
/// its friendly name, or `default` when it has none, as it always has.
fn decode_text(text: &str) -> Result<ConfiguredReceivers, OperationError> {
    let config = match schema_of(text)? {
        Schema::SingleReceiver => {
            let file: LegacyConfigFile = serde_yaml::from_str(text)
                .map_err(|error| configuration_error("parsing configuration", error))?;
            let identity = identity_of(file.receiver);
            let name = identity
                .friendly_name
                .clone()
                .unwrap_or_else(|| "default".into());
            ConfiguredReceivers {
                current: Some(name.clone()),
                receivers: BTreeMap::from([(name, identity)]),
                sound_mode_favorites: favorites_of(file.sound_mode_favorites.into_modes()),
            }
        }
        Schema::MultiReceiver => {
            // A duplicate entry name is a parse error, not a silent overwrite.
            let file: ConfigFile = serde_yaml::from_str(text)
                .map_err(|error| configuration_error("parsing configuration", error))?;
            if file.version != CURRENT_VERSION {
                return Err(configuration_error(
                    "parsing configuration",
                    format!(
                        "configuration version {} is not supported; this release reads version {CURRENT_VERSION}",
                        file.version
                    ),
                ));
            }
            ConfiguredReceivers {
                current: file.current,
                receivers: file
                    .receivers
                    .into_iter()
                    .map(|(name, record)| (name, identity_of(record)))
                    .collect(),
                sound_mode_favorites: favorites_of(file.sound_mode_favorites),
            }
        }
    };
    config
        .validate()
        .map_err(|error| configuration_error("validating configuration", error))?;
    Ok(config)
}

fn encode_text(config: &ConfiguredReceivers) -> Result<String, OperationError> {
    config
        .validate()
        .map_err(|error| configuration_error("validating configuration", error))?;
    let file = ConfigFile {
        version: CURRENT_VERSION,
        current: config.current.clone(),
        receivers: config
            .receivers
            .iter()
            .map(|(name, identity)| {
                (
                    name.clone(),
                    ReceiverRecord {
                        host: identity.host.clone(),
                        model: optional_text(identity.model.clone()),
                        friendly_name: optional_text(identity.friendly_name.clone()),
                    },
                )
            })
            .collect(),
        sound_mode_favorites: config
            .sound_mode_favorites
            .iter()
            .map(|(receiver, favorites)| {
                (
                    receiver.clone(),
                    favorites
                        .iter()
                        .map(|favorite| favorite.mode.clone())
                        .collect(),
                )
            })
            .collect(),
    };
    serde_yaml::to_string(&file)
        .map_err(|error| configuration_error("encoding configuration", error))
}

/// What a save writes, and the old file it must keep first.
struct SavePlan {
    text: String,
    /// The existing file's contents when it is not already in the current
    /// schema: the single-receiver file the first rewrite replaces, or a file
    /// this release cannot read. It is copied aside before anything is written.
    backup: Option<String>,
}

fn plan_save(
    existing: Option<String>,
    config: &ConfiguredReceivers,
) -> Result<SavePlan, OperationError> {
    let text = encode_text(config)?;
    let backup = existing.filter(|old| schema_of(old).ok() != Some(Schema::MultiReceiver));
    Ok(SavePlan { text, backup })
}

/// `<file>.v3.bak`, then `<file>.v3.bak.1`, and so on. An existing backup is
/// never overwritten.
fn backup_candidate(path: &Path, attempt: u32) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".v3.bak");
    if attempt > 0 {
        name.push(format!(".{attempt}"));
    }
    path.with_file_name(name)
}

const MAX_BACKUPS: u32 = 1000;

fn write_backup_sync(path: &Path, content: &str) -> std::io::Result<PathBuf> {
    use std::io::Write;
    for attempt in 0..MAX_BACKUPS {
        let candidate = backup_candidate(path, attempt);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(mut file) => {
                file.write_all(content.as_bytes())?;
                file.sync_all()?;
                return Ok(candidate);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "too many configuration backups exist",
    ))
}

async fn write_backup_async(path: &Path, content: &str) -> std::io::Result<PathBuf> {
    use tokio::io::AsyncWriteExt;
    for attempt in 0..MAX_BACKUPS {
        let candidate = backup_candidate(path, attempt);
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
            .await
        {
            Ok(mut file) => {
                file.write_all(content.as_bytes()).await?;
                file.sync_all().await?;
                return Ok(candidate);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "too many configuration backups exist",
    ))
}

fn read_config(path: &Path) -> Result<ConfiguredReceivers, OperationError> {
    match fs::read_to_string(path) {
        Ok(text) => decode_text(&text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(ConfiguredReceivers::default())
        }
        Err(error) => Err(configuration_error("reading configuration", error)),
    }
}

fn read_existing_sync(path: &Path) -> Result<Option<String>, OperationError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(configuration_error("reading existing configuration", error)),
    }
}

fn write_config(path: &Path, config: &ConfiguredReceivers) -> Result<(), OperationError> {
    let plan = plan_save(read_existing_sync(path)?, config)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| configuration_error("creating configuration directory", e))?;
    }
    // The old file is kept before it is replaced; if that fails, nothing is.
    if let Some(old) = &plan.backup {
        let kept = write_backup_sync(path, old)
            .map_err(|e| configuration_error("backing up the earlier configuration", e))?;
        warn!(backup = %kept.display(), "kept the earlier configuration file before rewriting it");
    }
    let temporary = create_temporary_path(path);
    fs::write(&temporary, plan.text).map_err(|e| {
        let _ = fs::remove_file(&temporary);
        configuration_error("writing temporary configuration", e)
    })?;
    install_config_sync(temporary, path)
        .map_err(|e| configuration_error("installing configuration", e))
}

impl ConfigRepository for YamlConfigRepository {
    fn load(&self) -> Result<ConfiguredReceivers, OperationError> {
        let result = read_config(&self.path);
        match &result {
            Ok(config) => {
                debug!(path = %self.path.display(), receivers = config.receivers.len(), "configuration loaded")
            }
            Err(error) => {
                warn!(path = %self.path.display(), error = %error, "configuration load failed")
            }
        }
        result
    }
    fn save(&self, config: &ConfiguredReceivers) -> Result<(), OperationError> {
        let result = write_config(&self.path, config);
        match &result {
            Ok(()) => {
                debug!(path = %self.path.display(), receivers = config.receivers.len(), "configuration saved")
            }
            Err(error) => {
                warn!(path = %self.path.display(), error = %error, "configuration save failed")
            }
        }
        result
    }
}

impl AsyncConfigRepository for YamlConfigRepository {
    fn load(&self) -> BoxFuture<'_, Result<ConfiguredReceivers, OperationError>> {
        Box::pin(async move {
            debug!(path = %self.path.display(), "asynchronous configuration load started");
            match tokio::fs::read_to_string(&self.path).await {
                Ok(text) => decode_text(&text),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Ok(ConfiguredReceivers::default())
                }
                Err(error) => Err(configuration_error("reading configuration", error)),
            }
        })
    }
    fn save<'a>(
        &'a self,
        config: &'a ConfiguredReceivers,
    ) -> BoxFuture<'a, Result<(), OperationError>> {
        Box::pin(async move {
            debug!(path = %self.path.display(), receivers = config.receivers.len(), "asynchronous configuration save started");
            let existing = match tokio::fs::read_to_string(&self.path).await {
                Ok(text) => Some(text),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(configuration_error("reading existing configuration", error))
                }
            };
            let plan = plan_save(existing, config)?;
            if let Some(parent) = self.path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| configuration_error("creating configuration directory", e))?;
            }
            if let Some(old) = &plan.backup {
                let kept = write_backup_async(&self.path, old)
                    .await
                    .map_err(|e| configuration_error("backing up the earlier configuration", e))?;
                warn!(backup = %kept.display(), "kept the earlier configuration file before rewriting it");
            }
            let temporary = create_temporary_path(&self.path);
            tokio::fs::write(&temporary, plan.text).await.map_err(|e| {
                let _ = std::fs::remove_file(&temporary);
                configuration_error("writing temporary configuration", e)
            })?;
            install_config_async(temporary, &self.path)
                .await
                .map_err(|e| configuration_error("installing configuration", e))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A scratch directory per test, so backups beside the file are visible.
    struct Scratch {
        dir: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!("denon-config-{stamp}-{name}"));
            fs::create_dir_all(&dir).unwrap();
            Self { dir }
        }
        fn file(&self) -> PathBuf {
            self.dir.join("denon-avr-remote.yaml")
        }
        fn backups(&self) -> Vec<String> {
            let mut names: Vec<_> = fs::read_dir(&self.dir)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| name.contains(".v3.bak"))
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    const SINGLE_RECEIVER: &str = "receiver:\n  host: 192.0.2.10\n  model: AVR-X3800H\n  friendly_name: living-room\nsound_mode_favorites:\n  living-room:\n    - DTS NEURAL:X\n";

    fn two_receivers() -> ConfiguredReceivers {
        ConfiguredReceivers {
            current: Some("living-room".into()),
            receivers: BTreeMap::from([
                (
                    "living-room".into(),
                    ReceiverIdentity {
                        host: "192.0.2.10".into(),
                        model: Some("AVR-X3800H".into()),
                        friendly_name: Some("Living Room".into()),
                    },
                ),
                ("den".into(), ReceiverIdentity::ad_hoc("192.0.2.11")),
            ]),
            sound_mode_favorites: BTreeMap::from([(
                "living-room".into(),
                BTreeSet::from([
                    SoundModeFavorite::new("DTS NEURAL:X").unwrap(),
                    SoundModeFavorite::new("DOLBY SURROUND").unwrap(),
                ]),
            )]),
        }
    }

    fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn the_multi_receiver_schema_round_trips_through_both_repositories() {
        let scratch = Scratch::new("round-trip");
        let repository = YamlConfigRepository::new(scratch.file());
        let config = two_receivers();

        ConfigRepository::save(&repository, &config).unwrap();
        let stored = fs::read_to_string(scratch.file()).unwrap();
        assert!(stored.starts_with("version: 2\n"), "{stored}");
        assert!(stored.contains("current: living-room"));
        assert!(stored.contains("den:"));
        assert!(stored.contains("DOLBY SURROUND") && stored.contains("DTS NEURAL:X"));
        assert_eq!(ConfigRepository::load(&repository).unwrap(), config);
        assert_eq!(
            block_on(AsyncConfigRepository::load(&repository)).unwrap(),
            config
        );

        // The asynchronous save writes the same bytes.
        let other = Scratch::new("round-trip-async");
        let async_repository = YamlConfigRepository::new(other.file());
        block_on(AsyncConfigRepository::save(&async_repository, &config)).unwrap();
        assert_eq!(fs::read_to_string(other.file()).unwrap(), stored);
    }

    #[test]
    fn an_empty_configuration_can_be_saved_and_a_missing_file_is_empty() {
        let scratch = Scratch::new("empty");
        let repository = YamlConfigRepository::new(scratch.file());
        assert_eq!(
            ConfigRepository::load(&repository).unwrap(),
            ConfiguredReceivers::default()
        );
        ConfigRepository::save(&repository, &ConfiguredReceivers::default()).unwrap();
        assert_eq!(
            ConfigRepository::load(&repository).unwrap(),
            ConfiguredReceivers::default()
        );
    }

    #[test]
    fn the_single_receiver_file_is_read_as_one_entry_named_by_its_friendly_name() {
        let scratch = Scratch::new("single");
        fs::write(scratch.file(), SINGLE_RECEIVER).unwrap();
        let repository = YamlConfigRepository::new(scratch.file());
        let config = ConfigRepository::load(&repository).unwrap();
        assert_eq!(config.current.as_deref(), Some("living-room"));
        assert_eq!(config.receivers.len(), 1);
        assert_eq!(config.receivers["living-room"].host, "192.0.2.10");
        assert!(config.is_sound_mode_favorite("DTS NEURAL:X"));
        // Reading does not rewrite anything.
        assert_eq!(fs::read_to_string(scratch.file()).unwrap(), SINGLE_RECEIVER);
        assert!(scratch.backups().is_empty());
    }

    #[test]
    fn an_unnamed_single_receiver_is_named_default() {
        let scratch = Scratch::new("unnamed");
        fs::write(scratch.file(), "receiver:\n  host: 192.0.2.10\n").unwrap();
        let config = ConfigRepository::load(&YamlConfigRepository::new(scratch.file())).unwrap();
        assert_eq!(config.current.as_deref(), Some("default"));
        assert!(config.receivers.contains_key("default"));
    }

    #[test]
    fn the_first_rewrite_keeps_the_old_file_once_and_later_saves_add_no_backup() {
        let scratch = Scratch::new("backup-once");
        fs::write(scratch.file(), SINGLE_RECEIVER).unwrap();
        let repository = YamlConfigRepository::new(scratch.file());
        let mut config = ConfigRepository::load(&repository).unwrap();

        ConfigRepository::save(&repository, &config).unwrap();
        assert_eq!(scratch.backups(), vec!["denon-avr-remote.yaml.v3.bak"]);
        let backup = scratch.dir.join("denon-avr-remote.yaml.v3.bak");
        assert_eq!(fs::read_to_string(&backup).unwrap(), SINGLE_RECEIVER);
        assert!(fs::read_to_string(scratch.file())
            .unwrap()
            .starts_with("version: 2\n"));

        // Later saves find the new schema and keep nothing more.
        config
            .receivers
            .insert("den".into(), ReceiverIdentity::ad_hoc("192.0.2.11"));
        ConfigRepository::save(&repository, &config).unwrap();
        block_on(AsyncConfigRepository::save(&repository, &config)).unwrap();
        assert_eq!(scratch.backups(), vec!["denon-avr-remote.yaml.v3.bak"]);
        assert_eq!(fs::read_to_string(&backup).unwrap(), SINGLE_RECEIVER);
    }

    #[test]
    fn the_asynchronous_save_also_keeps_the_old_file_once() {
        let scratch = Scratch::new("backup-async");
        fs::write(scratch.file(), SINGLE_RECEIVER).unwrap();
        let repository = YamlConfigRepository::new(scratch.file());
        let config = block_on(AsyncConfigRepository::load(&repository)).unwrap();
        block_on(AsyncConfigRepository::save(&repository, &config)).unwrap();
        block_on(AsyncConfigRepository::save(&repository, &config)).unwrap();
        assert_eq!(scratch.backups(), vec!["denon-avr-remote.yaml.v3.bak"]);
        assert_eq!(
            fs::read_to_string(scratch.dir.join("denon-avr-remote.yaml.v3.bak")).unwrap(),
            SINGLE_RECEIVER
        );
    }

    #[test]
    fn an_existing_backup_is_never_overwritten() {
        let scratch = Scratch::new("backup-kept");
        let earlier = scratch.dir.join("denon-avr-remote.yaml.v3.bak");
        fs::write(&earlier, "an earlier backup").unwrap();
        fs::write(scratch.file(), SINGLE_RECEIVER).unwrap();
        let repository = YamlConfigRepository::new(scratch.file());
        let config = ConfigRepository::load(&repository).unwrap();

        ConfigRepository::save(&repository, &config).unwrap();

        assert_eq!(fs::read_to_string(&earlier).unwrap(), "an earlier backup");
        assert_eq!(
            scratch.backups(),
            vec![
                "denon-avr-remote.yaml.v3.bak",
                "denon-avr-remote.yaml.v3.bak.1"
            ]
        );
        assert_eq!(
            fs::read_to_string(scratch.dir.join("denon-avr-remote.yaml.v3.bak.1")).unwrap(),
            SINGLE_RECEIVER
        );
    }

    #[test]
    fn a_new_file_needs_no_backup() {
        let scratch = Scratch::new("no-backup");
        let repository = YamlConfigRepository::new(scratch.file());
        ConfigRepository::save(&repository, &two_receivers()).unwrap();
        assert!(scratch.backups().is_empty());
    }

    #[test]
    fn a_file_this_release_cannot_read_is_kept_before_it_is_replaced() {
        let scratch = Scratch::new("unreadable");
        fs::write(scratch.file(), "this is not a configuration").unwrap();
        let repository = YamlConfigRepository::new(scratch.file());
        ConfigRepository::save(&repository, &two_receivers()).unwrap();
        assert_eq!(
            fs::read_to_string(scratch.dir.join("denon-avr-remote.yaml.v3.bak")).unwrap(),
            "this is not a configuration"
        );
    }

    #[test]
    fn a_rejected_save_leaves_the_old_file_and_makes_no_backup() {
        let scratch = Scratch::new("rejected-save");
        fs::write(scratch.file(), SINGLE_RECEIVER).unwrap();
        let repository = YamlConfigRepository::new(scratch.file());
        let mut config = ConfigRepository::load(&repository).unwrap();
        config.current = Some("missing".into());

        assert!(ConfigRepository::save(&repository, &config).is_err());
        assert!(block_on(AsyncConfigRepository::save(&repository, &config)).is_err());
        assert_eq!(fs::read_to_string(scratch.file()).unwrap(), SINGLE_RECEIVER);
        assert!(scratch.backups().is_empty());
    }

    fn assert_rejected_by_both(text: &str, expected: &str) {
        let scratch = Scratch::new("rejected-read");
        fs::write(scratch.file(), text).unwrap();
        let repository = YamlConfigRepository::new(scratch.file());
        let sync = ConfigRepository::load(&repository).unwrap_err();
        let asynchronous = block_on(AsyncConfigRepository::load(&repository)).unwrap_err();
        assert!(sync.to_string().contains(expected), "{sync} for {text:?}");
        assert_eq!(sync, asynchronous, "both repositories reject alike");
    }

    #[test]
    fn duplicate_receiver_names_are_rejected_not_merged() {
        assert_rejected_by_both(
            "version: 2\ncurrent: den\nreceivers:\n  den:\n    host: 192.0.2.1\n  den:\n    host: 192.0.2.2\n",
            "duplicate",
        );
    }

    #[test]
    fn duplicate_top_level_keys_are_rejected() {
        assert_rejected_by_both(
            "version: 2\nreceivers:\n  den:\n    host: 192.0.2.1\nreceivers:\n  den:\n    host: 192.0.2.2\n",
            "duplicate",
        );
    }

    #[test]
    fn reserved_and_empty_names_are_rejected_on_read_and_on_save() {
        assert_rejected_by_both(
            "version: 2\nreceivers:\n  \"adhoc:192.0.2.1\":\n    host: 192.0.2.1\n",
            "reserved",
        );
        assert_rejected_by_both(
            "version: 2\nreceivers:\n  \"\":\n    host: 192.0.2.1\n",
            "must not be empty",
        );
        let scratch = Scratch::new("reserved-save");
        let repository = YamlConfigRepository::new(scratch.file());
        let reserved = ConfiguredReceivers {
            current: None,
            receivers: BTreeMap::from([(
                "adhoc:192.0.2.1".into(),
                ReceiverIdentity::ad_hoc("192.0.2.1"),
            )]),
            ..ConfiguredReceivers::default()
        };
        let error = ConfigRepository::save(&repository, &reserved).unwrap_err();
        assert!(error.to_string().contains("reserved"), "{error}");
        assert!(!scratch.file().exists());
    }

    #[test]
    fn other_versions_and_inconsistent_files_are_rejected() {
        let receivers = "receivers:\n  den:\n    host: 192.0.2.1\n";
        assert_rejected_by_both(
            &format!("version: 3\n{receivers}"),
            "version 3 is not supported",
        );
        assert_rejected_by_both(
            &format!("version: 1\n{receivers}"),
            "version 1 is not supported",
        );
        assert_rejected_by_both(
            &format!("version: 2\ncurrent: gone\n{receivers}"),
            "current receiver gone is not configured",
        );
        assert_rejected_by_both(
            &format!("version: 2\n{receivers}sound_mode_favorites:\n  gone:\n    - STEREO\n"),
            "unknown receiver gone",
        );
        assert_rejected_by_both(
            &format!("version: 2\nreceiver:\n  host: 192.0.2.1\n{receivers}"),
            "both `version` and `receiver`",
        );
        assert_rejected_by_both("hosts: []\n", "neither `version` nor `receiver`");
        assert_rejected_by_both("receiver:\n  host: \"\"\n", "empty host");
    }

    #[test]
    fn an_earlier_release_cannot_read_the_new_file() {
        let scratch = Scratch::new("old-release");
        let repository = YamlConfigRepository::new(scratch.file());
        ConfigRepository::save(&repository, &two_receivers()).unwrap();
        let stored = fs::read_to_string(scratch.file()).unwrap();
        // Version 3.0.0 parsed exactly this shape and refused anything else.
        assert!(serde_yaml::from_str::<LegacyConfigFile>(&stored).is_err());
    }

    #[test]
    fn category_keyed_favorites_flatten_without_discarding_modes() {
        let scratch = Scratch::new("category-keyed");
        fs::write(
            scratch.file(),
            "receiver:\n  host: 192.0.2.10\nsound_mode_favorites:\n  default:\n    movie:\n      - DTS NEURAL:X\n      - DOLBY SURROUND\n    music:\n      - DTS NEURAL:X\n",
        )
        .unwrap();
        let repository = YamlConfigRepository::new(scratch.file());
        let config = ConfigRepository::load(&repository).unwrap();
        assert!(config.is_sound_mode_favorite("DTS NEURAL:X"));
        assert!(config.is_sound_mode_favorite("DOLBY SURROUND"));
        assert_eq!(config.sound_mode_favorites["default"].len(), 2);

        ConfigRepository::save(&repository, &config).unwrap();
        let stored = fs::read_to_string(scratch.file()).unwrap();
        assert!(stored.contains("DOLBY SURROUND") && stored.contains("DTS NEURAL:X"));
        assert!(!stored.contains("movie:"));
        assert_eq!(ConfigRepository::load(&repository).unwrap(), config);
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
