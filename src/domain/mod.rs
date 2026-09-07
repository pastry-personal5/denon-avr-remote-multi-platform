//! Domain types for Denon/Marantz AVR control.
//!
//! This module contains the canonical domain model that is independent of
//! protocol details, transport mechanisms, and infrastructure concerns.

pub mod capabilities;
pub mod main_zone;
pub mod receiver;

pub use capabilities::{Model, ModelCapabilities};
pub use main_zone::{
    ConnectionState, FieldError, FieldErrorKind, FieldStatus, Freshness, Input, MainZoneEvent,
    MainZoneField, MainZoneSnapshot, MainZoneValue, MuteState, PowerState, StateAuthority,
    SurroundMode, Volume,
};
pub use receiver::{ConfiguredReceivers, DiscoveredReceiver, ReceiverEndpoint, ReceiverIdentity};
