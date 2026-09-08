//! Infrastructure adapters for concrete implementations.
//!
//! This module contains all concrete I/O implementations that depend on
//! Tokio, filesystem, networking, and other platform-specific APIs.

pub mod avr_session;
pub mod config_yaml;
pub mod discovery_ssdp;
pub mod tcp_avr;

pub use avr_session::{AvrSession, AvrSessionConfig, AvrSessionFactory};
pub use config_yaml::YamlConfigRepository;
pub use discovery_ssdp::SsdpDiscoveryAdapter;
pub use tcp_avr::SyncAvrClient;
