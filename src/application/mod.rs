//! Application use cases and ports.

pub mod main_zone_status;
pub mod ports;
pub mod receiver_selection;

pub use main_zone_status::{query_main_zone_status, query_main_zone_status_async};
pub use ports::{
    AsyncConfigRepository, AsyncReceiverDiscovery, AsyncStatusGateway, BoxFuture, ConfigRepository,
    OperationError, OperationErrorKind, ReceiverDiscovery, SessionEvent, StatusGateway,
};
pub use receiver_selection::{
    resolve_receiver, resolve_status_receiver, resolve_status_receiver_with_probe, ResolvedReceiver,
};
