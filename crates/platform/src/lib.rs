//! Concrete platform adapters for the Denon AVR application contracts.

mod async_avr;
mod config;
mod discovery;
mod sync_avr;

pub use async_avr::{AvrSession, AvrSessionConfig, AvrSessionError, AvrSessionEvent};
pub use config::TomlConfigRepository;
pub use discovery::{SsdpDiscovery, DEFAULT_DISCOVERY_TIMEOUT};
pub use sync_avr::SyncAvrClient;
