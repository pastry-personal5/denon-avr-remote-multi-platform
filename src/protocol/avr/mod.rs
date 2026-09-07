//! Denon AVR protocol primitives.
//!
//! This module contains the canonical AVR protocol implementation that is
//! independent of transport mechanisms.

pub mod command;
pub mod response;

pub use command::{
    encode_control, encode_native_volume, encode_volume, query_command, AvrCommand,
    AvrProtocolError,
};
pub use response::{
    get_command_family, parse_main_zone_event, parse_main_zone_response, response_matches,
};
