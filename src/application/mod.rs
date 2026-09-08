//! Application use cases and ports.

pub mod controller;
pub mod main_zone_control;
pub mod main_zone_status;
pub mod ports;
pub mod receiver_selection;

pub use controller::{
    CommandReply, ControlResult, ControllerConfig, ControllerHandle, Diagnostic, Lifecycle,
    NoopObservability, Observability, ReceiverCommand, ReceiverController, ReceiverEvent,
    ReceiverSelection, ReceiverSession, SessionFactory, Sleeper, TokioSleeper,
};
pub use main_zone_control::{
    dispatch_main_zone_control, execute_main_zone_control, execute_main_zone_control_async,
    ControlOutcome,
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
