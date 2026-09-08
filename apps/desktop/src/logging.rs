//! Desktop-owned, file-backed logging configuration.
//!
//! All application crates emit through the `tracing` facade.  This module is
//! the one composition-root place that decides where those events go and keeps
//! the asynchronous writer alive for the lifetime of the desktop process.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

const LOG_FILE_NAME: &str = "denon-avr-remote.log";
const MAX_ROTATED_LOG_FILES: usize = 14;

pub struct LoggingGuard {
    _worker_guard: WorkerGuard,
    directory: PathBuf,
}

impl LoggingGuard {
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

/// Configures daily file rotation and keeps the newest fourteen rotated files.
///
/// `RUST_LOG` controls filtering when set; otherwise the desktop records
/// `info` and higher events. The returned guard must outlive the application.
pub fn initialize() -> Result<LoggingGuard, Box<dyn std::error::Error>> {
    initialize_at(app_log_directory())
}

fn initialize_at(directory: PathBuf) -> Result<LoggingGuard, Box<dyn std::error::Error>> {
    fs::create_dir_all(&directory)?;
    prune_rotated_logs(&directory)?;

    let appender = tracing_appender::rolling::daily(&directory, LOG_FILE_NAME);
    let (writer, worker_guard) = tracing_appender::non_blocking(appender);
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_file(true)
                .with_line_number(true)
                .with_target(true)
                .with_writer(writer),
        )
        .try_init()?;

    Ok(LoggingGuard {
        _worker_guard: worker_guard,
        directory,
    })
}

#[cfg(target_os = "windows")]
fn app_log_directory() -> PathBuf {
    env_path("LOCALAPPDATA")
        .unwrap_or_else(env::temp_dir)
        .join("Denon AVR Remote")
        .join("logs")
}

#[cfg(target_os = "macos")]
fn app_log_directory() -> PathBuf {
    home_directory()
        .unwrap_or_else(env::temp_dir)
        .join("Library")
        .join("Logs")
        .join("Denon AVR Remote")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn app_log_directory() -> PathBuf {
    env_path("XDG_STATE_HOME")
        .or_else(|| home_directory().map(|home| home.join(".local").join("state")))
        .unwrap_or_else(env::temp_dir)
        .join("denon-avr-remote")
        .join("logs")
}

#[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
fn app_log_directory() -> PathBuf {
    env::temp_dir().join("denon-avr-remote").join("logs")
}

fn env_path(variable: &str) -> Option<PathBuf> {
    env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn home_directory() -> Option<PathBuf> {
    env_path("HOME")
}

fn prune_rotated_logs(directory: &Path) -> io::Result<()> {
    let mut logs = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(&format!("{LOG_FILE_NAME}.")))
        })
        .collect::<Vec<_>>();
    logs.sort_by_key(|entry| entry.file_name());
    let remove_count = logs.len().saturating_sub(MAX_ROTATED_LOG_FILES);
    for entry in logs.into_iter().take(remove_count) {
        fs::remove_file(entry.path())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pruning_keeps_the_newest_rotated_logs() {
        let directory = env::temp_dir().join(format!(
            "denon-avr-remote-logging-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        for day in 1..=MAX_ROTATED_LOG_FILES + 2 {
            fs::write(
                directory.join(format!("{LOG_FILE_NAME}.2026-01-{day:02}")),
                "test",
            )
            .unwrap();
        }

        prune_rotated_logs(&directory).unwrap();

        let mut names = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names.len(), MAX_ROTATED_LOG_FILES);
        assert_eq!(
            names.first().unwrap(),
            &format!("{LOG_FILE_NAME}.2026-01-03")
        );
        assert_eq!(
            names.last().unwrap(),
            &format!("{LOG_FILE_NAME}.2026-01-16")
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn log_directory_has_a_logs_component() {
        #[cfg(target_os = "macos")]
        assert_eq!(app_log_directory().file_name().unwrap(), "Denon AVR Remote");

        #[cfg(not(target_os = "macos"))]
        assert_eq!(app_log_directory().file_name().unwrap(), "logs");
    }
}
