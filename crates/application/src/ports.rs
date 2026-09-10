//! Application ports for infrastructure abstraction.

use denon_avr_domain::{
    AudioContextSnapshot, ConfiguredReceivers, ConnectionState, DiscoveredReceiver, EqStatus,
    HttpInformationSnapshot, MainZoneControl, MainZoneEvent, MainZoneField, MainZoneValue,
    QuickSelectNameObservation, QuickSelectRecallConfirmation, QuickSelectSlot, ReceiverIdentity,
    SourceCatalogObservation,
};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    Connection(ConnectionState),
    MainZone(MainZoneEvent),
}

pub trait StatusGateway {
    fn query_field(&mut self, field: MainZoneField) -> Result<MainZoneValue, OperationError>;
    fn connection_generation(&self) -> u64;
    fn next_event(&mut self, timeout: Option<Duration>) -> Result<SessionEvent, OperationError>;
    fn query_audio_context(&mut self) -> AudioContextSnapshot;
}

/// A write-only boundary for state-changing commands. Implementations must
/// never retry a command after dispatch has begun.
pub trait ControlGateway {
    fn execute_once(
        &mut self,
        control: denon_avr_domain::MainZoneControl,
    ) -> Result<(), OperationError>;
}

pub trait AsyncControlGateway: Send {
    fn execute_once(
        &mut self,
        control: denon_avr_domain::MainZoneControl,
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

/// An asynchronous, stateful receiver connection.  This is the only port
/// which combines reads, writes, lifecycle events, and shutdown because a
/// single owner is required to preserve command and event ordering.
pub trait ReceiverSession: Send {
    fn query_field(
        &mut self,
        field: MainZoneField,
    ) -> BoxFuture<'_, Result<MainZoneValue, OperationError>>;
    /// Dispatches one state-changing AVR command. Implementations must never
    /// retry once dispatch begins.
    fn execute_once(
        &mut self,
        control: MainZoneControl,
    ) -> BoxFuture<'_, Result<(), OperationError>>;
    fn query_audio_context(&mut self) -> BoxFuture<'_, AudioContextSnapshot>;
    fn next_event(&mut self) -> BoxFuture<'_, Result<SessionEvent, OperationError>>;
    fn close(&mut self) -> BoxFuture<'_, Result<(), OperationError>>;

    fn recall_quick_select(
        &mut self,
        _slot: QuickSelectSlot,
    ) -> BoxFuture<'_, Result<QuickSelectRecallConfirmation, OperationError>> {
        Box::pin(async {
            Err(OperationError::new(
                OperationErrorKind::Unsupported,
                "Quick Select",
                "Quick Select protocol is not validated",
            ))
        })
    }
    fn query_eq_status(&mut self) -> BoxFuture<'_, Result<EqStatus, OperationError>> {
        Box::pin(async {
            Err(OperationError::new(
                OperationErrorKind::Unsupported,
                "EQ status",
                "EQ protocol is not validated",
            ))
        })
    }
    fn refresh_source_catalog(
        &mut self,
    ) -> BoxFuture<'_, Result<SourceCatalogObservation, OperationError>> {
        Box::pin(async {
            Err(OperationError::new(
                OperationErrorKind::Unsupported,
                "source catalog",
                "source catalog protocol is not validated",
            ))
        })
    }
    fn refresh_quick_select_names(
        &mut self,
    ) -> BoxFuture<'_, Result<QuickSelectNameObservation, OperationError>> {
        Box::pin(async {
            Err(OperationError::new(
                OperationErrorKind::Unsupported,
                "Quick Select names",
                "Quick Select name protocol is not validated",
            ))
        })
    }
    fn refresh_http_information(
        &mut self,
    ) -> BoxFuture<'_, Result<HttpInformationSnapshot, OperationError>> {
        Box::pin(async {
            Err(OperationError::new(
                OperationErrorKind::Unsupported,
                "HTTP information",
                "receiver has no validated HTTP information capability",
            ))
        })
    }
}

/// Creates a fresh session for a selected receiver. The coordinator owns the
/// returned session for its entire lifetime.
pub trait SessionFactory: Send + Sync + 'static {
    fn connect(
        &self,
        identity: ReceiverIdentity,
    ) -> BoxFuture<'_, Result<Box<dyn ReceiverSession>, OperationError>>;
}

impl<T: SessionFactory + ?Sized> SessionFactory for Arc<T> {
    fn connect(
        &self,
        identity: ReceiverIdentity,
    ) -> BoxFuture<'_, Result<Box<dyn ReceiverSession>, OperationError>> {
        (**self).connect(identity)
    }
}

/// Read-only receiver-owned source names and visibility. Implementations must
/// preserve the raw response as diagnostic evidence and never issue writes.
pub trait SourceCatalogReader: Send {
    fn refresh_source_catalog(
        &mut self,
    ) -> BoxFuture<'_, Result<SourceCatalogObservation, OperationError>>;
}
