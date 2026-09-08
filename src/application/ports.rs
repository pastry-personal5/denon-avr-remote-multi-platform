//! Application ports for infrastructure abstraction.

use crate::domain::{
    AudioContextSnapshot, ConfiguredReceivers, ConnectionState, DiscoveredReceiver, MainZoneEvent,
    MainZoneField, MainZoneValue,
};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    Connection(ConnectionState),
    MainZone(MainZoneEvent),
}

pub trait StatusGateway {
    fn query_field(&mut self, field: MainZoneField) -> Result<MainZoneValue, OperationError>;
    fn connection_generation(&self) -> u64;
    fn next_event(&mut self, timeout: Option<Duration>) -> Result<SessionEvent, OperationError>;
}

/// A write-only boundary for state-changing commands. Implementations must
/// never retry a command after dispatch has begun.
pub trait ControlGateway {
    fn execute_once(
        &mut self,
        control: crate::domain::MainZoneControl,
    ) -> Result<(), OperationError>;
}

pub trait AsyncControlGateway: Send {
    fn execute_once(
        &mut self,
        control: crate::domain::MainZoneControl,
    ) -> BoxFuture<'_, Result<(), OperationError>>;
}

pub trait AsyncStatusGateway: Send {
    fn query_field(
        &mut self,
        field: MainZoneField,
    ) -> BoxFuture<'_, Result<MainZoneValue, OperationError>>;
    fn connection_generation(&self) -> u64;
    fn next_event(&mut self) -> BoxFuture<'_, Result<SessionEvent, OperationError>>;
    fn query_audio_context(&mut self) -> BoxFuture<'_, AudioContextSnapshot>;
}
