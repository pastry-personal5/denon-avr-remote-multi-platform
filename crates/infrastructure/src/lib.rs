//! Infrastructure adapters for concrete implementations.
//!
//! This module contains all concrete I/O implementations that depend on
//! Tokio, filesystem, networking, and other platform-specific APIs.

pub mod app_command_http;
mod avr_session;
mod canonical_factory;
pub mod config_yaml;
pub mod discovery_ssdp;
pub mod http_information;
pub mod quick_select_names;
pub mod source_catalog;
pub mod x3800h_reducer;
pub mod x3800h_session;

pub use app_command_http::{AppCommandExchange, AppCommandHttpClient, RawHttpResponse};
pub use avr_session::AvrSessionConfig;
pub use canonical_factory::CanonicalSessionFactory;
pub use config_yaml::YamlConfigRepository;
pub use discovery_ssdp::SsdpDiscoveryAdapter;
pub use http_information::{HttpInformationHttpClient, X3800H_HTTP_PORT};
pub use quick_select_names::QuickSelectNamesHttpClient;
pub use source_catalog::SourceCatalogHttpClient;
pub use x3800h_session::X3800hSession;
