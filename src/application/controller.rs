//! Application-owned receiver lifecycle and control coordinator.

use super::ports::{BoxFuture, OperationError, OperationErrorKind};
use crate::domain::{
    AudioContextSnapshot, ConnectionState, DiscoveredReceiver, EqStatus, MainZoneControl,
    MainZoneEvent, MainZoneField, MainZoneSnapshot, MainZoneValue, Model, ModelCapabilities,
    QuickSelectRecallConfirmation, QuickSelectRecallOutcome, QuickSelectSlot, QuickSelectSnapshot,
    ReceiverIdentity, StateAuthority, ValidatedPhase8Capabilities,
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
    RefreshPhase8,
    RecallQuickSelect {
        slot: QuickSelectSlot,
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
    QuickSelect(Box<QuickSelectSnapshot>),
    QuickSelectRecall(QuickSelectRecallOutcome),
    EqStatus(Box<EqStatus>),
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
    /// Explicit model/firmware validation output. `None` keeps Phase 8
    /// operations read-only even for a known X3800H.
    pub validated_phase8: Option<ValidatedPhase8Capabilities>,
}
impl Default for ControllerConfig {
    fn default() -> Self {
        Self {
            power_on_quiet_time: Duration::from_secs(1),
            confirmation_timeout: Duration::from_secs(2),
            close_timeout: Duration::from_secs(1),
            queue_capacity: 32,
            sleeper: Arc::new(TokioSleeper),
            validated_phase8: None,
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
    pub async fn refresh_phase8(&self) -> Result<CommandReply, OperationError> {
        self.send(ReceiverCommand::RefreshPhase8).await
    }
    pub async fn recall_quick_select(
        &self,
        slot: QuickSelectSlot,
        expected_version: Option<u64>,
    ) -> Result<CommandReply, OperationError> {
        self.send(ReceiverCommand::RecallQuickSelect {
            slot,
            expected_version,
        })
        .await
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
            let validated_phase8 = config.validated_phase8;
            let mut state = State {
                factory,
                config,
                observability,
                selection: None,
                session: None,
                snapshot: MainZoneSnapshot::default(),
                lifecycle: Lifecycle::NoReceiver,
                generation: 0,
                quick_select: QuickSelectSnapshot::default(),
                eq_status: EqStatus::default(),
                validated_phase8,
            };
            while let Some(envelope) = rx.recv().await {
                let stop = matches!(envelope.command, ReceiverCommand::Shutdown);
                let result = state.handle(envelope.command, &events).await;
                let _ = envelope.reply.send(result);
                if stop {
                    break;
                }
                state.drain_session_events(&events).await;
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
    quick_select: QuickSelectSnapshot,
    eq_status: EqStatus,
    validated_phase8: Option<ValidatedPhase8Capabilities>,
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
            ReceiverCommand::RefreshPhase8 => {
                self.refresh_phase8(events).await?;
                Ok(Some(ReceiverEvent::EqStatus(Box::new(
                    self.eq_status.clone(),
                ))))
            }
            ReceiverCommand::RecallQuickSelect {
                slot,
                expected_version,
            } => {
                Self::emit(
                    events,
                    ReceiverEvent::QuickSelectRecall(QuickSelectRecallOutcome::Pending { slot }),
                )
                .await;
                let result = self
                    .recall_quick_select(slot, expected_version, events)
                    .await;
                Self::emit(events, ReceiverEvent::QuickSelectRecall(result)).await;
                Ok(Some(ReceiverEvent::QuickSelect(Box::new(
                    self.quick_select.clone(),
                ))))
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
                self.invalidate_phase8();
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
        self.invalidate_phase8();
        Ok(())
    }

    fn invalidate_phase8(&mut self) {
        self.quick_select.invalidate();
        self.eq_status.invalidate();
        self.quick_select.generation = self.generation;
        self.eq_status.generation = self.generation;
    }

    async fn drain_session_events(&mut self, events: &mpsc::Sender<ReceiverEvent>) {
        loop {
            let result = {
                let Some(session) = self.session.as_mut() else {
                    return;
                };
                match tokio::time::timeout(Duration::from_millis(1), session.next_event()).await {
                    Ok(result) => result,
                    Err(_) => return,
                }
            };
            match result {
                Ok(SessionEvent::Connection(ConnectionState::Reconnecting)) => {
                    self.generation = self.generation.saturating_add(1);
                    self.lifecycle = Lifecycle::Reconnecting {
                        generation: self.generation,
                    };
                    self.snapshot.invalidate();
                    self.invalidate_phase8();
                    Self::emit(events, ReceiverEvent::Lifecycle(self.lifecycle.clone())).await;
                }
                Ok(SessionEvent::Connection(ConnectionState::Connected)) => {
                    if matches!(self.lifecycle, Lifecycle::Reconnecting { .. }) {
                        self.lifecycle = Lifecycle::Connected {
                            generation: self.generation,
                        };
                        Self::emit(events, ReceiverEvent::Lifecycle(self.lifecycle.clone())).await;
                    }
                }
                Ok(SessionEvent::Connection(ConnectionState::Disconnected)) => {
                    self.lifecycle = Lifecycle::Disconnected;
                    self.snapshot.invalidate();
                    self.invalidate_phase8();
                    Self::emit(events, ReceiverEvent::Lifecycle(self.lifecycle.clone())).await;
                }
                Ok(SessionEvent::MainZone(event)) => {
                    self.snapshot.apply_event(event);
                    Self::emit(events, ReceiverEvent::Snapshot(self.snapshot.clone())).await;
                }
                Err(error) => {
                    self.observability.record(Diagnostic::Timeout {
                        context: error.to_string(),
                    });
                    self.lifecycle = Lifecycle::Disconnected;
                    self.snapshot.invalidate();
                    self.invalidate_phase8();
                    Self::emit(events, ReceiverEvent::Lifecycle(self.lifecycle.clone())).await;
                    return;
                }
            }
        }
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
        let context = {
            let s = self.session.as_mut().ok_or_else(stopped)?;
            s.query_audio_context().await
        };
        self.snapshot.set_audio_context(context);
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
    async fn refresh_phase8(
        &mut self,
        events: &mpsc::Sender<ReceiverEvent>,
    ) -> Result<(), OperationError> {
        let capabilities = self.capabilities();
        if !capabilities.eq_status {
            return Err(OperationError::new(
                OperationErrorKind::Unsupported,
                "EQ status",
                "selected receiver has no validated EQ status capability",
            ));
        }
        if self.session.is_none() {
            self.connect(events).await?;
        }
        // Quick Select is an execute-only capability.  Denon does not expose
        // a validated per-slot query for the registered fields, so refresh
        // must not invent availability or names from a guessed response.
        if capabilities.eq_status {
            let previous_eq_status = self.eq_status.clone();
            let result = {
                let session = self.session.as_mut().ok_or_else(stopped)?;
                session.query_eq_status().await
            };
            match result {
                Ok(mut refreshed_eq_status) => {
                    // The controller owns lifecycle generations. Transport
                    // adapters may use their own counters internally.
                    refreshed_eq_status.generation = self.generation;
                    refreshed_eq_status.preserve_failed_observations(&previous_eq_status);
                    if let Some(mode) = self.snapshot.audio_context.current_mode.value.known() {
                        refreshed_eq_status.apply_mode_restrictions(mode.as_str());
                    }
                    self.eq_status = refreshed_eq_status;
                    Self::emit(
                        events,
                        ReceiverEvent::EqStatus(Box::new(self.eq_status.clone())),
                    )
                    .await;
                }
                Err(error) => {
                    Self::emit(
                        events,
                        ReceiverEvent::Diagnostic(Diagnostic::Timeout {
                            context: format!("refreshing EQ status: {error}"),
                        }),
                    )
                    .await
                }
            }
        }
        Ok(())
    }
    async fn recall_quick_select(
        &mut self,
        slot: QuickSelectSlot,
        expected: Option<u64>,
        events: &mpsc::Sender<ReceiverEvent>,
    ) -> QuickSelectRecallOutcome {
        if !self.capabilities().quick_select_recall {
            return QuickSelectRecallOutcome::Unsupported(
                "selected receiver has no validated Quick Select capability".into(),
            );
        }
        if let Some(expected) = expected {
            let current = self.quick_select.resource_version();
            if expected != current {
                return QuickSelectRecallOutcome::Conflict { expected, current };
            }
        }
        if self.session.is_none() {
            if let Err(error) = self.connect(events).await {
                return QuickSelectRecallOutcome::Rejected(error.to_string());
            }
        }
        let Some(session) = self.session.as_mut() else {
            return QuickSelectRecallOutcome::Rejected("receiver is not connected".into());
        };
        match session.recall_quick_select(slot).await {
            Ok(QuickSelectRecallConfirmation::Authoritative) => {
                QuickSelectRecallOutcome::Confirmed { slot }
            }
            Ok(QuickSelectRecallConfirmation::Dispatched) => QuickSelectRecallOutcome::Unconfirmed(
                "receiver acknowledged the preset command; resulting state was not authoritative"
                    .into(),
            ),
            Err(error) if error.kind == OperationErrorKind::Unsupported => {
                QuickSelectRecallOutcome::Unsupported(error.to_string())
            }
            Err(error)
                if matches!(
                    error.kind,
                    OperationErrorKind::Timeout
                        | OperationErrorKind::Disconnected
                        | OperationErrorKind::Connection
                ) =>
            {
                QuickSelectRecallOutcome::Unconfirmed(error.to_string())
            }
            Err(error) => QuickSelectRecallOutcome::TransportFailure(error.to_string()),
        }
    }
    fn capabilities(&self) -> ModelCapabilities {
        let model = self
            .selection
            .as_ref()
            .and_then(|s| s.identity().model.clone())
            .map(|value| Model::from_reported(&value))
            .unwrap_or(Model::Unknown);
        ModelCapabilities::for_model(model)
            .with_validated_phase8(self.validated_phase8.unwrap_or_default())
    }
    async fn control(
        &mut self,
        control: MainZoneControl,
        expected: Option<u64>,
        events: &mpsc::Sender<ReceiverEvent>,
    ) -> ControlResult {
        let capabilities = self
            .selection
            .as_ref()
            .map(|selection| {
                let model = selection
                    .identity()
                    .model
                    .as_deref()
                    .map(Model::from_reported)
                    .unwrap_or(Model::Unknown);
                ModelCapabilities::for_model(model)
            })
            .unwrap_or_else(|| ModelCapabilities::for_model(Model::Unknown));
        if self.selection.is_some() && !capabilities.supports_control(&control) {
            return ControlResult::Unsupported(
                "selected receiver does not have validated support for this control".into(),
            );
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
            .is_some_and(|v| control_matches(&control, &v, &capabilities))
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
                    .is_some_and(|v| control_matches(&control, &v, &capabilities)) =>
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
        MainZoneControl::ListeningModeGroup(_) => MainZoneField::SurroundMode,
    }
}
fn control_matches(
    c: &MainZoneControl,
    v: &MainZoneValue,
    capabilities: &ModelCapabilities,
) -> bool {
    match (c, v) {
        (MainZoneControl::Power(a), MainZoneValue::Power(b)) => a == b,
        (MainZoneControl::Input(a), MainZoneValue::Input(b)) => a == b,
        (MainZoneControl::Volume(a), MainZoneValue::Volume(b)) => {
            b.level().ok().as_ref() == Some(a)
        }
        (MainZoneControl::Mute(a), MainZoneValue::Mute(b)) => a == b,
        (MainZoneControl::SurroundMode(a), MainZoneValue::SurroundMode(b)) => a == b,
        (MainZoneControl::ListeningModeGroup(group), MainZoneValue::SurroundMode(actual)) => {
            capabilities
                .listening_modes(*group)
                .contains(&actual.as_str())
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AudioContextSnapshot, QuickSelectRecallConfirmation, ReceiverIdentity};

    #[derive(Clone)]
    struct FakeFactory;

    struct FakeSession;

    impl SessionFactory for FakeFactory {
        fn connect(
            &self,
            _identity: ReceiverIdentity,
        ) -> BoxFuture<'_, Result<Box<dyn ReceiverSession>, OperationError>> {
            Box::pin(async { Ok(Box::new(FakeSession) as Box<dyn ReceiverSession>) })
        }
    }

    impl ReceiverSession for FakeSession {
        fn query_field(
            &mut self,
            _field: MainZoneField,
        ) -> BoxFuture<'_, Result<MainZoneValue, OperationError>> {
            Box::pin(async {
                Err(OperationError::new(
                    OperationErrorKind::Unsupported,
                    "test",
                    "not needed",
                ))
            })
        }

        fn execute_once(
            &mut self,
            _control: MainZoneControl,
        ) -> BoxFuture<'_, Result<(), OperationError>> {
            Box::pin(async { Ok(()) })
        }

        fn query_audio_context(&mut self) -> BoxFuture<'_, AudioContextSnapshot> {
            Box::pin(async { AudioContextSnapshot::default() })
        }

        fn next_event(&mut self) -> BoxFuture<'_, Result<SessionEvent, OperationError>> {
            Box::pin(async {
                Err(OperationError::new(
                    OperationErrorKind::Stopped,
                    "test",
                    "event stream is idle",
                ))
            })
        }

        fn close(&mut self) -> BoxFuture<'_, Result<(), OperationError>> {
            Box::pin(async { Ok(()) })
        }

        fn recall_quick_select(
            &mut self,
            _slot: QuickSelectSlot,
        ) -> BoxFuture<'_, Result<QuickSelectRecallConfirmation, OperationError>> {
            Box::pin(async { Ok(QuickSelectRecallConfirmation::Authoritative) })
        }

        fn query_eq_status(&mut self) -> BoxFuture<'_, Result<EqStatus, OperationError>> {
            Box::pin(async {
                Ok(EqStatus {
                    dynamic_eq: crate::domain::EqState::On,
                    generation: 999,
                    freshness: crate::domain::Freshness::Live,
                    ..EqStatus::default()
                })
            })
        }
    }

    fn selection() -> ReceiverSelection {
        ReceiverSelection::ExplicitHost(ReceiverIdentity {
            host: "test.invalid".into(),
            model: Some("AVR-X3800H".into()),
            friendly_name: None,
        })
    }

    async fn next_recall_outcome(handle: &mut ControllerHandle) -> QuickSelectRecallOutcome {
        for _ in 0..8 {
            if let Ok(Some(ReceiverEvent::QuickSelectRecall(outcome))) =
                tokio::time::timeout(Duration::from_millis(50), handle.next_event()).await
            {
                if !matches!(outcome, QuickSelectRecallOutcome::Pending { .. }) {
                    return outcome;
                }
            }
        }
        panic!("no Quick Select recall outcome was emitted")
    }

    #[tokio::test]
    async fn default_profile_rejects_unvalidated_quick_select() {
        let mut handle = ReceiverController::spawn(FakeFactory, ControllerConfig::default());
        handle.select(selection()).await.unwrap();
        handle
            .recall_quick_select(QuickSelectSlot::new(1).unwrap(), None)
            .await
            .unwrap();
        assert!(matches!(
            next_recall_outcome(&mut handle).await,
            QuickSelectRecallOutcome::Unsupported(_)
        ));
    }

    #[tokio::test]
    async fn validated_profile_preserves_authoritative_recall_result() {
        let config = ControllerConfig {
            validated_phase8: Some(ValidatedPhase8Capabilities {
                quick_select_recall: true,
                eq_status: true,
            }),
            ..ControllerConfig::default()
        };
        let mut handle = ReceiverController::spawn(FakeFactory, config);
        handle.select(selection()).await.unwrap();
        handle
            .recall_quick_select(QuickSelectSlot::new(2).unwrap(), None)
            .await
            .unwrap();
        assert_eq!(
            next_recall_outcome(&mut handle).await,
            QuickSelectRecallOutcome::Confirmed {
                slot: QuickSelectSlot::new(2).unwrap()
            }
        );
    }

    #[tokio::test]
    async fn stale_quick_select_version_is_rejected_before_dispatch() {
        let config = ControllerConfig {
            validated_phase8: Some(ValidatedPhase8Capabilities {
                quick_select_recall: true,
                eq_status: true,
            }),
            ..ControllerConfig::default()
        };
        let mut handle = ReceiverController::spawn(FakeFactory, config);
        handle.select(selection()).await.unwrap();
        handle
            .recall_quick_select(QuickSelectSlot::new(1).unwrap(), Some(0))
            .await
            .unwrap();
        assert_eq!(
            next_recall_outcome(&mut handle).await,
            QuickSelectRecallOutcome::Conflict {
                expected: 0,
                current: 1
            }
        );
    }

    #[tokio::test]
    async fn eq_refresh_returns_status_with_controller_generation() {
        let config = ControllerConfig {
            validated_phase8: Some(ValidatedPhase8Capabilities {
                quick_select_recall: false,
                eq_status: true,
            }),
            ..ControllerConfig::default()
        };
        let handle = ReceiverController::spawn(FakeFactory, config);
        handle.select(selection()).await.unwrap();
        let reply = handle.refresh_phase8().await.unwrap();
        let Some(ReceiverEvent::EqStatus(status)) = reply else {
            panic!("EQ refresh did not return an EQ status snapshot")
        };
        assert_eq!(status.dynamic_eq, crate::domain::EqState::On);
        assert_eq!(status.generation, 1);
    }

    #[tokio::test]
    async fn quick_select_only_profile_rejects_status_refresh() {
        let config = ControllerConfig {
            validated_phase8: Some(ValidatedPhase8Capabilities {
                quick_select_recall: true,
                eq_status: false,
            }),
            ..ControllerConfig::default()
        };
        let handle = ReceiverController::spawn(FakeFactory, config);
        handle.select(selection()).await.unwrap();
        let error = handle.refresh_phase8().await.unwrap_err();
        assert_eq!(error.kind, OperationErrorKind::Unsupported);
    }
}
