//! Infrastructure adapters for concrete implementations.
//!
//! This module contains all concrete I/O implementations that depend on
//! Tokio, filesystem, networking, and other platform-specific APIs.

pub mod app_command_http;
pub mod avr_session;
pub mod config_yaml;
pub mod discovery_ssdp;
pub mod http_information;
pub mod source_catalog;
pub mod tcp_avr;

pub use app_command_http::{AppCommandExchange, AppCommandHttpClient, RawHttpResponse};
pub use avr_session::{AvrSession, AvrSessionConfig, AvrSessionFactory};
pub use config_yaml::YamlConfigRepository;
pub use discovery_ssdp::SsdpDiscoveryAdapter;
pub use http_information::{HttpInformationHttpClient, X3800H_HTTP_PORT};
pub use source_catalog::SourceCatalogHttpClient;
pub use tcp_avr::SyncAvrClient;
