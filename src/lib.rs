//! Building blocks for Denon/Marantz AVR and HEOS IP control.
//!
//! The AVR and HEOS protocols intentionally have separate modules and framing
//! types. Receiver-specific command availability belongs in [`capabilities`]
//! and must be confirmed against live hardware.

pub mod application;
pub mod avr;
pub mod capabilities;
pub mod config;
pub mod discovery;
pub mod heos;
pub mod response;
pub mod session;
pub mod state;
pub mod status;
pub mod transport;

pub use application::ApplicationService;
pub use avr::{AvrCommand, AvrEvent, AvrLine, AvrResponse, VolumeCode};
pub use capabilities::{Model, ModelCapabilities};
pub use heos::{HeosCommand, HeosLine};
pub use session::{AvrSession, AvrSessionConfig, AvrSessionError, AvrSessionEvent};
pub use state::{Freshness, MainZoneEvent, MainZoneState, StateAuthority};
#[allow(deprecated)]
pub use status::{
    query_main_zone, query_main_zone_async, render as render_status, AvrTransport, MainZoneStatus,
    StatusField, TcpAvrTransport,
};
pub use transport::{AsyncAvrTransport, TransportConnectionState, TransportError, TransportEvent};
