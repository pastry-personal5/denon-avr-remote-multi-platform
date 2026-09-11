//! Denon AVR protocol primitives.
//!
//! This module contains the canonical AVR protocol implementation that is
//! independent of transport mechanisms.

pub mod command;
pub mod quick_select_eq;
pub mod response;
pub mod x3800h;

pub use command::{
    audio_context_query_commands, encode_control, encode_native_volume, encode_volume,
    encode_zone2_control, is_read_only_audio_context_query, query_command, AvrCommand,
    AvrProtocolError,
};
pub use quick_select_eq::{eq_status_query, parse_eq_status, quick_select_command};
pub use response::{
    get_command_family, parse_audio_context_response, parse_channel_volume_response,
    parse_main_zone_event, parse_main_zone_response, parse_zone2_power, response_matches,
};
pub use x3800h::{
    encode as encode_x3800h, parse as parse_x3800h, query as x3800h_query, X3800hFrame,
};
