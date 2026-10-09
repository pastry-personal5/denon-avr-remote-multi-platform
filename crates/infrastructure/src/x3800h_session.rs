//! State-first X3800H session adapter.
//!
//! The legacy [`AvrSession`] remains available for the v2 controller while it
//! is being removed.  This type is the Phase 5 production-facing boundary: it
//! owns the only public core-state copy and publishes complete snapshots with
//! a `watch` channel.

use crate::avr_session::AvrSession;
use crate::{
    AvrSessionConfig, HttpInformationHttpClient, QuickSelectNamesHttpClient,
    SourceCatalogHttpClient,
};
use denon_avr_application::ports::OperationError;
use denon_avr_application::{
    CanonicalReceiverSession, OperationRequest, Readiness, StateSubscription,
};
use denon_avr_domain::{
    CoreField, DispatchCertainty, Epoch, OperationOutcome, ReceiverId, ReceiverIntent,
    ReceiverState,
};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::net::lookup_host;
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio::task::JoinHandle;
use tracing::info;

mod actor;
mod operate;
mod reduce;
#[cfg(test)]
mod tests;

use actor::run_actor;

const CONTROL_OBSERVATION_WINDOW: std::time::Duration = std::time::Duration::from_secs(5);
const CONTROL_RECHECK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
const MAX_USER_ACTIONS_BEFORE_DEBT: u8 = 8;
const MAX_USER_WORK_WINDOW: std::time::Duration = std::time::Duration::from_millis(500);

fn core_intents() -> Vec<ReceiverIntent> {
    vec![
        ReceiverIntent::SystemPower(denon_avr_domain::SystemPower::On),
        ReceiverIntent::MainZonePower(denon_avr_domain::ZonePower::On),
        ReceiverIntent::Zone2Power(denon_avr_domain::ZonePower::On),
        ReceiverIntent::Source(denon_avr_domain::SourceId::new("CD").expect("fixed source")),
        ReceiverIntent::Volume(denon_avr_domain::MasterVolume::Minimum),
        ReceiverIntent::Mute(denon_avr_domain::MuteState::Off),
        ReceiverIntent::SoundMode(denon_avr_domain::SoundModeIntent::Auto),
    ]
}

fn intent_for_field(field: CoreField) -> ReceiverIntent {
    match field {
        CoreField::SystemPower => ReceiverIntent::SystemPower(denon_avr_domain::SystemPower::On),
        CoreField::MainZonePower => ReceiverIntent::MainZonePower(denon_avr_domain::ZonePower::On),
        CoreField::Zone2Power => ReceiverIntent::Zone2Power(denon_avr_domain::ZonePower::On),
        CoreField::Source => {
            ReceiverIntent::Source(denon_avr_domain::SourceId::new("CD").expect("fixed source"))
        }
        CoreField::Volume => ReceiverIntent::Volume(denon_avr_domain::MasterVolume::Minimum),
        CoreField::Mute => ReceiverIntent::Mute(denon_avr_domain::MuteState::Off),
        CoreField::SoundMode => ReceiverIntent::SoundMode(denon_avr_domain::SoundModeIntent::Auto),
    }
}

enum Command {
    Observe(CoreField, oneshot::Sender<Result<(), OperationError>>),
    Synchronize(oneshot::Sender<Result<Readiness, OperationError>>),
    Operate(OperationRequest, oneshot::Sender<OperationOutcome>),
    Close(oneshot::Sender<Result<(), OperationError>>),
}

/// Cloneable handle to one serialized receiver connection.  Cloning this
/// handle only clones the mailbox sender; it never opens another socket.
pub struct X3800hSession {
    commands: mpsc::Sender<Command>,
    states: watch::Sender<ReceiverState>,
    actor: Arc<Mutex<Option<JoinHandle<()>>>>,
    closed: AtomicBool,
    inspection: Inspection,
}

/// Where the receiver's HTTP inspection reads go. The session records it at
/// connect time so a caller never supplies an address for a read.
#[derive(Debug, Clone)]
struct Inspection {
    host: String,
    timeout: std::time::Duration,
    app_command_port: u16,
}

impl X3800hSession {
    pub async fn connect(
        receiver: ReceiverId,
        host: &str,
        config: AvrSessionConfig,
    ) -> Result<Arc<Self>, OperationError> {
        let address = tokio::time::timeout(config.connect_timeout, lookup_host((host, 23)))
            .await
            .map_err(|_| {
                OperationError::new(
                    denon_avr_application::ports::OperationErrorKind::Timeout,
                    "resolving receiver",
                    "receiver address resolution timed out",
                )
            })?
            .map_err(|error| {
                OperationError::new(
                    denon_avr_application::ports::OperationErrorKind::Connection,
                    "resolving receiver",
                    error.to_string(),
                )
            })?
            .next()
            .ok_or_else(|| {
                OperationError::new(
                    denon_avr_application::ports::OperationErrorKind::Connection,
                    "resolving receiver",
                    "receiver host has no address",
                )
            })?;
        Self::connect_with_host(receiver, address, host.to_owned(), config).await
    }

    pub async fn connect_addr(
        receiver: ReceiverId,
        address: SocketAddr,
        config: AvrSessionConfig,
    ) -> Result<Arc<Self>, OperationError> {
        let host = address.ip().to_string();
        Self::connect_with_host(receiver, address, host, config).await
    }

    async fn connect_with_host(
        receiver: ReceiverId,
        address: SocketAddr,
        host: String,
        mut config: AvrSessionConfig,
    ) -> Result<Arc<Self>, OperationError> {
        info!(receiver = receiver.as_str(), %address, "connecting to X3800H receiver");
        // Canonical monitoring is long-lived. A temporary connection refusal
        // is degraded evidence, never a reason to silently stop monitoring.
        config.reconnect_indefinitely = true;
        let inspection = Inspection {
            host,
            timeout: config.connect_timeout,
            app_command_port: config.app_command_port,
        };
        let avr = AvrSession::connect_addr(address, config)
            .await
            .map_err(|error| {
                let mapped = OperationError::from(error);
                OperationError::new(
                    mapped.kind,
                    mapped.context,
                    initial_connection_guidance(mapped.message),
                )
            })?;
        let mut initial = ReceiverState::new(receiver);
        initial.establish_epoch(Epoch(1));
        let (states, _) = watch::channel(initial);
        let (commands, rx) = mpsc::channel(32);
        let actor = Arc::new(Mutex::new(None));
        let handle = Arc::new(Self {
            commands,
            states,
            actor: Arc::clone(&actor),
            closed: AtomicBool::new(false),
            inspection,
        });
        let task = tokio::spawn(run_actor(avr, rx, handle.states.clone()));
        *actor.lock().await = Some(task);
        Ok(handle)
    }

    pub fn current_state(&self) -> ReceiverState {
        self.states.borrow().clone()
    }

    /// The connection generation an inspection read is stamped with: the
    /// receiver epoch, which starts at one.
    fn generation(&self) -> u64 {
        self.states
            .borrow()
            .epoch
            .map_or(0, |epoch| epoch.0.saturating_sub(1))
    }

    /// Run a blocking HTTP read off the async runtime, mapping its failures
    /// into the session's error type. The reads only ever issue queries.
    async fn inspect<T: Send + 'static>(
        &self,
        what: &'static str,
        read: impl FnOnce(&Inspection, u64) -> std::io::Result<T> + Send + 'static,
    ) -> Result<T, OperationError> {
        use denon_avr_application::ports::OperationErrorKind;
        let inspection = self.inspection.clone();
        let generation = self.generation();
        tokio::task::spawn_blocking(move || read(&inspection, generation))
            .await
            .map_err(|error| {
                OperationError::new(OperationErrorKind::Stopped, what, error.to_string())
            })?
            .map_err(|error| {
                OperationError::new(OperationErrorKind::Connection, what, error.to_string())
            })
    }
}

fn initial_connection_guidance(message: impl Into<String>) -> String {
    format!(
        "{}; verify Network Control is enabled and no other Telnet client is connected",
        message.into()
    )
}

impl CanonicalReceiverSession for X3800hSession {
    fn state(&self) -> StateSubscription {
        StateSubscription::new(self.states.subscribe())
    }

    fn observe(
        &self,
        field: CoreField,
    ) -> denon_avr_application::ports::BoxFuture<'_, Result<(), OperationError>> {
        Box::pin(async move {
            let (reply, result) = oneshot::channel();
            self.commands
                .send(Command::Observe(field, reply))
                .await
                .map_err(|_| stopped())?;
            result.await.map_err(|_| stopped())?
        })
    }

    fn synchronize(
        &self,
    ) -> denon_avr_application::ports::BoxFuture<'_, Result<Readiness, OperationError>> {
        Box::pin(async move {
            let (reply, result) = oneshot::channel();
            self.commands
                .send(Command::Synchronize(reply))
                .await
                .map_err(|_| stopped())?;
            result.await.map_err(|_| stopped())?
        })
    }

    fn operate(
        &self,
        request: OperationRequest,
    ) -> denon_avr_application::ports::BoxFuture<'_, OperationOutcome> {
        Box::pin(async move {
            let (reply, result) = oneshot::channel();
            let operation = request.id;
            if self
                .commands
                .send(Command::Operate(request, reply))
                .await
                .is_err()
            {
                return OperationOutcome::Indeterminate {
                    operation,
                    dispatch: DispatchCertainty::NotDispatched,
                    reason: "receiver session stopped before operation admission".into(),
                };
            }
            result.await.unwrap_or(OperationOutcome::Indeterminate {
                operation,
                dispatch: DispatchCertainty::NotDispatched,
                reason: "receiver session stopped before operation completion".into(),
            })
        })
    }

    fn source_catalog(
        &self,
    ) -> denon_avr_application::ports::BoxFuture<
        '_,
        Result<denon_avr_domain::SourceCatalogObservation, OperationError>,
    > {
        Box::pin(self.inspect("source catalog", |target, generation| {
            SourceCatalogHttpClient::new(
                denon_avr_domain::ReceiverEndpoint {
                    host: target.host.clone(),
                    port: target.app_command_port,
                },
                target.timeout,
            )
            .and_then(|client| client.read(generation))
        }))
    }

    fn quick_select_names(
        &self,
    ) -> denon_avr_application::ports::BoxFuture<
        '_,
        Result<denon_avr_domain::QuickSelectNameObservation, OperationError>,
    > {
        Box::pin(self.inspect("Quick Select names", |target, generation| {
            QuickSelectNamesHttpClient::new(
                denon_avr_domain::ReceiverEndpoint {
                    host: target.host.clone(),
                    port: target.app_command_port,
                },
                target.timeout,
            )
            .and_then(|client| client.read(generation))
        }))
    }

    fn http_information(
        &self,
    ) -> denon_avr_application::ports::BoxFuture<
        '_,
        Result<denon_avr_domain::HttpInformationSnapshot, OperationError>,
    > {
        Box::pin(self.inspect("HTTP information", |target, generation| {
            HttpInformationHttpClient::new(target.host.clone(), target.timeout)
                .and_then(|client| client.read(generation))
        }))
    }

    fn close(&self) -> denon_avr_application::ports::BoxFuture<'_, Result<(), OperationError>> {
        Box::pin(async move {
            if self.closed.swap(true, Ordering::AcqRel) {
                return Ok(());
            }
            let (reply, result) = oneshot::channel();
            if self.commands.send(Command::Close(reply)).await.is_err() {
                return Err(stopped());
            }
            match tokio::time::timeout(std::time::Duration::from_millis(250), result).await {
                Ok(result) => result.map_err(|_| stopped())?,
                Err(_) => {
                    // The actor can be waiting for an indefinitely retrying
                    // transport. Abort the owned task; AvrSession's Drop
                    // implementation then aborts its socket owner as well.
                    if let Some(task) = self.actor.lock().await.take() {
                        task.abort();
                        let _ = task.await;
                    }
                    Ok(())
                }
            }
        })
    }
}

fn stopped() -> OperationError {
    OperationError::new(
        denon_avr_application::ports::OperationErrorKind::Stopped,
        "receiver session",
        "session actor stopped",
    )
}
