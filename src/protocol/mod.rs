//! Protocol framing and parsing.
//!
//! This module contains transport-independent protocol implementations that
//! remain usable without a network connection or async runtime.

pub mod avr;
pub mod heos;

pub use avr::{
    encode_volume, get_command_family, parse_main_zone_event, parse_main_zone_response,
    query_command, response_matches, AvrCommand, AvrProtocolError,
};
pub use heos::{parse_heos_line, HeosCommand, HeosLine, HeosProtocolError};
