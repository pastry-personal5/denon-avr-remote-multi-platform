//! Errors on the wire.
//!
//! A failed call carries a status and a body of the form
//! `{"error":{"code":"...","message":"..."}}`. [`ApiError`] is what a server
//! builds from a [`ControlError`] or from its own refusal, and
//! [`ApiError::into_control_error`] is what a client turns it back into, so the
//! caller of the port sees the same typed error either way.

use crate::operations::OperationDto;
use denon_avr_application::ports::{OperationError, OperationErrorKind};
use denon_avr_application::{ControlError, OperationSnapshot};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// The words a `not_found` error may name. A client maps any other word to
/// `resource`, because the port's variant holds a static string.
const NOT_FOUND_WORDS: [&str; 3] = ["receiver", "operation", "token"];

/// The context a client gives an error from the Control API: the server's own
/// context is part of the message it sent.
const CLIENT_CONTEXT: &str = "control service";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
    /// What was not found: `receiver`, `operation`, or `token`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub what: Option<String>,
    /// For `receiver_error`, the kind of the session's error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// For `rate_limited`, whole seconds to wait, rounded up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u64>,
    /// For `too_late`, the operation as it actually is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<Box<OperationDto>>,
}

impl ErrorDetail {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            what: None,
            kind: None,
            retry_after_secs: None,
            operation: None,
        }
    }
}

/// A status and the body that goes with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub status: u16,
    pub body: ErrorBody,
}

impl ApiError {
    /// An error the server found before the port was involved: a missing
    /// credential, an unknown route, a body that is too large.
    pub fn server(status: u16, code: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            body: ErrorBody {
                error: ErrorDetail::new(code, message),
            },
        }
    }

    /// The 401 every refused credential gets: it does not say whether the token
    /// was missing, unknown, revoked, or for the other endpoint.
    pub fn unauthenticated() -> Self {
        Self::server(401, "unauthenticated", "a valid credential is required")
    }

    pub fn code(&self) -> &str {
        &self.body.error.code
    }

    /// The `Retry-After` header value, when the error asks for one.
    pub fn retry_after_secs(&self) -> Option<u64> {
        self.body.error.retry_after_secs
    }

    /// The client's reading of an error response.
    pub fn into_control_error(self) -> ControlError {
        let Self { status, body } = self;
        let detail = body.error;
        match detail.code.as_str() {
            "not_found" => ControlError::NotFound(
                NOT_FOUND_WORDS
                    .into_iter()
                    .find(|word| Some(*word) == detail.what.as_deref())
                    .unwrap_or("resource"),
            ),
            "forbidden" | "unauthenticated" => ControlError::Forbidden,
            "invalid_request" => ControlError::InvalidRequest(detail.message),
            "unavailable" | "shutting_down" => ControlError::Unavailable(detail.message),
            "too_late" => match detail
                .operation
                .map(|dto| OperationSnapshot::try_from(*dto))
            {
                Some(Ok(snapshot)) => ControlError::TooLate(Box::new(snapshot)),
                _ => ControlError::Unavailable("the server's answer could not be read".into()),
            },
            "rate_limited" | "too_many_streams" => ControlError::RateLimited {
                retry_after: detail.retry_after_secs.map(Duration::from_secs),
            },
            "receiver_error" => ControlError::Receiver(OperationError::new(
                detail
                    .kind
                    .as_deref()
                    .and_then(parse_kind)
                    .unwrap_or(OperationErrorKind::Unavailable),
                CLIENT_CONTEXT,
                detail.message,
            )),
            // Codes the server adds outside the port. They say what was wrong with
            // the request, so they read as an invalid request, except by status.
            _ => match status {
                401 | 403 => ControlError::Forbidden,
                404 => ControlError::NotFound("resource"),
                429 => ControlError::RateLimited {
                    retry_after: detail.retry_after_secs.map(Duration::from_secs),
                },
                500..=599 => ControlError::Unavailable(detail.message),
                _ => ControlError::InvalidRequest(detail.message),
            },
        }
    }
}

impl From<&ControlError> for ApiError {
    fn from(error: &ControlError) -> Self {
        match error {
            ControlError::NotFound(what) => {
                let mut detail = ErrorDetail::new("not_found", error.to_string());
                detail.what = Some((*what).to_owned());
                Self::with(404, detail)
            }
            ControlError::Forbidden => {
                Self::with(403, ErrorDetail::new("forbidden", "not permitted"))
            }
            ControlError::InvalidRequest(message) => {
                Self::with(400, ErrorDetail::new("invalid_request", message.clone()))
            }
            ControlError::Unavailable(message) => {
                Self::with(503, ErrorDetail::new("unavailable", message.clone()))
            }
            ControlError::TooLate(snapshot) => {
                let mut detail = ErrorDetail::new("too_late", error.to_string());
                detail.operation = Some(Box::new(OperationDto::from(snapshot.as_ref())));
                Self::with(409, detail)
            }
            ControlError::RateLimited { retry_after } => {
                let mut detail = ErrorDetail::new("rate_limited", "rate limited");
                detail.retry_after_secs = retry_after.map(whole_seconds_rounded_up);
                Self::with(429, detail)
            }
            ControlError::Receiver(inner) => {
                let mut detail = ErrorDetail::new("receiver_error", inner.to_string());
                detail.kind = Some(kind_name(inner.kind).to_owned());
                Self::with(502, detail)
            }
        }
    }
}

impl ApiError {
    fn with(status: u16, error: ErrorDetail) -> Self {
        Self {
            status,
            body: ErrorBody { error },
        }
    }
}

fn whole_seconds_rounded_up(duration: Duration) -> u64 {
    duration.as_secs() + u64::from(duration.subsec_nanos() > 0)
}

/// The stable name of each kind of session error.
pub fn kind_name(kind: OperationErrorKind) -> &'static str {
    match kind {
        OperationErrorKind::Configuration => "configuration",
        OperationErrorKind::Discovery => "discovery",
        OperationErrorKind::InvalidSelection => "invalid_selection",
        OperationErrorKind::Connection => "connection",
        OperationErrorKind::Timeout => "timeout",
        OperationErrorKind::Disconnected => "disconnected",
        OperationErrorKind::Malformed => "malformed",
        OperationErrorKind::Unavailable => "unavailable",
        OperationErrorKind::Unsupported => "unsupported",
        OperationErrorKind::Stopped => "stopped",
        OperationErrorKind::Conflict => "conflict",
    }
}

/// Every kind, for tests and for readers that must name them all.
pub const ALL_KINDS: [OperationErrorKind; 11] = [
    OperationErrorKind::Configuration,
    OperationErrorKind::Discovery,
    OperationErrorKind::InvalidSelection,
    OperationErrorKind::Connection,
    OperationErrorKind::Timeout,
    OperationErrorKind::Disconnected,
    OperationErrorKind::Malformed,
    OperationErrorKind::Unavailable,
    OperationErrorKind::Unsupported,
    OperationErrorKind::Stopped,
    OperationErrorKind::Conflict,
];

fn parse_kind(name: &str) -> Option<OperationErrorKind> {
    ALL_KINDS.into_iter().find(|kind| kind_name(*kind) == name)
}
