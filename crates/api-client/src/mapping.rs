//! What the wire says, in the port's terms.

use denon_avr_api_contract::{ApiError, ErrorBody};
use denon_avr_application::ControlError;
use hyper::StatusCode;
use std::fmt::Display;

/// The words the design gives an agent when the server cannot be reached.
pub(crate) const UNAVAILABLE: &str = "receiver service unavailable";

/// A success answer that is not the shape the contract describes.
pub(crate) fn unreadable(why: impl Display) -> ControlError {
    ControlError::Unavailable(format!("the server's answer could not be read: {why}"))
}

/// An error response as the port's error. An answer that is not the contract's
/// error body, whatever its status, is `Unavailable`: it is not guessed to mean
/// something else.
pub(crate) fn error_response(status: StatusCode, body: &[u8]) -> ControlError {
    match serde_json::from_slice::<ErrorBody>(body) {
        Ok(body) => ApiError {
            status: status.as_u16(),
            body,
        }
        .into_control_error(),
        Err(_) => ControlError::Unavailable(format!("the server answered {}", status.as_u16())),
    }
}
