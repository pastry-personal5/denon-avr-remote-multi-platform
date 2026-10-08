//! Infrastructure adapters for concrete implementations.
//!
//! This module contains all concrete I/O implementations that depend on
//! Tokio, filesystem, networking, and other platform-specific APIs.

pub mod app_command_http;
pub mod audit_jsonl;
mod avr_session;
pub mod config_yaml;
pub mod data_directory;
pub mod discovery_ssdp;
pub mod http_information;
pub mod policy_yaml;
pub mod quick_select_names;
mod receiver_connector;
pub mod source_catalog;
pub mod system_clock;
pub mod token_store;
pub mod x3800h_reducer;
pub mod x3800h_session;

pub use app_command_http::{AppCommandExchange, AppCommandHttpClient, RawHttpResponse};
pub use audit_jsonl::{AuditLimits, JsonlAuditLog};
pub use avr_session::AvrSessionConfig;
pub use config_yaml::YamlConfigRepository;
pub use data_directory::{audit_directory, data_directory, policy_path};
pub use discovery_ssdp::SsdpDiscoveryAdapter;
pub use http_information::{HttpInformationHttpClient, X3800H_HTTP_PORT};
pub use policy_yaml::YamlPolicySource;
pub use quick_select_names::QuickSelectNamesHttpClient;
pub use receiver_connector::X3800hConnector;
pub use source_catalog::SourceCatalogHttpClient;
pub use system_clock::SystemClock;
pub use token_store::{FileTokenStore, TokenStoreError};
pub use x3800h_session::X3800hSession;
