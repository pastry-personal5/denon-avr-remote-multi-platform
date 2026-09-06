//! Building blocks for Denon/Marantz AVR and HEOS IP control.
//!
//! The AVR and HEOS protocols intentionally have separate modules and framing
//! types. Receiver-specific command availability belongs in [`capabilities`]
//! and must be confirmed against live hardware.

pub mod avr;
pub mod capabilities;
pub mod heos;

pub use avr::{AvrCommand, AvrEvent, AvrLine, AvrResponse, VolumeCode};
pub use capabilities::{Model, ModelCapabilities};
pub use heos::{HeosCommand, HeosLine};
