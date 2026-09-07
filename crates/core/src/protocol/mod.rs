mod avr;
mod heos;

pub use avr::{
    command_family, encode_volume, parse_event, parse_main_zone_response, query_command,
    response_matches, AvrCommand, AvrProtocolError,
};
pub use heos::{parse_line as parse_heos_line, HeosCommand, HeosLine, HeosProtocolError};
