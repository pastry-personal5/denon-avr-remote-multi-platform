//! Application use cases and ports.

pub mod main_zone_control;
pub mod main_zone_status;
pub mod ports;
pub mod receiver_selection;

pub use main_zone_control::{
    execute_main_zone_control, execute_main_zone_control_async, ControlOutcome,
};
pub use main_zone_status::{query_main_zone_status, query_main_zone_status_async};
pub use ports::{
    AsyncConfigRepository, AsyncControlGateway, AsyncReceiverDiscovery, AsyncStatusGateway,
    BoxFuture, ConfigRepository, ControlGateway, OperationError, OperationErrorKind,
    ReceiverDiscovery, SessionEvent, StatusGateway,
};
pub use receiver_selection::{
    resolve_receiver, resolve_status_receiver, resolve_status_receiver_with_probe, ResolvedReceiver,
};
