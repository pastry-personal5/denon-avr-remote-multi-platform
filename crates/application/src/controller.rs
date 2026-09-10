//! Application-owned receiver lifecycle and control coordinator.

use super::ports::{
    BoxFuture, OperationError, OperationErrorKind, ReceiverSession, SessionEvent, SessionFactory,
};
use crate::main_zone_control::{
    admit_main_zone_control, control_field, control_matches, ControlAdmission,
};
use denon_avr_domain::{
    ConnectionState, DiscoveredReceiver, EqStatus, MainZoneControl, MainZoneEvent, MainZoneField,
    MainZoneSnapshot, MainZoneValue, Model, ModelCapabilities, PowerState,
    QuickSelectEqCapabilities, QuickSelectNameObservation, QuickSelectRecallOutcome,
    QuickSelectSlot, QuickSelectSnapshot, ReceiverIdentity, SourceCatalog,
    SourceCatalogCapabilities, SourceCatalogObservation, StateAuthority, Zone2Control,
    Zone2Snapshot,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

const MAX_SESSION_EVENTS_PER_DRAIN: usize = 32;

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
pub enum ReceiverCommand {
    Select(ReceiverSelection),
    Connect,
    Disconnect,
    Refresh,
    Control {
        control: MainZoneControl,
        expected_version: Option<u64>,
    },
    ControlZone2 {
        power: PowerState,
    },
    RefreshQuickSelectEq,
    RefreshSourceCatalog,
    RefreshQuickSelectNames,
    RefreshHttpInformation,
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
    Zone2Snapshot(Zone2Snapshot),
    FieldError {
        field: MainZoneField,
        error: denon_avr_domain::FieldError,
    },
    ResourceVersionChanged(u64),
    Control(ControlResult),
    Cancelled,
    Diagnostic(Diagnostic),
    QuickSelect(Box<QuickSelectSnapshot>),
    QuickSelectRecall(QuickSelectRecallOutcome),
    EqStatus(Box<EqStatus>),
    /// Catalog state plus the protocol/transport evidence that produced it.
    SourceCatalog(Box<SourceCatalogObservation>),
    QuickSelectNames(Box<QuickSelectNameObservation>),
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
    /// Explicit model/firmware validation output. `None` keeps Quick Select/EQ
    /// operations read-only even for a known X3800H.
    pub validated_quick_select_eq: Option<QuickSelectEqCapabilities>,
    /// Explicit model/firmware source-catalog validation. The default keeps the
    /// candidate AppCommand reader disabled.
    pub validated_source_catalog: Option<SourceCatalogCapabilities>,
}
impl Default for ControllerConfig {
    fn default() -> Self {
        Self {
            power_on_quiet_time: Duration::from_secs(1),
            confirmation_timeout: Duration::from_secs(2),
            close_timeout: Duration::from_secs(1),
            queue_capacity: 32,
            sleeper: Arc::new(TokioSleeper),
            validated_quick_select_eq: None,
            validated_source_catalog: None,
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
    pub async fn control_zone2(&self, power: PowerState) -> Result<CommandReply, OperationError> {
        self.send(ReceiverCommand::ControlZone2 { power }).await
    }
    pub async fn shutdown(&self) -> Result<CommandReply, OperationError> {
        self.send(ReceiverCommand::Shutdown).await
    }
    pub async fn refresh_quick_select_eq(&self) -> Result<CommandReply, OperationError> {
        self.send(ReceiverCommand::RefreshQuickSelectEq).await
    }
    pub async fn refresh_source_catalog(&self) -> Result<CommandReply, OperationError> {
        self.send(ReceiverCommand::RefreshSourceCatalog).await
    }
    pub async fn refresh_quick_select_names(&self) -> Result<CommandReply, OperationError> {
        self.send(ReceiverCommand::RefreshQuickSelectNames).await
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
            let validated_quick_select_eq = config.validated_quick_select_eq;
            let validated_source_catalog = config.validated_source_catalog;
            let mut state = State {
                factory,
                config,
                observability,
                selection: None,
                session: None,
                snapshot: MainZoneSnapshot::default(),
                zone2: Zone2Snapshot::default(),
                lifecycle: Lifecycle::NoReceiver,
                generation: 0,
                quick_select: QuickSelectSnapshot::default(),
                eq_status: EqStatus::default(),
                validated_quick_select_eq,
                validated_source_catalog,
                source_catalog: SourceCatalog::default(),
                source_catalog_observation: None,
                quick_select_names_observation: None,
            };
            let mut monitor_tick = tokio::time::interval(Duration::from_millis(25));
            monitor_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut reconcile_interval = tokio::time::interval(Duration::from_secs(15));
            reconcile_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // The first automatic read follows an authoritative status refresh,
            // not controller startup.
            monitor_tick.tick().await;
            reconcile_interval.tick().await;
            loop {
                tokio::select! {
                    envelope = rx.recv() => {
                        let Some(envelope) = envelope else { break; };
                        let stop = matches!(envelope.command, ReceiverCommand::Shutdown);
                        let refresh_supplemental = matches!(envelope.command, ReceiverCommand::Refresh);
                        let result = state.handle(envelope.command, &events).await;
                        if stop {
                            let _ = envelope.reply.send(result);
                            break;
                        }
                        // Consume connection/session notifications before releasing the
                        // command reply. A receiver can report a reconnect while a
                        // refresh is in flight; replying first lets the GUI render a
                        // confirmed power value and then immediately overwrite it with
                        // the invalidated snapshot emitted by the reconnect drain.
                        state.drain_session_events(&events).await;
                        let _ = envelope.reply.send(result);
                        // Core status is now published and the GUI can leave
                        // its launch screen. Optional reads must not delay
                        // that reply when an endpoint stalls or is unsupported.
                        if refresh_supplemental {
                            let _ = state.refresh_http_information(&events).await;
                            let _ = state.refresh_quick_select_names(&events).await;
                        }
                    }
                    _ = monitor_tick.tick() => {
                        state.drain_session_events(&events).await;
                    }
                    _ = reconcile_interval.tick() => {
                        let _ = state.refresh(&events).await;
                        let _ = state.refresh_zone2(&events).await;
                        let _ = state.refresh_http_information(&events).await;
                    }
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
    zone2: Zone2Snapshot,
    lifecycle: Lifecycle,
    generation: u64,
    quick_select: QuickSelectSnapshot,
    eq_status: EqStatus,
    validated_quick_select_eq: Option<QuickSelectEqCapabilities>,
    validated_source_catalog: Option<SourceCatalogCapabilities>,
    source_catalog: SourceCatalog,
    source_catalog_observation: Option<SourceCatalogObservation>,
    quick_select_names_observation: Option<QuickSelectNameObservation>,
}
impl<F: SessionFactory> State<F> {
    async fn handle(
        &mut self,
        command: ReceiverCommand,
        events: &mpsc::Sender<ReceiverEvent>,
    ) -> Result<CommandReply, OperationError> {
        match command {
            ReceiverCommand::Select(s) => {
                // A replacement selection must not be stranded by a failed
                // teardown of the previous session. `disconnect` has already
                // invalidated state and taken ownership of that session before
                // it can fail, so the new receiver can be selected safely.
                if let Err(error) = self.disconnect().await {
                    self.observability.record(Diagnostic::Timeout {
                        context: format!("closing previous receiver session: {error}"),
                    });
                }
                self.selection = Some(s);
                Self::emit(events, ReceiverEvent::Lifecycle(Lifecycle::Selected)).await;
                Ok(Some(ReceiverEvent::Snapshot(self.snapshot.clone())))
            }
            ReceiverCommand::Connect => {
                self.connect(events).await?;
                Ok(Some(ReceiverEvent::Lifecycle(self.lifecycle.clone())))
            }
            ReceiverCommand::Disconnect => {
                let result = self.disconnect().await;
                Self::emit(events, ReceiverEvent::Snapshot(self.snapshot.clone())).await;
                result?;
                Ok(Some(ReceiverEvent::Lifecycle(self.lifecycle.clone())))
            }
            ReceiverCommand::Refresh => {
                self.refresh(events).await?;
                let _ = self.refresh_zone2(events).await;
                // Keep startup/refresh bounded by the core Main Zone queries.
                // HTTP information and Quick Select names are supplemental
                // reads; awaiting either here would prevent the bridge from
                // forwarding the Connected/core Snapshot events when an
                // optional endpoint stalls or is unsupported.
                Ok(Some(ReceiverEvent::Snapshot(self.snapshot.clone())))
            }
            ReceiverCommand::ControlZone2 { power } => {
                let result = self.control_zone2(power, events).await;
                Ok(Some(ReceiverEvent::Control(result)))
            }
            ReceiverCommand::RefreshQuickSelectEq => {
                self.refresh_quick_select_eq(events).await?;
                Ok(Some(ReceiverEvent::EqStatus(Box::new(
                    self.eq_status.clone(),
                ))))
            }
            ReceiverCommand::RefreshSourceCatalog => {
                self.refresh_source_catalog(events).await?;
                Ok(Some(ReceiverEvent::SourceCatalog(Box::new(
                    self.source_catalog_observation
                        .clone()
                        .expect("successful source catalog refresh records an observation"),
                ))))
            }
            ReceiverCommand::RefreshQuickSelectNames => {
                self.refresh_quick_select_names(events).await?;
                Ok(Some(ReceiverEvent::QuickSelect(Box::new(
                    self.quick_select.clone(),
                ))))
            }
            ReceiverCommand::RefreshHttpInformation => {
                self.refresh_http_information(events).await?;
                Ok(Some(ReceiverEvent::Snapshot(self.snapshot.clone())))
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
                self.invalidate_quick_select_eq();
                self.invalidate_source_catalog();
                self.snapshot.invalidate_http_information(self.generation);
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
                self.snapshot.invalidate();
                self.zone2.invalidate();
                self.invalidate_quick_select_eq();
                self.invalidate_source_catalog();
                self.snapshot.invalidate_http_information(self.generation);
                Self::emit(events, ReceiverEvent::Lifecycle(Lifecycle::Disconnected)).await;
                Self::emit(events, ReceiverEvent::Snapshot(self.snapshot.clone())).await;
                Self::emit(events, ReceiverEvent::Zone2Snapshot(self.zone2.clone())).await;
                Err(e)
            }
        }
    }
    async fn disconnect(&mut self) -> Result<(), OperationError> {
        self.snapshot.invalidate();
        self.zone2.invalidate();
        self.invalidate_quick_select_eq();
        self.invalidate_source_catalog();
        self.snapshot.invalidate_http_information(self.generation);
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

    fn invalidate_quick_select_eq(&mut self) {
        crate::quick_select::invalidate(
            &mut self.quick_select,
            &mut self.eq_status,
            &mut self.quick_select_names_observation,
            self.generation,
        );
    }

    fn invalidate_source_catalog(&mut self) {
        self.source_catalog.invalidate(self.generation);
        self.source_catalog_observation = None;
    }

    async fn drain_session_events(&mut self, events: &mpsc::Sender<ReceiverEvent>) {
        // A receiver may produce an uninterrupted stream of unsolicited
        // notifications. Draining until the stream becomes idle would starve
        // the serialized command queue, including Refresh and a newly
        // discovered receiver selection. Process a bounded batch and let the
        // next command/idle turn continue draining remaining notifications.
        for _ in 0..MAX_SESSION_EVENTS_PER_DRAIN {
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
                    self.zone2.invalidate();
                    self.invalidate_quick_select_eq();
                    self.invalidate_source_catalog();
                    self.snapshot.invalidate_http_information(self.generation);
                    Self::emit(events, ReceiverEvent::Lifecycle(self.lifecycle.clone())).await;
                    Self::emit(events, ReceiverEvent::Snapshot(self.snapshot.clone())).await;
                    Self::emit(events, ReceiverEvent::Zone2Snapshot(self.zone2.clone())).await;
                }
                Ok(SessionEvent::Connection(ConnectionState::Connected)) => {
                    let was_reconnecting = matches!(self.lifecycle, Lifecycle::Reconnecting { .. });
                    if was_reconnecting {
                        self.lifecycle = Lifecycle::Connected {
                            generation: self.generation,
                        };
                        Self::emit(events, ReceiverEvent::Lifecycle(self.lifecycle.clone())).await;
                        if let Err(error) = self.refresh(events).await {
                            self.observability.record(Diagnostic::Timeout {
                                context: format!("refreshing after reconnect: {error}"),
                            });
                        }
                        let _ = self.refresh_http_information(events).await;
                        let _ = self.refresh_quick_select_names(events).await;
                    }
                }
                Ok(SessionEvent::Connection(ConnectionState::Disconnected)) => {
                    self.lifecycle = Lifecycle::Disconnected;
                    self.snapshot.invalidate();
                    self.zone2.invalidate();
                    self.invalidate_quick_select_eq();
                    self.invalidate_source_catalog();
                    self.snapshot.invalidate_http_information(self.generation);
                    Self::emit(events, ReceiverEvent::Lifecycle(self.lifecycle.clone())).await;
                    Self::emit(events, ReceiverEvent::Snapshot(self.snapshot.clone())).await;
                    Self::emit(events, ReceiverEvent::Zone2Snapshot(self.zone2.clone())).await;
                }
                Ok(SessionEvent::MainZone(event)) => {
                    let refresh_information = matches!(
                        event,
                        MainZoneEvent::Changed(MainZoneValue::Input(_))
                            | MainZoneEvent::Changed(MainZoneValue::SurroundMode(_))
                            | MainZoneEvent::Changed(MainZoneValue::Power(
                                denon_avr_domain::PowerState::On
                            ))
                    );
                    self.snapshot.apply_event(event);
                    Self::emit(events, ReceiverEvent::Snapshot(self.snapshot.clone())).await;
                    if refresh_information {
                        let _ = self.refresh_http_information(events).await;
                    }
                }
                Ok(SessionEvent::Zone2Power(power)) => {
                    self.zone2.set_power(power, StateAuthority::Event);
                    Self::emit(events, ReceiverEvent::Zone2Snapshot(self.zone2.clone())).await;
                }
                Err(error) => {
                    self.observability.record(Diagnostic::Timeout {
                        context: error.to_string(),
                    });
                    self.lifecycle = Lifecycle::Disconnected;
                    self.snapshot.invalidate();
                    self.zone2.invalidate();
                    self.invalidate_quick_select_eq();
                    self.invalidate_source_catalog();
                    self.snapshot.invalidate_http_information(self.generation);
                    Self::emit(events, ReceiverEvent::Lifecycle(self.lifecycle.clone())).await;
                    Self::emit(events, ReceiverEvent::Snapshot(self.snapshot.clone())).await;
                    Self::emit(events, ReceiverEvent::Zone2Snapshot(self.zone2.clone())).await;
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

        // The five Main Zone fields form the status contract. Complete this
        // refresh once they have been queried: supplemental audio-context
        // probes are not part of connection or dashboard readiness. In
        // particular, some receivers do not answer every diagnostic family
        // (for example, `DC?`), so waiting for them here can strand a GUI in
        // its initial "connecting"/unknown-power state after power was
        // already confirmed.
        if self.snapshot.resource_version() != before {
            Self::emit(
                events,
                ReceiverEvent::ResourceVersionChanged(self.snapshot.resource_version()),
            )
            .await;
        }
        Self::emit(events, ReceiverEvent::Snapshot(self.snapshot.clone())).await;
        // Source-catalog retrieval is intentionally separate from this
        // command. Main Zone status (especially PWON) must be published even
        // when the receiver's optional HTTP endpoint is unavailable.
        Ok(())
    }
    async fn refresh_zone2(
        &mut self,
        events: &mpsc::Sender<ReceiverEvent>,
    ) -> Result<(), OperationError> {
        if !self.capabilities().zone2_power {
            return Ok(());
        }
        let result = self
            .session
            .as_mut()
            .ok_or_else(stopped)?
            .query_zone2_power()
            .await;
        match result {
            Ok(power) => self.zone2.set_power(power, StateAuthority::Authoritative),
            Err(error) => self.zone2.set_error(field_error(error)),
        }
        Self::emit(events, ReceiverEvent::Zone2Snapshot(self.zone2.clone())).await;
        Ok(())
    }

    async fn control_zone2(
        &mut self,
        power: PowerState,
        events: &mpsc::Sender<ReceiverEvent>,
    ) -> ControlResult {
        if !self.capabilities().zone2_power {
            return ControlResult::Unsupported(
                "Zone 2 power is available only on AVR-X3800H receivers".into(),
            );
        }
        if self.session.is_none() {
            if let Err(error) = self.connect(events).await {
                return ControlResult::Rejected(error);
            }
        }
        if self.zone2.power.value() == Some(&power) {
            return ControlResult::NoOp {
                snapshot: self.snapshot.clone(),
            };
        }
        let Some(session) = self.session.as_mut() else {
            return ControlResult::Rejected(stopped());
        };
        if let Err(error) = session.execute_zone2_once(Zone2Control::Power(power)).await {
            return ControlResult::TransportFailure(error);
        }
        match tokio::time::timeout(self.config.confirmation_timeout, self.refresh_zone2(events))
            .await
        {
            Ok(Ok(())) if self.zone2.power.value() == Some(&power) => ControlResult::Confirmed {
                snapshot: self.snapshot.clone(),
            },
            Ok(Ok(())) => ControlResult::Unconfirmed(OperationError::new(
                OperationErrorKind::Unavailable,
                "confirming Zone 2 power",
                "receiver did not report requested value",
            )),
            Ok(Err(error)) => ControlResult::Unconfirmed(error),
            Err(_) => ControlResult::Unconfirmed(OperationError::new(
                OperationErrorKind::Timeout,
                "confirming Zone 2 power",
                "confirmation deadline expired",
            )),
        }
    }
    async fn refresh_http_information(
        &mut self,
        events: &mpsc::Sender<ReceiverEvent>,
    ) -> Result<(), OperationError> {
        if !self.should_read_http_information() {
            return Ok(());
        }
        if self.session.is_none()
            || self.snapshot.power.value() != Some(&denon_avr_domain::PowerState::On)
        {
            self.snapshot.invalidate_http_information(self.generation);
            return Ok(());
        }
        let previous = self.snapshot.http_information.clone();
        let result = self
            .session
            .as_mut()
            .ok_or_else(stopped)?
            .refresh_http_information()
            .await;
        let (information, outcome) =
            crate::http_information::merge_refresh(previous, self.generation, result);
        self.snapshot.set_http_information(information);
        Self::emit(events, ReceiverEvent::Snapshot(self.snapshot.clone())).await;
        outcome
    }
    async fn refresh_source_catalog(
        &mut self,
        events: &mpsc::Sender<ReceiverEvent>,
    ) -> Result<(), OperationError> {
        crate::source_catalog::ensure_supported(&self.capabilities())?;
        if self.session.is_none() {
            self.connect(events).await?;
        }
        let previous = self.source_catalog.clone();
        let result = self
            .session
            .as_mut()
            .ok_or_else(stopped)?
            .refresh_source_catalog()
            .await;
        let (catalog, observation, outcome) =
            crate::source_catalog::merge_refresh(previous, self.generation, result);
        self.source_catalog = catalog;
        self.source_catalog_observation = Some(observation.clone());
        Self::emit(events, ReceiverEvent::SourceCatalog(Box::new(observation))).await;
        outcome
    }
    async fn refresh_quick_select_eq(
        &mut self,
        events: &mpsc::Sender<ReceiverEvent>,
    ) -> Result<(), OperationError> {
        crate::quick_select::ensure_eq_supported(&self.capabilities())?;
        if self.session.is_none() {
            self.connect(events).await?;
        }
        // Quick Select is an execute-only capability.  Denon does not expose
        // a validated per-slot query for the registered fields, so refresh
        // must not invent availability or names from a guessed response.
        let previous_eq_status = self.eq_status.clone();
        let result = {
            let session = self.session.as_mut().ok_or_else(stopped)?;
            session.query_eq_status().await
        };
        match result {
            Ok(refreshed_eq_status) => {
                self.eq_status = crate::quick_select::merge_eq_status(
                    &previous_eq_status,
                    refreshed_eq_status,
                    self.generation,
                    self.snapshot.audio_context.current_mode.value.known(),
                );
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
        Ok(())
    }
    async fn refresh_quick_select_names(
        &mut self,
        events: &mpsc::Sender<ReceiverEvent>,
    ) -> Result<(), OperationError> {
        crate::quick_select::ensure_names_supported(&self.capabilities())?;
        if self.session.is_none() {
            self.connect(events).await?;
        }
        let previous = self.quick_select_names_observation.clone();
        let result = self
            .session
            .as_mut()
            .ok_or_else(stopped)?
            .refresh_quick_select_names()
            .await;
        match result {
            Ok(observation) => {
                let observation = crate::quick_select::apply_names(
                    &mut self.quick_select,
                    observation,
                    self.generation,
                );
                self.quick_select_names_observation = Some(observation.clone());
                Self::emit(
                    events,
                    ReceiverEvent::QuickSelectNames(Box::new(observation)),
                )
                .await;
                Self::emit(
                    events,
                    ReceiverEvent::QuickSelect(Box::new(self.quick_select.clone())),
                )
                .await;
                Ok(())
            }
            Err(error) => {
                self.quick_select_names_observation = previous;
                Err(error)
            }
        }
    }
    async fn recall_quick_select(
        &mut self,
        slot: QuickSelectSlot,
        expected: Option<u64>,
        events: &mpsc::Sender<ReceiverEvent>,
    ) -> QuickSelectRecallOutcome {
        if let Some(outcome) = crate::quick_select::recall_unsupported(&self.capabilities()) {
            return outcome;
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
        crate::quick_select::recall_outcome(slot, session.recall_quick_select(slot).await)
    }
    fn capabilities(&self) -> ModelCapabilities {
        let model = self
            .selection
            .as_ref()
            .and_then(|s| s.identity().model.clone())
            .map(|value| Model::from_reported(&value))
            .unwrap_or(Model::Unknown);
        ModelCapabilities::for_model(model)
            .with_validated_quick_select_eq(self.validated_quick_select_eq.unwrap_or_default())
            .with_validated_source_catalog(self.validated_source_catalog.unwrap_or_default())
    }
    fn should_read_http_information(&self) -> bool {
        crate::http_information::should_read(
            &self.capabilities(),
            matches!(self.selection, Some(ReceiverSelection::Saved { .. })),
        )
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
        if self.session.is_none() {
            if let Err(e) = self.connect(events).await {
                return ControlResult::Rejected(e);
            }
        }
        match admit_main_zone_control(&self.snapshot, &capabilities, &control, expected) {
            ControlAdmission::Dispatch => {}
            ControlAdmission::NoOp => {
                return ControlResult::NoOp {
                    snapshot: self.snapshot.clone(),
                };
            }
            ControlAdmission::Rejected(error) => {
                if let Some(expected) = expected {
                    if error.kind == OperationErrorKind::Conflict {
                        return ControlResult::Conflict {
                            expected,
                            current: self.snapshot.resource_version(),
                        };
                    }
                }
                return ControlResult::Rejected(error);
            }
            ControlAdmission::Unsupported(message) => {
                return ControlResult::Unsupported(message);
            }
        }
        let field = control_field(&control);
        let Some(s) = self.session.as_mut() else {
            return ControlResult::Rejected(stopped());
        };
        if let Err(e) = s.execute_once(control.clone()).await {
            return ControlResult::TransportFailure(e);
        }
        if matches!(
            control,
            MainZoneControl::Power(denon_avr_domain::PowerState::On)
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
                if let Some(category) = sound_mode_category_from_control(&control) {
                    // `refresh` has just queried `MS?` authoritatively. Pair
                    // that reported detailed mode with the category that was
                    // actually dispatched, then publish the confirmed pair.
                    self.snapshot.confirm_sound_mode_category(category);
                    Self::emit(events, ReceiverEvent::Snapshot(self.snapshot.clone())).await;
                }
                if crate::http_information::may_have_changed(&control) {
                    let _ = self.refresh_http_information(events).await;
                }
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
fn sound_mode_category_from_control(
    control: &MainZoneControl,
) -> Option<denon_avr_domain::SoundModeCategory> {
    match control {
        MainZoneControl::SelectSoundMode { category, .. }
        | MainZoneControl::RecallSoundModeCategory(category) => Some(*category),
        _ => None,
    }
}
fn stopped() -> OperationError {
    invalid("controller", "controller is stopped")
}
fn invalid(context: &'static str, message: impl Into<String>) -> OperationError {
    OperationError::new(OperationErrorKind::Stopped, context, message)
}
fn field_error(e: OperationError) -> denon_avr_domain::FieldError {
    denon_avr_domain::FieldError {
        kind: match e.kind {
            OperationErrorKind::Timeout => denon_avr_domain::FieldErrorKind::Timeout,
            OperationErrorKind::Disconnected | OperationErrorKind::Connection => {
                denon_avr_domain::FieldErrorKind::Disconnected
            }
            OperationErrorKind::Malformed => denon_avr_domain::FieldErrorKind::Malformed,
            OperationErrorKind::Unsupported => denon_avr_domain::FieldErrorKind::Unsupported,
            _ => denon_avr_domain::FieldErrorKind::Unavailable,
        },
        message: e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use denon_avr_domain::{AudioContextSnapshot, QuickSelectRecallConfirmation, ReceiverIdentity};
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    #[derive(Clone)]
    struct FakeFactory;

    struct FakeSession {
        closes: Option<Arc<AtomicUsize>>,
    }

    #[derive(Clone)]
    struct ShutdownFactory {
        closes: Arc<AtomicUsize>,
    }

    #[derive(Clone)]
    struct ReplacementFactory {
        connections: Arc<AtomicUsize>,
    }

    struct CloseFailureSession;
    struct StatusSession;

    #[derive(Clone)]
    struct EventFloodFactory;

    struct EventFloodSession;

    #[derive(Clone)]
    struct CatalogFactory {
        observations: Arc<Mutex<VecDeque<Result<SourceCatalogObservation, OperationError>>>>,
    }

    struct CatalogSession {
        observations: Arc<Mutex<VecDeque<Result<SourceCatalogObservation, OperationError>>>>,
    }

    impl SessionFactory for FakeFactory {
        fn connect(
            &self,
            _identity: ReceiverIdentity,
        ) -> BoxFuture<'_, Result<Box<dyn ReceiverSession>, OperationError>> {
            Box::pin(async {
                Ok(Box::new(FakeSession { closes: None }) as Box<dyn ReceiverSession>)
            })
        }
    }

    impl SessionFactory for ShutdownFactory {
        fn connect(
            &self,
            _identity: ReceiverIdentity,
        ) -> BoxFuture<'_, Result<Box<dyn ReceiverSession>, OperationError>> {
            let closes = Arc::clone(&self.closes);
            Box::pin(async move {
                Ok(Box::new(FakeSession {
                    closes: Some(closes),
                }) as Box<dyn ReceiverSession>)
            })
        }
    }

    impl SessionFactory for ReplacementFactory {
        fn connect(
            &self,
            _identity: ReceiverIdentity,
        ) -> BoxFuture<'_, Result<Box<dyn ReceiverSession>, OperationError>> {
            let connection = self.connections.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if connection == 0 {
                    Ok(Box::new(CloseFailureSession) as Box<dyn ReceiverSession>)
                } else {
                    Ok(Box::new(StatusSession) as Box<dyn ReceiverSession>)
                }
            })
        }
    }

    impl SessionFactory for EventFloodFactory {
        fn connect(
            &self,
            _identity: ReceiverIdentity,
        ) -> BoxFuture<'_, Result<Box<dyn ReceiverSession>, OperationError>> {
            Box::pin(async { Ok(Box::new(EventFloodSession) as Box<dyn ReceiverSession>) })
        }
    }

    impl SessionFactory for CatalogFactory {
        fn connect(
            &self,
            _identity: ReceiverIdentity,
        ) -> BoxFuture<'_, Result<Box<dyn ReceiverSession>, OperationError>> {
            let observations = Arc::clone(&self.observations);
            Box::pin(async move {
                Ok(Box::new(CatalogSession { observations }) as Box<dyn ReceiverSession>)
            })
        }
    }

    impl ReceiverSession for FakeSession {
        fn query_field(
            &mut self,
            _field: MainZoneField,
        ) -> BoxFuture<'_, Result<MainZoneValue, OperationError>> {
            Box::pin(async move {
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
            Box::pin(async { std::future::pending::<Result<SessionEvent, OperationError>>().await })
        }

        fn close(&mut self) -> BoxFuture<'_, Result<(), OperationError>> {
            if let Some(closes) = &self.closes {
                closes.fetch_add(1, Ordering::SeqCst);
            }
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
                    dynamic_eq: denon_avr_domain::EqState::On,
                    generation: 999,
                    freshness: denon_avr_domain::Freshness::Live,
                    ..EqStatus::default()
                })
            })
        }

        fn refresh_source_catalog(
            &mut self,
        ) -> BoxFuture<'_, Result<SourceCatalogObservation, OperationError>> {
            Box::pin(async {
                Ok(SourceCatalogObservation {
                    catalog: SourceCatalog {
                        entries: vec![denon_avr_domain::SourceEntry {
                            id: denon_avr_domain::SourceId::new("GAME").unwrap(),
                            display_name: Some("Console".into()),
                            visibility: denon_avr_domain::SourceVisibility::Shown,
                        }],
                        freshness: denon_avr_domain::Freshness::Live,
                        generation: 42,
                        observed_at: None,
                        error: None,
                    },
                    raw_response: "<rx/>".into(),
                    response_evidence: denon_avr_domain::CatalogResponseEvidence::Complete,
                })
            })
        }
    }

    impl ReceiverSession for CloseFailureSession {
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
            Box::pin(async { std::future::pending::<Result<SessionEvent, OperationError>>().await })
        }

        fn close(&mut self) -> BoxFuture<'_, Result<(), OperationError>> {
            Box::pin(async {
                Err(OperationError::new(
                    OperationErrorKind::Stopped,
                    "test close",
                    "simulated teardown failure",
                ))
            })
        }
    }

    impl ReceiverSession for StatusSession {
        fn query_field(
            &mut self,
            field: MainZoneField,
        ) -> BoxFuture<'_, Result<MainZoneValue, OperationError>> {
            Box::pin(async move {
                let value = match field {
                    MainZoneField::Power => MainZoneValue::Power(denon_avr_domain::PowerState::On),
                    MainZoneField::Input => MainZoneValue::Input(
                        denon_avr_domain::Input::new("CD").expect("test input"),
                    ),
                    MainZoneField::Volume => {
                        MainZoneValue::Volume(denon_avr_domain::Volume::from_parts("50", -300))
                    }
                    MainZoneField::Mute => MainZoneValue::Mute(denon_avr_domain::MuteState::Off),
                    MainZoneField::SurroundMode => MainZoneValue::SurroundMode(
                        denon_avr_domain::SurroundMode::new("STEREO").expect("test mode"),
                    ),
                };
                Ok(value)
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
            Box::pin(async { std::future::pending::<Result<SessionEvent, OperationError>>().await })
        }

        fn close(&mut self) -> BoxFuture<'_, Result<(), OperationError>> {
            Box::pin(async { Ok(()) })
        }
    }

    impl ReceiverSession for EventFloodSession {
        fn query_field(
            &mut self,
            field: MainZoneField,
        ) -> BoxFuture<'_, Result<MainZoneValue, OperationError>> {
            Box::pin(async move {
                let value = match field {
                    MainZoneField::Power => MainZoneValue::Power(denon_avr_domain::PowerState::On),
                    MainZoneField::Input => MainZoneValue::Input(
                        denon_avr_domain::Input::new("CD").expect("test input"),
                    ),
                    MainZoneField::Volume => {
                        MainZoneValue::Volume(denon_avr_domain::Volume::from_parts("50", -300))
                    }
                    MainZoneField::Mute => MainZoneValue::Mute(denon_avr_domain::MuteState::Off),
                    MainZoneField::SurroundMode => MainZoneValue::SurroundMode(
                        denon_avr_domain::SurroundMode::new("STEREO").expect("test mode"),
                    ),
                };
                Ok(value)
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
                Ok(SessionEvent::MainZone(MainZoneEvent::Unknown(
                    "test event".into(),
                )))
            })
        }

        fn close(&mut self) -> BoxFuture<'_, Result<(), OperationError>> {
            Box::pin(async { Ok(()) })
        }
    }

    impl ReceiverSession for CatalogSession {
        fn query_field(
            &mut self,
            field: MainZoneField,
        ) -> BoxFuture<'_, Result<MainZoneValue, OperationError>> {
            Box::pin(async move {
                if field == MainZoneField::Power {
                    Ok(MainZoneValue::Power(denon_avr_domain::PowerState::On))
                } else {
                    Err(OperationError::new(
                        OperationErrorKind::Unsupported,
                        "test",
                        "not needed",
                    ))
                }
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
            Box::pin(async { std::future::pending::<Result<SessionEvent, OperationError>>().await })
        }

        fn close(&mut self) -> BoxFuture<'_, Result<(), OperationError>> {
            Box::pin(async { Ok(()) })
        }

        fn refresh_source_catalog(
            &mut self,
        ) -> BoxFuture<'_, Result<SourceCatalogObservation, OperationError>> {
            Box::pin(async move {
                self.observations
                    .lock()
                    .expect("test observations lock")
                    .pop_front()
                    .expect("test observation")
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
    async fn shutdown_closes_the_owned_session_exactly_once() {
        let closes = Arc::new(AtomicUsize::new(0));
        let factory = ShutdownFactory {
            closes: Arc::clone(&closes),
        };
        let handle = ReceiverController::spawn(factory, ControllerConfig::default());
        handle.select(selection()).await.unwrap();
        handle.connect().await.unwrap();

        assert_eq!(handle.shutdown().await.unwrap(), None);
        assert_eq!(closes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn replacement_selection_recovers_from_previous_session_close_failure() {
        let connections = Arc::new(AtomicUsize::new(0));
        let factory = ReplacementFactory {
            connections: Arc::clone(&connections),
        };
        let handle = ReceiverController::spawn(factory, ControllerConfig::default());

        handle.select(selection()).await.unwrap();
        handle.connect().await.unwrap();

        // Re-discovery selects a receiver again. A failure while closing the
        // old session must not leave the newly selected receiver without a
        // session or an authoritative dashboard snapshot.
        handle.select(selection()).await.unwrap();
        let reply = handle.refresh().await.unwrap();

        let Some(ReceiverEvent::Snapshot(snapshot)) = reply else {
            panic!("refresh did not return a Main Zone snapshot");
        };
        assert_eq!(
            snapshot.power.value(),
            Some(&denon_avr_domain::PowerState::On)
        );
        assert_eq!(connections.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn refresh_completes_when_session_events_are_continuous() {
        let config = ControllerConfig {
            // The flood emits one UI event for every drained session event.
            // Keep this larger than a single bounded drain batch.
            queue_capacity: 128,
            ..ControllerConfig::default()
        };
        let handle = ReceiverController::spawn(EventFloodFactory, config);

        handle.select(selection()).await.unwrap();
        let reply = tokio::time::timeout(Duration::from_millis(250), handle.refresh())
            .await
            .expect("continuous unsolicited events must not starve refresh")
            .unwrap();

        let Some(ReceiverEvent::Snapshot(snapshot)) = reply else {
            panic!("refresh did not return a Main Zone snapshot");
        };
        assert_eq!(
            snapshot.power.value(),
            Some(&denon_avr_domain::PowerState::On)
        );
    }

    #[tokio::test]
    async fn validated_profile_preserves_authoritative_recall_result() {
        let config = ControllerConfig {
            validated_quick_select_eq: Some(QuickSelectEqCapabilities {
                quick_select_recall: true,
                quick_select_names: true,
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
            validated_quick_select_eq: Some(QuickSelectEqCapabilities {
                quick_select_recall: true,
                quick_select_names: true,
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
            validated_quick_select_eq: Some(QuickSelectEqCapabilities {
                quick_select_recall: false,
                quick_select_names: false,
                eq_status: true,
            }),
            ..ControllerConfig::default()
        };
        let handle = ReceiverController::spawn(FakeFactory, config);
        handle.select(selection()).await.unwrap();
        let reply = handle.refresh_quick_select_eq().await.unwrap();
        let Some(ReceiverEvent::EqStatus(status)) = reply else {
            panic!("EQ refresh did not return an EQ status snapshot")
        };
        assert_eq!(status.dynamic_eq, denon_avr_domain::EqState::On);
        assert_eq!(status.generation, 1);
    }

    #[tokio::test]
    async fn quick_select_only_profile_rejects_status_refresh() {
        let config = ControllerConfig {
            validated_quick_select_eq: Some(QuickSelectEqCapabilities {
                quick_select_recall: true,
                quick_select_names: true,
                eq_status: false,
            }),
            ..ControllerConfig::default()
        };
        let handle = ReceiverController::spawn(FakeFactory, config);
        handle.select(selection()).await.unwrap();
        let error = handle.refresh_quick_select_eq().await.unwrap_err();
        assert_eq!(error.kind, OperationErrorKind::Unsupported);
    }

    #[tokio::test]
    async fn validated_source_catalog_can_be_refreshed_after_core_status() {
        let config = ControllerConfig {
            validated_source_catalog: Some(SourceCatalogCapabilities {
                source_catalog_read: true,
            }),
            ..ControllerConfig::default()
        };
        let mut handle = ReceiverController::spawn(FakeFactory, config);
        handle.select(selection()).await.unwrap();
        handle.refresh().await.unwrap();
        handle.refresh_source_catalog().await.unwrap();
        let mut catalog = None;
        for _ in 0..12 {
            if let Some(ReceiverEvent::SourceCatalog(value)) = handle.next_event().await {
                catalog = Some(value);
                break;
            }
        }
        let catalog = catalog.expect("catalog event");
        assert_eq!(catalog.catalog.generation, 1);
        assert_eq!(
            catalog.catalog.entries[0].display_name.as_deref(),
            Some("Console")
        );
        assert_eq!(catalog.raw_response, "<rx/>");
    }

    #[tokio::test]
    async fn core_refresh_does_not_wait_for_source_catalog_http() {
        let config = ControllerConfig {
            validated_source_catalog: Some(SourceCatalogCapabilities {
                source_catalog_read: true,
            }),
            ..ControllerConfig::default()
        };
        let factory = CatalogFactory {
            observations: Arc::new(Mutex::new(VecDeque::new())),
        };
        let handle = ReceiverController::spawn(factory, config);
        handle.select(selection()).await.unwrap();
        let result = handle.refresh().await.unwrap();
        let Some(ReceiverEvent::Snapshot(snapshot)) = result else {
            panic!("core refresh result")
        };
        assert_eq!(
            snapshot.power.value(),
            Some(&denon_avr_domain::PowerState::On)
        );
    }

    #[tokio::test]
    async fn unsupported_reply_preserves_last_known_catalog_in_the_same_generation() {
        let first = SourceCatalogObservation {
            catalog: SourceCatalog {
                entries: vec![denon_avr_domain::SourceEntry {
                    id: denon_avr_domain::SourceId::new("GAME").unwrap(),
                    display_name: Some("Console".into()),
                    visibility: denon_avr_domain::SourceVisibility::Shown,
                }],
                freshness: denon_avr_domain::Freshness::Live,
                generation: 0,
                observed_at: None,
                error: None,
            },
            raw_response: "<rx><functionrename/></rx>".into(),
            response_evidence: denon_avr_domain::CatalogResponseEvidence::Complete,
        };
        let unsupported = SourceCatalogObservation {
            catalog: SourceCatalog::default(),
            raw_response: "<rx/>".into(),
            response_evidence: denon_avr_domain::CatalogResponseEvidence::Unsupported,
        };
        let factory = CatalogFactory {
            observations: Arc::new(Mutex::new(VecDeque::from([Ok(first), Ok(unsupported)]))),
        };
        let config = ControllerConfig {
            validated_source_catalog: Some(SourceCatalogCapabilities {
                source_catalog_read: true,
            }),
            ..ControllerConfig::default()
        };
        let handle = ReceiverController::spawn(factory, config);
        handle.select(selection()).await.unwrap();
        handle.refresh().await.unwrap();
        handle.refresh_source_catalog().await.unwrap();
        let Some(ReceiverEvent::SourceCatalog(observation)) =
            handle.refresh_source_catalog().await.unwrap()
        else {
            panic!("source catalog refresh result")
        };
        assert_eq!(observation.raw_response, "<rx/>");
        assert_eq!(
            observation.catalog.entries[0].display_name.as_deref(),
            Some("Console")
        );
        assert_eq!(
            observation.catalog.freshness,
            denon_avr_domain::Freshness::Partial
        );
    }
}
