//! Phase 5 receiver-session contract.
//!
//! Unlike the legacy gateway traits, this is state-first: subscribers read the
//! same complete receiver state that operation matching uses.

use crate::ports::{BoxFuture, OperationError};
use denon_avr_domain::{CoreField, OperationId, OperationOutcome, ReceiverIntent, ReceiverState};
use std::any::Any;
use std::sync::Arc;
use tokio::sync::watch;
use tracing::debug;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationRequest {
    pub id: OperationId,
    pub intent: ReceiverIntent,
}

impl OperationRequest {
    pub fn new(id: OperationId, intent: ReceiverIntent) -> Self {
        Self { id, intent }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Readiness {
    pub ready: bool,
    pub degraded: bool,
    pub detail: String,
}

/// A coalescing state subscription. A slow reader observes the newest complete
/// state; it cannot exert backpressure on the socket owner.
pub struct StateSubscription {
    receiver: watch::Receiver<ReceiverState>,
    /// Whatever the owner of the session wants kept alive for as long as this
    /// subscription lives. The control service uses it to keep a receiver
    /// connected while a subscriber is reading.
    _hold: Option<Box<dyn Any + Send + Sync>>,
}

impl StateSubscription {
    pub fn new(receiver: watch::Receiver<ReceiverState>) -> Self {
        Self {
            receiver,
            _hold: None,
        }
    }

    /// Keep `hold` alive until this subscription is dropped.
    pub fn holding(mut self, hold: impl Any + Send + Sync) -> Self {
        self._hold = Some(Box::new(hold));
        self
    }
    pub fn latest(&self) -> ReceiverState {
        self.receiver.borrow().clone()
    }
    pub async fn changed(&mut self) -> Result<ReceiverState, OperationError> {
        self.receiver.changed().await.map_err(|_| {
            debug!("receiver state subscription closed");
            OperationError::new(
                crate::ports::OperationErrorKind::Stopped,
                "receiver state subscription",
                "session closed",
            )
        })?;
        Ok(self.latest())
    }
}

pub trait CanonicalReceiverSession: Send + Sync {
    fn state(&self) -> StateSubscription;
    /// Observe one field without dispatching a mutating intent. Implementors
    /// may use a broader synchronization pass when targeted reads are not
    /// available, but must preserve the same canonical state owner.
    fn observe(&self, _field: CoreField) -> BoxFuture<'_, Result<(), OperationError>> {
        Box::pin(async move { self.synchronize().await.map(|_| ()) })
    }
    fn synchronize(&self) -> BoxFuture<'_, Result<Readiness, OperationError>>;
    fn operate(&self, request: OperationRequest) -> BoxFuture<'_, OperationOutcome>;
    /// Close the serialized session. Implementations must make repeated
    /// close requests harmless and must not reopen transport work.
    fn close(&self) -> BoxFuture<'_, Result<(), OperationError>>;
}

pub type SharedReceiverSession = Arc<dyn CanonicalReceiverSession>;
