//! Protocol framing and parsing.
//!
//! This module contains transport-independent protocol implementations that
//! remain usable without a network connection or async runtime.

pub mod app_command;
pub mod avr;
pub mod heos;

pub use app_command::{
    parse_app_command_response, AppCommandParameter, AppCommandProtocolError, AppCommandQuery,
    AppCommandRequest, AppCommandResponse, AppCommandResult, APP_COMMAND_0300_PATH,
};

pub use avr::{
    encode_volume, get_command_family, is_read_only_audio_context_query,
    parse_audio_context_response, parse_channel_volume_response, parse_main_zone_event,
    parse_main_zone_response, query_command, response_matches, AvrCommand, AvrProtocolError,
};
pub use heos::{parse_heos_line, HeosCommand, HeosLine, HeosProtocolError};
