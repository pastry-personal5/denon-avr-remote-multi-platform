//! Application ports for infrastructure abstraction.

use crate::session_v3::SharedReceiverSession;
use denon_avr_domain::{ConfiguredReceivers, DiscoveredReceiver, ReceiverId, ReceiverIdentity};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationErrorKind {
    Configuration,
    Discovery,
    InvalidSelection,
    Connection,
    Timeout,
    Disconnected,
    Malformed,
    Unavailable,
    Unsupported,
    Stopped,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationError {
    pub kind: OperationErrorKind,
    pub context: &'static str,
    pub message: String,
}

impl OperationError {
    pub fn new(
        kind: OperationErrorKind,
        context: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            context,
            message: message.into(),
        }
    }
}

impl fmt::Display for OperationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.context, self.message)
    }
}

impl std::error::Error for OperationError {}

pub trait ConfigRepository {
    fn load(&self) -> Result<ConfiguredReceivers, OperationError>;
    fn save(&self, config: &ConfiguredReceivers) -> Result<(), OperationError>;
}

pub trait AsyncConfigRepository: Send + Sync {
    fn load(&self) -> BoxFuture<'_, Result<ConfiguredReceivers, OperationError>>;
    fn save<'a>(
        &'a self,
        config: &'a ConfiguredReceivers,
    ) -> BoxFuture<'a, Result<(), OperationError>>;
}

pub trait ReceiverDiscovery {
    fn discover(&self, timeout: Duration) -> Result<Vec<DiscoveredReceiver>, OperationError>;
}

pub trait AsyncReceiverDiscovery: Send + Sync {
    fn discover(
        &self,
        timeout: Duration,
    ) -> BoxFuture<'_, Result<Vec<DiscoveredReceiver>, OperationError>>;
}

/// Opens the canonical session for one receiver. The control service owns the
/// returned session until it releases it.
///
/// The receiver id is passed explicitly: it is the configuration entry name and
/// must not be derived from the address, which can change.
pub trait ReceiverConnector: Send + Sync + 'static {
    /// Connect and return a session that has not yet been synchronized. A
    /// receiver that is unreachable, or whose single control connection is held
    /// by another client, is an error.
    fn connect<'a>(
        &'a self,
        receiver: &'a ReceiverId,
        identity: &'a ReceiverIdentity,
    ) -> BoxFuture<'a, Result<SharedReceiverSession, OperationError>>;
}

impl<T: ReceiverConnector + ?Sized> ReceiverConnector for Arc<T> {
    fn connect<'a>(
        &'a self,
        receiver: &'a ReceiverId,
        identity: &'a ReceiverIdentity,
    ) -> BoxFuture<'a, Result<SharedReceiverSession, OperationError>> {
        (**self).connect(receiver, identity)
    }
}
