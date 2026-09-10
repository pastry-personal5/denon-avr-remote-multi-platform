//! Application use cases and ports.

pub mod controller;
mod http_information;
pub mod main_zone_control;
pub mod main_zone_status;
pub mod ports;
mod quick_select;
pub mod receiver_selection;
mod source_catalog;

pub use controller::{
    ControlResult, ControllerConfig, ControllerHandle, Diagnostic, Lifecycle, Observability,
    ReceiverController, ReceiverEvent, ReceiverSelection,
};
