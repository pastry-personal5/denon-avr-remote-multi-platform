//! Building blocks for Denon/Marantz AVR and HEOS IP control.
//!
//! The AVR and HEOS protocols intentionally have separate modules and framing
//! types. Receiver-specific command availability belongs in [`capabilities`]
//! and must be confirmed against live hardware.

pub mod avr;
pub mod capabilities;
pub mod config;
pub mod discovery;
pub mod heos;
pub mod status;

pub use avr::{AvrCommand, AvrEvent, AvrLine, AvrResponse, VolumeCode};
pub use capabilities::{Model, ModelCapabilities};
pub use heos::{HeosCommand, HeosLine};
pub use status::{
    query_main_zone, render as render_status, AvrTransport, MainZoneStatus, StatusField,
    TcpAvrTransport,
};
