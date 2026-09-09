//! Desktop-owned, file-backed logging configuration.
//!
//! All application crates emit through the `tracing` facade.  This module is
//! the one composition-root place that decides where those events go and keeps
//! the asynchronous writer alive for the lifetime of the desktop process.

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

const LOG_FILE_NAME: &str = "denon-avr-remote.log";
const MAX_ROTATED_LOG_FILES: usize = 14;
const MAX_TOTAL_LOG_BYTES: u64 = 500 * 1024;

pub struct LoggingGuard {
    _worker_guard: WorkerGuard,
    directory: PathBuf,
}

impl LoggingGuard {
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

/// Configures daily file rotation while retaining at most fourteen files and
/// 500 KiB in total.
///
/// `RUST_LOG` controls filtering when set; otherwise the desktop records
/// `info` and higher events. The returned guard must outlive the application.
pub fn initialize() -> Result<LoggingGuard, Box<dyn std::error::Error>> {
    initialize_at(app_log_directory())
}

fn initialize_at(directory: PathBuf) -> Result<LoggingGuard, Box<dyn std::error::Error>> {
    fs::create_dir_all(&directory)?;
    let (writer, worker_guard) =
        tracing_appender::non_blocking(CappedDailyWriter::new(directory.clone())?);
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

/// A serial writer used behind `tracing_appender`'s non-blocking worker.
/// Keeping quota enforcement here means a busy day cannot exceed the on-disk
/// budget before the next daily rotation.
struct CappedDailyWriter {
    directory: PathBuf,
    file_name: String,
    file: fs::File,
}

impl CappedDailyWriter {
    fn new(directory: PathBuf) -> io::Result<Self> {
        let file_name = dated_log_file_name();
        let active_path = directory.join(&file_name);
        if active_path
            .metadata()
            .is_ok_and(|metadata| metadata.len() > MAX_TOTAL_LOG_BYTES)
        {
            // A log from a previous version may already exceed the newly
            // introduced quota. Start a fresh current-day file so the limit
            // takes effect immediately rather than waiting for tomorrow.
            fs::remove_file(&active_path)?;
        }
        let file = open_log_file(&directory, &file_name)?;
        let writer = Self {
            directory,
            file_name,
            file,
        };
        writer.enforce_budget(0)?;
        Ok(writer)
    }

    fn rotate_if_needed(&mut self) -> io::Result<()> {
        let file_name = dated_log_file_name();
        if file_name != self.file_name {
            self.file = open_log_file(&self.directory, &file_name)?;
            self.file_name = file_name;
        }
        Ok(())
    }

    fn enforce_budget(&self, incoming_bytes: usize) -> io::Result<bool> {
        prune_logs(
            &self.directory,
            Some(&self.file_name),
            incoming_bytes as u64,
        )
    }
}

impl Write for CappedDailyWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.rotate_if_needed()?;
        if !self.enforce_budget(buffer.len())? {
            // `Write` callers expect a successful full write. Deliberately
            // discard this low-value diagnostic record once the fixed quota is
            // exhausted rather than expanding the local log footprint.
            return Ok(buffer.len());
        }
        self.file.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn dated_log_file_name() -> String {
    format!("{LOG_FILE_NAME}.{}", OffsetDateTime::now_utc().date())
}

fn open_log_file(directory: &Path, file_name: &str) -> io::Result<fs::File> {
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join(file_name))
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

/// Deletes oldest files until both retention limits can accommodate an
/// incoming record. The active file is never removed during a write.
fn prune_logs(
    directory: &Path,
    active_file_name: Option<&str>,
    incoming_bytes: u64,
) -> io::Result<bool> {
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

    let mut total_bytes = logs
        .iter()
        .map(|entry| entry.metadata().map(|metadata| metadata.len()))
        .collect::<io::Result<Vec<_>>>()?
        .into_iter()
        .sum::<u64>();
    let mut retained_count = logs.len();
    for entry in logs {
        let is_active = active_file_name.is_some_and(|active| entry.file_name() == active);
        let size = entry.metadata()?.len();
        if !is_active
            && (retained_count > MAX_ROTATED_LOG_FILES
                || total_bytes.saturating_add(incoming_bytes) > MAX_TOTAL_LOG_BYTES)
        {
            fs::remove_file(entry.path())?;
            retained_count -= 1;
            total_bytes = total_bytes.saturating_sub(size);
        }
    }
    Ok(total_bytes.saturating_add(incoming_bytes) <= MAX_TOTAL_LOG_BYTES)
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

        assert!(prune_logs(&directory, None, 0).unwrap());

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
    fn pruning_removes_old_files_to_make_room_for_new_records() {
        let directory = env::temp_dir().join(format!(
            "denon-avr-remote-size-logging-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let old_file = directory.join(format!("{LOG_FILE_NAME}.2026-01-01"));
        let active_name = format!("{LOG_FILE_NAME}.2026-01-02");
        let active_file = directory.join(&active_name);
        fs::write(&old_file, vec![0; 300 * 1024]).unwrap();
        fs::write(&active_file, vec![0; 300 * 1024]).unwrap();

        assert!(prune_logs(&directory, Some(&active_name), 200 * 1024).unwrap());
        assert!(!old_file.exists());
        assert!(active_file.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn quota_rejects_a_record_when_the_active_file_is_full() {
        let directory = env::temp_dir().join(format!(
            "denon-avr-remote-full-logging-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let active_name = format!("{LOG_FILE_NAME}.2026-01-01");
        fs::write(
            directory.join(&active_name),
            vec![0; MAX_TOTAL_LOG_BYTES as usize],
        )
        .unwrap();

        assert!(!prune_logs(&directory, Some(&active_name), 1).unwrap());
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
