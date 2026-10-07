//! Denon AVR protocol primitives.
//!
//! This module contains the canonical AVR protocol implementation that is
//! independent of transport mechanisms.

pub mod command;
pub mod response;
pub mod x3800h;

pub use command::{AvrCommand, AvrProtocolError};
pub use response::{get_command_family, response_matches};
pub use x3800h::{
    encode as encode_x3800h, parse as parse_x3800h, query as x3800h_query, X3800hFrame,
};
