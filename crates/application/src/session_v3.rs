//! Phase 5 receiver-session contract.
//!
//! Unlike the legacy gateway traits, this is state-first: subscribers read the
//! same complete receiver state that operation matching uses.

use crate::ports::{BoxFuture, OperationError, OperationErrorKind};
use denon_avr_domain::{
    CoreField, HttpInformationSnapshot, OperationId, OperationOutcome, Precondition,
    QuickSelectNameObservation, ReceiverIntent, ReceiverState, SourceCatalogObservation,
};
use std::any::Any;
use std::sync::Arc;
use tokio::sync::watch;
use tracing::debug;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OperationRequest {
    pub id: OperationId,
    pub intent: ReceiverIntent,
    /// What the decision behind this request saw. The session re-observes every
    /// field it names before writing and refuses the write if any differs.
    /// Operator operations carry none.
    pub precondition: Option<Precondition>,
}

impl OperationRequest {
    pub fn new(id: OperationId, intent: ReceiverIntent) -> Self {
        Self {
            id,
            intent,
            precondition: None,
        }
    }

    pub fn with_precondition(mut self, precondition: Precondition) -> Self {
        self.precondition = Some(precondition);
        self
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
    /// Becomes true when the owner closes the session. A session keeps the
    /// sender of its state channel for as long as it exists, so without this a
    /// subscriber to a closed session would wait for ever.
    ended: Option<watch::Receiver<bool>>,
}

impl StateSubscription {
    pub fn new(receiver: watch::Receiver<ReceiverState>) -> Self {
        Self {
            receiver,
            _hold: None,
            ended: None,
        }
    }

    /// Keep `hold` alive until this subscription is dropped.
    pub fn holding(mut self, hold: impl Any + Send + Sync) -> Self {
        self._hold = Some(Box::new(hold));
        self
    }

    /// End this subscription when `ended` becomes true, or its sender is dropped.
    pub fn ending_with(mut self, ended: watch::Receiver<bool>) -> Self {
        self.ended = Some(ended);
        self
    }

    pub fn latest(&self) -> ReceiverState {
        self.receiver.borrow().clone()
    }

    /// The next state, or an error once the session has closed.
    pub async fn changed(&mut self) -> Result<ReceiverState, OperationError> {
        let closed = || {
            debug!("receiver state subscription closed");
            OperationError::new(
                crate::ports::OperationErrorKind::Stopped,
                "receiver state subscription",
                "session closed",
            )
        };
        match self.ended.as_mut() {
            Some(ended) => {
                tokio::select! {
                    biased;
                    _ = ended.wait_for(|ended| *ended) => return Err(closed()),
                    changed = self.receiver.changed() => changed.map_err(|_| closed())?,
                }
            }
            None => self.receiver.changed().await.map_err(|_| closed())?,
        }
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

    /// The receiver's own source names and visibility. A read: it never writes,
    /// and the result carries the connection generation it was read under.
    /// Sessions that cannot read it report `Unsupported`.
    fn source_catalog(&self) -> BoxFuture<'_, Result<SourceCatalogObservation, OperationError>> {
        unsupported_read("source catalog")
    }

    /// Quick Select names, for Operator use only. A read, like the others.
    fn quick_select_names(
        &self,
    ) -> BoxFuture<'_, Result<QuickSelectNameObservation, OperationError>> {
        unsupported_read("Quick Select names")
    }

    /// Audio, video, and Audyssey information over HTTP. A read, like the others.
    fn http_information(&self) -> BoxFuture<'_, Result<HttpInformationSnapshot, OperationError>> {
        unsupported_read("HTTP information")
    }
    /// Close the serialized session. Implementations must make repeated
    /// close requests harmless and must not reopen transport work.
    fn close(&self) -> BoxFuture<'_, Result<(), OperationError>>;
}

fn unsupported_read<T: Send + 'static>(
    what: &'static str,
) -> BoxFuture<'static, Result<T, OperationError>> {
    Box::pin(async move {
        Err(OperationError::new(
            OperationErrorKind::Unsupported,
            what,
            "this receiver session does not provide the read",
        ))
    })
}

pub type SharedReceiverSession = Arc<dyn CanonicalReceiverSession>;
