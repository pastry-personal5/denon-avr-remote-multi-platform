//! YAML configuration adapter.
//!
//! The file holds several receivers by name, with a version field. Entry names
//! are receiver ids. The single-receiver file written by earlier releases is
//! still read, and is copied once to a backup before the first rewrite in the
//! new schema, which earlier releases cannot read.

use denon_avr_application::ports::{
    AsyncConfigRepository, BoxFuture, ConfigRepository, OperationError,
};
use denon_avr_domain::ConfiguredReceivers;
use files::{
    create_temporary_path, install_config_async, read_config, write_backup_async, write_config,
};
use schema::{configuration_error, decode_text, encode_text, schema_of, Schema};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

mod files;
mod schema;
#[cfg(test)]
mod tests;

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

pub(crate) fn default_config_path() -> PathBuf {
    crate::data_directory::data_directory().join("denon-avr-remote.yaml")
}

/// What a save writes, and the old file it must keep first.
struct SavePlan {
    text: String,
    /// The existing file's contents unless this release reads it as the current
    /// schema: the single-receiver file the first rewrite replaces, or a file
    /// this release cannot read. It is copied aside before anything is written.
    backup: Option<String>,
}

/// Whether `text` is a file this release reads in the current schema. Having the
/// current schema's keys is not enough: a later version, a misspelt key, or an
/// inconsistent entry makes it unreadable, and replacing it would lose it.
fn is_readable_current_schema(text: &str) -> bool {
    schema_of(text).ok() == Some(Schema::MultiReceiver) && decode_text(text).is_ok()
}

fn plan_save(
    existing: Option<String>,
    config: &ConfiguredReceivers,
) -> Result<SavePlan, OperationError> {
    let text = encode_text(config)?;
    // A blank file holds nothing worth keeping.
    let backup = existing.filter(|old| !old.trim().is_empty() && !is_readable_current_schema(old));
    Ok(SavePlan { text, backup })
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
