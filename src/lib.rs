//! Denon/Marantz AVR and HEOS IP control library.
//!
//! The public API is organized into domain, application, protocol, and
//! infrastructure layers.

pub mod application;
pub mod domain;
pub mod infrastructure;
pub mod protocol;

pub use application::{query_main_zone_status, query_main_zone_status_async};
pub use application::{ControllerConfig, ReceiverCommand, ReceiverController, ReceiverEvent};

pub use application::ports::{
    AsyncConfigRepository, AsyncControlGateway, AsyncReceiverDiscovery, AsyncStatusGateway,
    BoxFuture, ConfigRepository, ControlGateway, OperationError, OperationErrorKind,
    ReceiverDiscovery, SessionEvent, StatusGateway,
};
pub use domain::{
    ConfiguredReceivers, ConnectionState, DiscoveredReceiver, FieldError, FieldErrorKind,
    FieldStatus, Input, MainZoneControl, MainZoneField, MainZoneSnapshot, MainZoneValue, MuteState,
    PowerState, ReceiverEndpoint, SurroundMode, Volume, VolumeLevel,
};
