//! Framework-independent Denon AVR domain, protocol, and application logic.

pub mod application;
pub mod domain;
pub mod protocol;

pub use application::{
    query_main_zone, query_main_zone_async, resolve_receiver, AsyncConfigRepository,
    AsyncReceiverDiscovery, AsyncStatusGateway, BoxFuture, ConfigRepository, OperationError,
    OperationErrorKind, ReceiverDiscovery, ResolvedReceiver, SessionEvent, StatusGateway,
};
pub use domain::{
    ConfiguredReceivers, ConnectionState, DiscoveredReceiver, FieldError, FieldErrorKind,
    FieldStatus, Freshness, Input, MainZoneEvent, MainZoneField, MainZoneSnapshot, MainZoneValue,
    Model, ModelCapabilities, MuteState, PowerState, ReceiverIdentity, StateAuthority,
    SurroundMode, Volume,
};
pub use protocol::{
    encode_volume, parse_heos_line, AvrCommand, AvrProtocolError, HeosCommand, HeosLine,
    HeosProtocolError,
};
