//! Application-owned receiver lifecycle and control coordinator.

use super::ports::{BoxFuture, OperationError, OperationErrorKind};
use crate::domain::{
    ConnectionState, DiscoveredReceiver, MainZoneControl, MainZoneEvent, MainZoneField,
    MainZoneSnapshot, MainZoneValue, Model, ModelCapabilities, ReceiverIdentity, StateAuthority,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// A typed session boundary. Protocol commands never cross into application code.
pub trait ReceiverSession: Send {
    fn query_field(
        &mut self,
        field: MainZoneField,
    ) -> BoxFuture<'_, Result<MainZoneValue, OperationError>>;
    fn execute_once(
        &mut self,
        control: MainZoneControl,
    ) -> BoxFuture<'_, Result<(), OperationError>>;
    fn next_event(&mut self) -> BoxFuture<'_, Result<SessionEvent, OperationError>>;
    fn close(&mut self) -> BoxFuture<'_, Result<(), OperationError>>;
}
pub trait SessionFactory: Send + Sync + 'static {
    fn connect(
        &self,
        identity: ReceiverIdentity,
    ) -> BoxFuture<'_, Result<Box<dyn ReceiverSession>, OperationError>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiverSelection {
    Saved {
        name: String,
        identity: ReceiverIdentity,
    },
    Discovered(DiscoveredReceiver),
    ExplicitHost(ReceiverIdentity),
}
impl ReceiverSelection {
    pub fn identity(&self) -> ReceiverIdentity {
        match self {
            Self::Saved { identity, .. } | Self::ExplicitHost(identity) => identity.clone(),
            Self::Discovered(r) => r.identity(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    Connection(ConnectionState),
    MainZone(MainZoneEvent),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiverCommand {
    Select(ReceiverSelection),
    Connect,
    Disconnect,
    Refresh,
    Control {
        control: MainZoneControl,
        expected_version: Option<u64>,
    },
    Shutdown,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lifecycle {
    NoReceiver,
    Selected,
    Connecting,
    Connected { generation: u64 },
    Reconnecting { generation: u64 },
    Disconnected,
    Stopping,
    Stopped,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlResult {
    Dispatched { version: u64 },
    Confirmed { snapshot: MainZoneSnapshot },
    NoOp { snapshot: MainZoneSnapshot },
    Conflict { expected: u64, current: u64 },
    Unsupported(String),
    Rejected(OperationError),
    TransportFailure(OperationError),
    Unconfirmed(OperationError),
    Cancelled,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diagnostic {
    ConnectionGeneration(u64),
    ReconnectAttempt { attempt: usize },
    Timeout { context: String },
    MalformedFrame { context: String },
    QueuePressure { queued: usize },
    Shutdown,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiverEvent {
    Lifecycle(Lifecycle),
    Snapshot(MainZoneSnapshot),
    FieldError {
        field: MainZoneField,
        error: crate::domain::FieldError,
    },
    ResourceVersionChanged(u64),
    Control(ControlResult),
    Cancelled,
    Diagnostic(Diagnostic),
}
pub trait Observability: Send + Sync {
    fn record(&self, event: Diagnostic);
}
#[derive(Default)]
pub struct NoopObservability;
impl Observability for NoopObservability {
    fn record(&self, _: Diagnostic) {}
}

pub trait Sleeper: Send + Sync {
    fn sleep(&self, duration: Duration) -> BoxFuture<'_, ()>;
}
#[derive(Default)]
pub struct TokioSleeper;
impl Sleeper for TokioSleeper {
    fn sleep(&self, duration: Duration) -> BoxFuture<'_, ()> {
        Box::pin(async move { tokio::time::sleep(duration).await })
    }
}

#[derive(Clone)]
pub struct ControllerConfig {
    pub power_on_quiet_time: Duration,
    pub confirmation_timeout: Duration,
    pub close_timeout: Duration,
    pub queue_capacity: usize,
    pub sleeper: Arc<dyn Sleeper>,
}
impl Default for ControllerConfig {
    fn default() -> Self {
        Self {
            power_on_quiet_time: Duration::from_secs(1),
            confirmation_timeout: Duration::from_secs(2),
            close_timeout: Duration::from_secs(1),
            queue_capacity: 32,
            sleeper: Arc::new(TokioSleeper),
        }
    }
}

pub struct ControllerHandle {
    commands: mpsc::Sender<Envelope>,
    events: mpsc::Receiver<ReceiverEvent>,
}
pub type CommandReply = Option<ReceiverEvent>;
impl ControllerHandle {
    pub async fn send(&self, command: ReceiverCommand) -> Result<CommandReply, OperationError> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Envelope { command, reply })
            .await
            .map_err(|_| stopped())?;
        result.await.map_err(|_| stopped())?
    }
    pub async fn next_event(&mut self) -> Option<ReceiverEvent> {
        self.events.recv().await
    }
    pub async fn select(
        &self,
        selection: ReceiverSelection,
    ) -> Result<CommandReply, OperationError> {
        self.send(ReceiverCommand::Select(selection)).await
    }
    pub async fn connect(&self) -> Result<CommandReply, OperationError> {
        self.send(ReceiverCommand::Connect).await
    }
    pub async fn disconnect(&self) -> Result<CommandReply, OperationError> {
        self.send(ReceiverCommand::Disconnect).await
    }
    pub async fn refresh(&self) -> Result<CommandReply, OperationError> {
        self.send(ReceiverCommand::Refresh).await
    }
    pub async fn control(
        &self,
        control: MainZoneControl,
        expected_version: Option<u64>,
    ) -> Result<CommandReply, OperationError> {
        self.send(ReceiverCommand::Control {
            control,
            expected_version,
        })
        .await
    }
    pub async fn shutdown(&self) -> Result<CommandReply, OperationError> {
        self.send(ReceiverCommand::Shutdown).await
    }
}

pub struct ReceiverController;
impl ReceiverController {
    pub fn spawn<F: SessionFactory>(factory: F, config: ControllerConfig) -> ControllerHandle {
        Self::spawn_with_observability(factory, config, Arc::new(NoopObservability))
    }
    pub fn spawn_with_observability<F: SessionFactory>(
        factory: F,
        config: ControllerConfig,
        observability: Arc<dyn Observability>,
    ) -> ControllerHandle {
        let (commands, mut rx) = mpsc::channel::<Envelope>(config.queue_capacity.max(1));
        let (events, event_rx) = mpsc::channel::<ReceiverEvent>(config.queue_capacity.max(1));
        tokio::spawn(async move {
            let mut state = State {
                factory,
                config,
                observability,
                selection: None,
                session: None,
                snapshot: MainZoneSnapshot::default(),
                lifecycle: Lifecycle::NoReceiver,
                generation: 0,
            };
            while let Some(envelope) = rx.recv().await {
                let stop = matches!(envelope.command, ReceiverCommand::Shutdown);
                let result = state.handle(envelope.command, &events).await;
                let _ = envelope.reply.send(result);
                if stop {
                    break;
                }
            }
            let _ = events
                .send(ReceiverEvent::Lifecycle(Lifecycle::Stopped))
                .await;
        });
        ControllerHandle {
            commands,
            events: event_rx,
        }
    }
}
struct Envelope {
    command: ReceiverCommand,
    reply: oneshot::Sender<Result<CommandReply, OperationError>>,
}
struct State<F> {
    factory: F,
    config: ControllerConfig,
    observability: Arc<dyn Observability>,
    selection: Option<ReceiverSelection>,
    session: Option<Box<dyn ReceiverSession>>,
    snapshot: MainZoneSnapshot,
    lifecycle: Lifecycle,
    generation: u64,
}
impl<F: SessionFactory> State<F> {
    async fn handle(
        &mut self,
        command: ReceiverCommand,
        events: &mpsc::Sender<ReceiverEvent>,
    ) -> Result<CommandReply, OperationError> {
        match command {
            ReceiverCommand::Select(s) => {
                self.disconnect().await?;
                self.selection = Some(s);
                self.snapshot.invalidate();
                Self::emit(events, ReceiverEvent::Lifecycle(Lifecycle::Selected)).await;
                Ok(Some(ReceiverEvent::Snapshot(self.snapshot.clone())))
            }
            ReceiverCommand::Connect => {
                self.connect(events).await?;
                Ok(Some(ReceiverEvent::Lifecycle(self.lifecycle.clone())))
            }
            ReceiverCommand::Disconnect => {
                self.disconnect().await?;
                Ok(Some(ReceiverEvent::Lifecycle(self.lifecycle.clone())))
            }
            ReceiverCommand::Refresh => {
                self.refresh(events).await?;
                Ok(Some(ReceiverEvent::Snapshot(self.snapshot.clone())))
            }
            ReceiverCommand::Control {
                control,
                expected_version,
            } => {
                let result = self.control(control, expected_version, events).await;
                Self::emit(events, ReceiverEvent::Control(result.clone())).await;
                Ok(Some(ReceiverEvent::Control(result)))
            }
            ReceiverCommand::Shutdown => {
                self.lifecycle = Lifecycle::Stopping;
                Self::emit(events, ReceiverEvent::Lifecycle(Lifecycle::Stopping)).await;
                self.disconnect().await?;
                self.observability.record(Diagnostic::Shutdown);
                Ok(None)
            }
        }
    }
    async fn connect(
        &mut self,
        events: &mpsc::Sender<ReceiverEvent>,
    ) -> Result<(), OperationError> {
        let selection = self
            .selection
            .clone()
            .ok_or_else(|| invalid("connecting", "no receiver selected"))?;
        self.lifecycle = Lifecycle::Connecting;
        Self::emit(events, ReceiverEvent::Lifecycle(Lifecycle::Connecting)).await;
        match self.factory.connect(selection.identity()).await {
            Ok(s) => {
                self.session = Some(s);
                self.generation += 1;
                self.lifecycle = Lifecycle::Connected {
                    generation: self.generation,
                };
                self.observability
                    .record(Diagnostic::ConnectionGeneration(self.generation));
                Self::emit(events, ReceiverEvent::Lifecycle(self.lifecycle.clone())).await;
                Ok(())
            }
            Err(e) => {
                self.lifecycle = Lifecycle::Disconnected;
                Self::emit(events, ReceiverEvent::Lifecycle(Lifecycle::Disconnected)).await;
                Err(e)
            }
        }
    }
    async fn disconnect(&mut self) -> Result<(), OperationError> {
        if let Some(mut s) = self.session.take() {
            tokio::time::timeout(self.config.close_timeout, s.close())
                .await
                .map_err(|_| invalid("closing session", "close deadline expired"))??;
        }
        self.lifecycle = if self.selection.is_some() {
            Lifecycle::Selected
        } else {
            Lifecycle::NoReceiver
        };
        Ok(())
    }
    async fn refresh(
        &mut self,
        events: &mpsc::Sender<ReceiverEvent>,
    ) -> Result<(), OperationError> {
        if self.session.is_none() {
            self.connect(events).await?;
        }
        let before = self.snapshot.resource_version();
        for field in MainZoneField::ALL {
            let result = {
                let s = self.session.as_mut().ok_or_else(stopped)?;
                s.query_field(field).await
            };
            match result {
                Ok(value) => self
                    .snapshot
                    .set_value(value, StateAuthority::Authoritative),
                Err(error) => {
                    let field_error = field_error(error);
                    self.snapshot.set_error(field, field_error.clone());
                    Self::emit(
                        events,
                        ReceiverEvent::FieldError {
                            field,
                            error: field_error,
                        },
                    )
                    .await;
                }
            }
        }
        if self.snapshot.resource_version() != before {
            Self::emit(
                events,
                ReceiverEvent::ResourceVersionChanged(self.snapshot.resource_version()),
            )
            .await;
        }
        Self::emit(events, ReceiverEvent::Snapshot(self.snapshot.clone())).await;
        Ok(())
    }
    async fn control(
        &mut self,
        control: MainZoneControl,
        expected: Option<u64>,
        events: &mpsc::Sender<ReceiverEvent>,
    ) -> ControlResult {
        if let Some(selection) = &self.selection {
            let model = selection
                .identity()
                .model
                .as_deref()
                .map(Model::from_reported)
                .unwrap_or(Model::Unknown);
            if !ModelCapabilities::for_model(model).supports_control(&control) {
                return ControlResult::Unsupported(
                    "selected receiver does not have validated support for this control".into(),
                );
            }
        }
        if self.session.is_none() {
            if let Err(e) = self.connect(events).await {
                return ControlResult::Rejected(e);
            }
        }
        if let Some(expected) = expected {
            if expected != self.snapshot.resource_version() {
                return ControlResult::Conflict {
                    expected,
                    current: self.snapshot.resource_version(),
                };
            }
        }
        let field = control_field(&control);
        if self
            .snapshot
            .value(field)
            .is_some_and(|v| control_matches(&control, &v))
        {
            return ControlResult::NoOp {
                snapshot: self.snapshot.clone(),
            };
        }
        let Some(s) = self.session.as_mut() else {
            return ControlResult::Rejected(stopped());
        };
        if let Err(e) = s.execute_once(control.clone()).await {
            return ControlResult::TransportFailure(e);
        }
        if matches!(
            control,
            MainZoneControl::Power(crate::domain::PowerState::On)
        ) {
            self.config
                .sleeper
                .sleep(self.config.power_on_quiet_time)
                .await;
        }
        match tokio::time::timeout(self.config.confirmation_timeout, self.refresh(events)).await {
            Ok(Ok(()))
                if self
                    .snapshot
                    .value(field)
                    .is_some_and(|v| control_matches(&control, &v)) =>
            {
                ControlResult::Confirmed {
                    snapshot: self.snapshot.clone(),
                }
            }
            Ok(Ok(())) => ControlResult::Unconfirmed(OperationError::new(
                OperationErrorKind::Unavailable,
                "confirming control",
                "receiver did not report requested value",
            )),
            Ok(Err(e)) => ControlResult::Unconfirmed(e),
            Err(_) => ControlResult::Unconfirmed(OperationError::new(
                OperationErrorKind::Timeout,
                "confirming control",
                "confirmation deadline expired",
            )),
        }
    }
    async fn emit(events: &mpsc::Sender<ReceiverEvent>, event: ReceiverEvent) {
        let _ = events.send(event).await;
    }
}
fn stopped() -> OperationError {
    invalid("controller", "controller is stopped")
}
fn invalid(context: &'static str, message: impl Into<String>) -> OperationError {
    OperationError::new(OperationErrorKind::Stopped, context, message)
}
fn field_error(e: OperationError) -> crate::domain::FieldError {
    crate::domain::FieldError {
        kind: match e.kind {
            OperationErrorKind::Timeout => crate::domain::FieldErrorKind::Timeout,
            OperationErrorKind::Disconnected | OperationErrorKind::Connection => {
                crate::domain::FieldErrorKind::Disconnected
            }
            OperationErrorKind::Malformed => crate::domain::FieldErrorKind::Malformed,
            OperationErrorKind::Unsupported => crate::domain::FieldErrorKind::Unsupported,
            _ => crate::domain::FieldErrorKind::Unavailable,
        },
        message: e.to_string(),
    }
}
fn control_field(c: &MainZoneControl) -> MainZoneField {
    match c {
        MainZoneControl::Power(_) => MainZoneField::Power,
        MainZoneControl::Input(_) => MainZoneField::Input,
        MainZoneControl::Volume(_) => MainZoneField::Volume,
        MainZoneControl::Mute(_) => MainZoneField::Mute,
        MainZoneControl::SurroundMode(_) => MainZoneField::SurroundMode,
    }
}
fn control_matches(c: &MainZoneControl, v: &MainZoneValue) -> bool {
    match (c, v) {
        (MainZoneControl::Power(a), MainZoneValue::Power(b)) => a == b,
        (MainZoneControl::Input(a), MainZoneValue::Input(b)) => a == b,
        (MainZoneControl::Volume(a), MainZoneValue::Volume(b)) => {
            b.level().ok().as_ref() == Some(a)
        }
        (MainZoneControl::Mute(a), MainZoneValue::Mute(b)) => a == b,
        (MainZoneControl::SurroundMode(a), MainZoneValue::SurroundMode(b)) => a == b,
        _ => false,
    }
}
