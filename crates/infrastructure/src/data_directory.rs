//! Where the application keeps its files.
//!
//! The configuration, the policy, and the audit log live in one directory, so
//! one place says where. On macOS it is `~/Library/Application Support/Denon
//! AVR Remote`; elsewhere it is the relative `config/` directory the
//! configuration has always used.

use std::path::PathBuf;

/// The directory the application's files are in.
#[cfg(target_os = "macos")]
pub fn data_directory() -> PathBuf {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("~"))
        .join("Library")
        .join("Application Support")
        .join("Denon AVR Remote")
}

/// The directory the application's files are in.
#[cfg(not(target_os = "macos"))]
pub fn data_directory() -> PathBuf {
    PathBuf::from("config")
}

/// The audit log's directory.
pub fn audit_directory() -> PathBuf {
    data_directory().join("audit")
}

/// The policy file, beside the configuration.
pub fn policy_path() -> PathBuf {
    data_directory().join("policy.yaml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_configuration_file_and_the_audit_log_share_one_directory() {
        let directory = data_directory();
        assert_eq!(
            crate::config_yaml::default_config_path(),
            directory.join("denon-avr-remote.yaml")
        );
        assert_eq!(audit_directory(), directory.join("audit"));
        assert_eq!(policy_path(), directory.join("policy.yaml"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn on_macos_it_is_the_application_support_directory() {
        let directory = data_directory();
        assert!(directory.ends_with("Library/Application Support/Denon AVR Remote"));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn elsewhere_it_is_the_relative_config_directory() {
        assert_eq!(data_directory(), std::path::PathBuf::from("config"));
    }
}
