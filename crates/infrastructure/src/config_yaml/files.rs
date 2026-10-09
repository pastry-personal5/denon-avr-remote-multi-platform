//! Filesystem side of the YAML configuration adapter: reading the file, writing
//! it through a temporary file, installing it, and keeping backups beside it.

use super::plan_save;
use super::schema::{configuration_error, decode_text};
use denon_avr_application::ports::OperationError;
use denon_avr_domain::ConfiguredReceivers;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::warn;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn create_unique_path(path: &Path, suffix: &str) -> PathBuf {
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    path.with_extension(format!("yaml.{suffix}.{}.{}", std::process::id(), counter))
}

pub(super) fn create_temporary_path(path: &Path) -> PathBuf {
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

pub(super) async fn install_config_async(temporary: PathBuf, path: &Path) -> std::io::Result<()> {
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

pub(super) async fn write_backup_async(path: &Path, content: &str) -> std::io::Result<PathBuf> {
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

pub(super) fn read_config(path: &Path) -> Result<ConfiguredReceivers, OperationError> {
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

pub(super) fn write_config(
    path: &Path,
    config: &ConfiguredReceivers,
) -> Result<(), OperationError> {
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
