//! Runtime-neutral asynchronous AVR transport contract.
use crate::session::{AvrSession, AvrSessionError};
use std::future::Future;
use std::pin::Pin;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportConnectionState {
    Connected,
    Reconnecting,
    Disconnected,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportEvent {
    Line(String),
    State(TransportConnectionState),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    InvalidCommand(String),
    Connection(String),
    Timeout(String),
    Disconnected(String),
    Malformed(String),
    Unexpected(String),
    Stopped,
}
impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for TransportError {}
pub trait AsyncAvrTransport: Send + Sync {
    fn request<'a>(
        &'a self,
        command: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String, TransportError>> + Send + 'a>>;
    fn next_event<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Option<TransportEvent>> + Send + 'a>>;
}
impl From<AvrSessionError> for TransportError {
    fn from(e: AvrSessionError) -> Self {
        match e {
            AvrSessionError::InvalidCommand(x) => Self::InvalidCommand(x),
            AvrSessionError::Connection(x) => Self::Connection(x),
            AvrSessionError::Timeout(x) => Self::Timeout(x),
            AvrSessionError::Disconnected(x) => Self::Disconnected(x),
            AvrSessionError::MalformedFrame(x) => Self::Malformed(x),
            AvrSessionError::UnexpectedResponse(x) => Self::Unexpected(x),
            AvrSessionError::SessionStopped => Self::Stopped,
            AvrSessionError::ReceiverError(x) => Self::Unexpected(x),
        }
    }
}
impl AsyncAvrTransport for AvrSession {
    fn request<'a>(
        &'a self,
        command: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String, TransportError>> + Send + 'a>> {
        Box::pin(async move { self.request_query(command).await.map_err(Into::into) })
    }
    fn next_event<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Option<TransportEvent>> + Send + 'a>> {
        Box::pin(async move {
            self.next_event().await.map(|event| match event {
                crate::session::AvrSessionEvent::Connected => {
                    TransportEvent::State(TransportConnectionState::Connected)
                }
                crate::session::AvrSessionEvent::Reconnected => {
                    TransportEvent::State(TransportConnectionState::Connected)
                }
                crate::session::AvrSessionEvent::Line(line) => TransportEvent::Line(line),
                crate::session::AvrSessionEvent::Disconnected(_) => {
                    TransportEvent::State(TransportConnectionState::Disconnected)
                }
            })
        })
    }
}
