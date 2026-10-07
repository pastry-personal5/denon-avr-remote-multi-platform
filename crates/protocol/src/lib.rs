//! Protocol framing and parsing.
//!
//! This module contains transport-independent protocol implementations that
//! remain usable without a network connection or async runtime.

pub mod app_command;
pub mod avr;
pub mod heos;
pub mod http_information;
pub mod quick_select_name;
pub mod source_catalog;

pub use app_command::{
    parse_app_command_response, AppCommandParameter, AppCommandProtocolError, AppCommandQuery,
    AppCommandRequest, AppCommandResponse, AppCommandResult, APP_COMMAND_0300_PATH,
};

pub use avr::{get_command_family, response_matches, AvrCommand, AvrProtocolError};
pub use heos::{parse_heos_line, HeosCommand, HeosLine, HeosProtocolError};
pub use quick_select_name::{
    parse_quick_select_names, ParsedQuickSelectNames, QuickSelectNameProtocolError,
};
