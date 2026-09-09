//! Iced presentation layer for the desktop client.
//!
//! This module deliberately contains no AVR protocol knowledge.  All receiver
//! policy, confirmation, retry, and lifecycle decisions remain in the typed
//! application controller.

use denon_avr_application::ports::AsyncReceiverDiscovery;
use denon_avr_application::{
    ControllerHandle, ReceiverController, ReceiverEvent, ReceiverSelection, SessionFactory,
};
use denon_avr_domain::{
    ConfiguredReceivers, EqStatus, Input, ListeningModeGroup, MainZoneControl, MainZoneSnapshot,
    MainZoneValue, Model, ModelCapabilities, MuteState, PowerState, QuickSelectSlot,
    QuickSelectSnapshot, ReceiverIdentity, StateAuthority, SurroundMode,
    ValidatedPhase8Capabilities, Volume,
};
use iced::futures::SinkExt;
use iced::widget::{
    button, column, container, row, scrollable, slider, space, stack, text, text_input,
};
use iced::{Element, Length, Subscription, Task};
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};

mod capture;
pub mod components;
pub mod design;
mod feedback;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Dashboard,
    Receivers,
    Settings,
    Advanced,
    Diagnostics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowClass {
    Wide,
    Compact,
}

/// Session-only accessibility preferences. They deliberately live in GUI
/// state: a user override wins for this run and is never written to YAML.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionPreference {
    Normal,
    Reduced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContrastPreference {
    Normal,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeEvent {
    pub request_id: u64,
    pub generation: u64,
    pub event: ReceiverEvent,
}

#[derive(Debug, Clone)]
enum BridgeCommand {
    Select(u64, ReceiverSelection),
    Connect(u64),
    Refresh(u64),
    Disconnect(u64),
    Control(u64, MainZoneControl, Option<u64>),
    RefreshPhase8(u64),
    RecallQuickSelect(u64, QuickSelectSlot, Option<u64>),
    Shutdown(u64),
}

/// A single serialized owner of `ControllerHandle`.
#[derive(Clone)]
pub struct ControllerBridge {
    commands: mpsc::Sender<BridgeCommand>,
    events: Arc<Mutex<mpsc::Receiver<BridgeEvent>>>,
    validated_phase8: Option<ValidatedPhase8Capabilities>,
}

/// Presentation-facing service ports supplied by the desktop composition root.
#[derive(Clone)]
pub struct GuiServices {
    pub factory: Arc<dyn SessionFactory>,
    pub configuration: Arc<dyn denon_avr_application::AsyncConfigRepository>,
    pub discovery: Arc<dyn AsyncReceiverDiscovery>,
}

impl ControllerBridge {
    pub fn new<F: SessionFactory>(factory: F) -> Self {
        Self::new_with_config(factory, Default::default())
    }

    pub fn new_with_config<F: SessionFactory>(
        factory: F,
        config: denon_avr_application::ControllerConfig,
    ) -> Self {
        let validated_phase8 = config.validated_phase8;
        let (commands, mut command_rx) = mpsc::channel(16);
        let (event_sender, event_rx) = mpsc::channel(32);
        let events = Arc::new(Mutex::new(event_rx));
        tokio::spawn(async move {
            let handle = ReceiverController::spawn_with_observability(
                factory,
                config,
                Arc::new(TracingObservability),
            );
            run_bridge(handle, &mut command_rx, event_sender).await;
        });
        Self {
            commands,
            events: Arc::clone(&events),
            validated_phase8,
        }
    }

    pub fn subscription(&self) -> Subscription<BridgeEvent> {
        let data = SubscriptionData(Arc::clone(&self.events));
        Subscription::run_with(data, |data| {
            let events = Arc::clone(&data.0);
            iced::stream::channel(32, async move |mut output| loop {
                let event = events.lock().await.recv().await;
                if let Some(event) = event {
                    if output.send(event).await.is_err() {
                        break;
                    }
                } else {
                    break;
                }
            })
        })
    }

    async fn send(&self, command: BridgeCommand) -> Result<(), String> {
        self.commands
            .send(command)
            .await
            .map_err(|_| "controller bridge stopped".into())
    }
}

#[derive(Clone)]
struct SubscriptionData(Arc<Mutex<mpsc::Receiver<BridgeEvent>>>);
impl Hash for SubscriptionData {
    fn hash<H: Hasher>(&self, state: &mut H) {
        0x5d_u8.hash(state);
    }
}

async fn run_bridge(
    handle: ControllerHandle,
    commands: &mut mpsc::Receiver<BridgeCommand>,
    event_sender: mpsc::Sender<BridgeEvent>,
) {
    // The controller is intentionally owned by this task.  Replies are
    // converted to events so the GUI never borrows or polls it directly.
    let mut handle = handle;
    let mut pending_request_id: Option<u64> = None;
    let mut pending_generation: u64 = 0;

    // Continuously poll for commands and events independently to prevent
    // bounded-channel deadlock. While idle, continue consuming events instead
    // of waiting exclusively for the next GUI command.
    loop {
        tokio::select! {
            // Poll for a new command request
            Some(command) = commands.recv() => {
                let is_shutdown = matches!(&command, BridgeCommand::Shutdown(_));
                tracing::debug!(command = bridge_command_name(&command), "processing GUI command");
                let (request_id, result) = match command {
                    BridgeCommand::Select(id, selection) => {
                        let selected = handle.select(selection).await;
                        if selected.is_ok() {
                            (id, connect_and_refresh(&handle).await)
                        } else {
                            (id, selected)
                        }
                    }
                    BridgeCommand::Connect(id) => (id, connect_and_refresh(&handle).await),
                    BridgeCommand::Refresh(id) => (id, handle.refresh().await),
                    BridgeCommand::Disconnect(id) => (id, handle.disconnect().await),
                    BridgeCommand::Control(id, control, version) => {
                        (id, handle.control(control, version).await)
                    }
                    BridgeCommand::RefreshPhase8(id) => (id, handle.refresh_phase8().await),
                    BridgeCommand::RecallQuickSelect(id, slot, expected_version) => {
                        (id, handle.recall_quick_select(slot, expected_version).await)
                    }
                    BridgeCommand::Shutdown(id) => (id, handle.shutdown().await),
                };
                let event = result
                    .map(|reply| reply.unwrap_or(ReceiverEvent::Cancelled))
                    .unwrap_or_else(|error| {
                        ReceiverEvent::Diagnostic(denon_avr_application::Diagnostic::Timeout {
                            context: error.to_string(),
                        })
                    });
                let generation = match &event {
                    ReceiverEvent::Lifecycle(denon_avr_application::Lifecycle::Connected {
                        generation,
                    })
                    | ReceiverEvent::Lifecycle(denon_avr_application::Lifecycle::Reconnecting {
                        generation,
                    }) => *generation,
                    _ => 0,
                };
                pending_request_id = Some(request_id);
                pending_generation = generation;

                // Drain already-queued events before publishing the command's
                // final authoritative reply, preventing an invalidated selection
                // snapshot from replacing confirmed status.
                let mut reply_already_forwarded = false;
                while let Ok(Some(controller_event)) =
                    tokio::time::timeout(Duration::from_millis(1), handle.next_event()).await
                {
                    reply_already_forwarded |= controller_event == event;
                    let generation = match &controller_event {
                        ReceiverEvent::Lifecycle(denon_avr_application::Lifecycle::Connected {
                            generation,
                        })
                        | ReceiverEvent::Lifecycle(denon_avr_application::Lifecycle::Reconnecting {
                            generation,
                        }) => *generation,
                        _ => 0,
                    };
                    let _ = event_sender
                        .send(BridgeEvent {
                            request_id,
                            generation,
                            event: controller_event,
                        })
                        .await;
                }
                // A control result is both emitted by the controller and
                // returned as its command reply. Forward it once only.
                if !reply_already_forwarded {
                    let _ = event_sender
                        .send(BridgeEvent {
                            request_id,
                            generation,
                            event,
                        })
                        .await;
                }
                if is_shutdown {
                    tracing::info!("controller bridge stopped");
                    break;
                }
            }
                        // While idle, continue consuming events instead of waiting exclusively
            // for the next GUI command. This prevents bounded-channel deadlock when
            // unsolicited AVR events accumulate.
            Some(controller_event) = handle.next_event() => {
                let generation = match &controller_event {
                    ReceiverEvent::Lifecycle(denon_avr_application::Lifecycle::Connected {
                        generation,
                    })
                    | ReceiverEvent::Lifecycle(denon_avr_application::Lifecycle::Reconnecting {
                        generation,
                    }) => *generation,
                    _ => pending_generation,
                };
                let request_id = pending_request_id.unwrap_or(0);
                let _ = event_sender
                    .send(BridgeEvent {
                        request_id,
                        generation,
                        event: controller_event,
                    })
                    .await;
            }
        }
    }
}

fn bridge_command_name(command: &BridgeCommand) -> &'static str {
    match command {
        BridgeCommand::Select(..) => "select",
        BridgeCommand::Connect(..) => "connect",
        BridgeCommand::Refresh(..) => "refresh",
        BridgeCommand::Disconnect(..) => "disconnect",
        BridgeCommand::Control(..) => "control",
        BridgeCommand::RefreshPhase8(..) => "refresh_phase8",
        BridgeCommand::RecallQuickSelect(..) => "recall_quick_select",
        BridgeCommand::Shutdown(..) => "shutdown",
    }
}

/// Adapts application-owned, redacted diagnostics to the desktop's centralized
/// tracing subscriber without making the application layer depend on logging.
struct TracingObservability;

impl denon_avr_application::Observability for TracingObservability {
    fn record(&self, diagnostic: denon_avr_application::Diagnostic) {
        match diagnostic {
            denon_avr_application::Diagnostic::ConnectionGeneration(generation) => {
                tracing::info!(generation, "receiver connection established");
            }
            denon_avr_application::Diagnostic::ReconnectAttempt { attempt } => {
                tracing::warn!(attempt, "receiver reconnecting");
            }
            denon_avr_application::Diagnostic::Timeout { context } => {
                tracing::warn!(%context, "receiver operation timed out");
            }
            denon_avr_application::Diagnostic::MalformedFrame { context } => {
                tracing::warn!(%context, "receiver returned a malformed frame");
            }
            denon_avr_application::Diagnostic::QueuePressure { queued } => {
                tracing::warn!(queued, "receiver command queue under pressure");
            }
            denon_avr_application::Diagnostic::Shutdown => {
                tracing::info!("receiver controller shut down");
            }
        }
    }
}

/// A transport connection only establishes the session; it does not populate
/// the controller snapshot. Every GUI connection entry point must resolve the
/// complete Main Zone status before it reports success to the view.
async fn connect_and_refresh(
    handle: &ControllerHandle,
) -> Result<Option<ReceiverEvent>, denon_avr_application::OperationError> {
    let connected = handle.connect().await;
    if connected.is_ok() {
        handle.refresh().await
    } else {
        connected
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    ConfigLoaded(Result<ConfiguredReceivers, String>),
    CommandFinished(Result<(), String>),
    Navigate(Route),
    AddressChanged(String),
    NameChanged(String),
    Discover,
    DiscoveryFinished(Result<Vec<denon_avr_domain::DiscoveredReceiver>, String>),
    SaveDiscovered(denon_avr_domain::DiscoveredReceiver),
    DiscoveredSaved(Result<(ConfiguredReceivers, ReceiverSelection), String>),
    ManualSetup,
    ManualSaved(Result<(ConfiguredReceivers, ReceiverSelection), String>),
    Select(ReceiverSelection),
    Connect,
    Refresh,
    Disconnect,
    PowerOn,
    PowerOff,
    Mute,
    Unmute,
    VolumeChanged(f32),
    CommitVolume,
    AdjustVolume(f32),
    HideVolumeValue(u64),
    SelectListeningModeGroup(ListeningModeGroup),
    SelectSurroundMode(String),
    OpenSourcePicker,
    CloseSourcePicker,
    SelectInput(String),
    RefreshPhase8,
    RecallQuickSelect(QuickSelectSlot),
    Shutdown,
    ToggleMessages,
    ClearMessages,
    SetTextScale(u8),
    SetMotion(MotionPreference),
    SetContrast(ContrastPreference),
    CaptureVisual,
    ScreenshotCaptured(iced::window::Screenshot),
    ScreenshotWritten(Result<PathBuf, String>),
    Bridge(Box<BridgeEvent>),
}

pub struct Gui {
    pub route: Route,
    pub window_class: WindowClass,
    pub configured: ConfiguredReceivers,
    pub discovered: Vec<denon_avr_domain::DiscoveredReceiver>,
    pub selection: Option<ReceiverSelection>,
    pub lifecycle: denon_avr_application::Lifecycle,
    pub snapshot: MainZoneSnapshot,
    pub volume_slider: f32,
    volume_value: Option<f32>,
    volume_value_request_id: u64,
    source_picker_open: bool,
    pub announcement: String,
    pub address: String,
    pub name: String,
    pub request_id: u64,
    pub generation: u64,
    pub quick_select: QuickSelectSnapshot,
    pub eq_status: EqStatus,
    status_confirmed_generation: Option<u64>,
    /// Chronological, bounded command and lifecycle feedback for the global
    /// message panel. The panel is the only outcome surface in the shell.
    pub messages: VecDeque<String>,
    pub messages_collapsed: bool,
    pub text_scale: u8,
    pub motion_preference: MotionPreference,
    pub contrast_preference: ContrastPreference,
    capture_directory: Option<PathBuf>,
    capture_scenario: Option<String>,
    validated_phase8: Option<ValidatedPhase8Capabilities>,
    bridge: ControllerBridge,
    discovery: Arc<dyn AsyncReceiverDiscovery>,
    configuration: Arc<dyn denon_avr_application::AsyncConfigRepository>,
}

impl Gui {
    pub fn new(bridge: ControllerBridge) -> Self {
        let validated_phase8 = bridge.validated_phase8;
        Self {
            route: Route::Dashboard,
            window_class: WindowClass::Wide,
            configured: ConfiguredReceivers::default(),
            discovered: Vec::new(),
            selection: None,
            lifecycle: denon_avr_application::Lifecycle::NoReceiver,
            snapshot: MainZoneSnapshot::default(),
            volume_slider: 0.0,
            volume_value: None,
            volume_value_request_id: 0,
            source_picker_open: false,
            announcement: "Loading configuration…".into(),
            address: String::new(),
            name: String::new(),
            request_id: 0,
            generation: 0,
            quick_select: QuickSelectSnapshot::default(),
            eq_status: EqStatus::default(),
            status_confirmed_generation: None,
            messages: VecDeque::new(),
            messages_collapsed: false,
            text_scale: 100,
            motion_preference: MotionPreference::Normal,
            contrast_preference: ContrastPreference::Normal,
            capture_directory: std::env::var_os("DENON_AVR_CAPTURE_DIR").map(PathBuf::from),
            capture_scenario: None,
            validated_phase8,
            bridge,
            discovery: Arc::new(NoopDiscovery),
            configuration: Arc::new(NoopConfiguration),
        }
    }

    fn next_request(&mut self) -> u64 {
        self.request_id += 1;
        self.request_id
    }
    fn announce(&mut self, message: impl Into<String>) {
        let message = message.into();
        if self.messages.back() == Some(&message) {
            return;
        }
        tracing::info!(message = %message, "GUI feedback");
        self.announcement = message.clone();
        if self.messages.len() == design::MAX_SESSION_MESSAGES {
            self.messages.pop_front();
        }
        self.messages.push_back(message);
    }

    /// Adds feedback to the global chronological message surface.
    pub fn record_message(&mut self, message: impl Into<String>) {
        self.announce(message);
    }
    fn capture_destination(&self) -> Option<PathBuf> {
        let route = match self.route {
            Route::Dashboard => "dashboard",
            Route::Receivers => "receivers",
            Route::Settings => "settings",
            Route::Advanced => "advanced",
            Route::Diagnostics => "diagnostics",
        };
        let scenario = self.capture_scenario.as_deref().unwrap_or(route);
        self.capture_directory.as_ref().map(|directory| {
            directory
                .join(std::env::consts::OS)
                .join(format!("{scenario}-{}pct.png", self.text_scale))
        })
    }
    fn configure_capture_scenario(&mut self, scenario: &str) -> Result<(), String> {
        self.messages.clear();
        self.messages_collapsed = false;
        self.route = Route::Dashboard;
        self.selection = Some(ReceiverSelection::ExplicitHost(ReceiverIdentity {
            host: "capture.invalid".into(),
            model: Some("AVR-X3800H".into()),
            friendly_name: Some("Capture receiver".into()),
        }));
        self.generation = 1;
        self.lifecycle = denon_avr_application::Lifecycle::Connected { generation: 1 };
        self.snapshot = MainZoneSnapshot::default();
        self.snapshot.set_value(
            MainZoneValue::Power(PowerState::On),
            StateAuthority::Authoritative,
        );
        self.snapshot.set_value(
            MainZoneValue::Input(Input::new("GAME").expect("fixed capture input")),
            StateAuthority::Authoritative,
        );
        self.snapshot.set_value(
            MainZoneValue::Volume(Volume::from_parts("350", -300)),
            StateAuthority::Authoritative,
        );
        self.snapshot.set_value(
            MainZoneValue::Mute(MuteState::Off),
            StateAuthority::Authoritative,
        );
        self.snapshot.set_value(
            MainZoneValue::SurroundMode(
                SurroundMode::new("DOLBY SURROUND").expect("fixed capture mode"),
            ),
            StateAuthority::Authoritative,
        );
        self.volume_slider = 35.0;
        self.source_picker_open = false;
        match scenario {
            "connected" => self.announce("Deterministic capture scenario: connected."),
            "source-picker" => {
                self.source_picker_open = true;
                self.announce("Deterministic capture scenario: source picker.");
            }
            "unavailable" => {
                self.snapshot.invalidate();
                self.lifecycle = denon_avr_application::Lifecycle::Disconnected;
                self.announce("Deterministic capture scenario: unavailable receiver.");
            }
            "settings" => {
                self.route = Route::Settings;
                self.announce("Deterministic capture scenario: settings.");
            }
            "receivers" => {
                self.route = Route::Receivers;
                self.announce("Deterministic capture scenario: receiver setup.");
            }
            "diagnostics" => {
                self.route = Route::Diagnostics;
                self.announce("Deterministic capture scenario: diagnostics.");
            }
            "messages" => {
                self.announce("Deterministic capture scenario: global messages.");
                self.announce("Command confirmed by capture receiver.");
                self.announce("No pending receiver operation.");
            }
            _ => return Err(format!("unknown capture scenario {scenario:?}; use connected, source-picker, unavailable, settings, receivers, diagnostics, or messages")),
        }
        self.capture_scenario = Some(scenario.into());
        Ok(())
    }
    fn command(&mut self, command: BridgeCommand) -> Task<Message> {
        let bridge = self.bridge.clone();
        Task::perform(
            async move { bridge.send(command).await },
            Message::CommandFinished,
        )
    }

    fn selected_capabilities(&self) -> ModelCapabilities {
        let model = self
            .selection
            .as_ref()
            .and_then(|selection| selection.identity().model)
            .map(|model| Model::from_reported(&model))
            .unwrap_or(Model::Unknown);
        ModelCapabilities::for_model(model)
            .with_validated_phase8(self.validated_phase8.unwrap_or_default())
    }

    fn invalidate_phase8(&mut self) {
        self.quick_select.invalidate();
        self.eq_status.invalidate();
        self.quick_select.generation = self.generation;
        self.eq_status.generation = self.generation;
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ConfigLoaded(Ok(config)) => {
                self.configured = config;
                let message: String = if let Some((name, identity)) = self.configured.current() {
                    self.selection = Some(ReceiverSelection::Saved {
                        name: name.into(),
                        identity: identity.clone(),
                    });
                    "Saved receiver selected; connecting…".into()
                } else {
                    "Connect a receiver to see its status.".into()
                };
                self.announce(message);
                if let Some(selection) = self.selection.clone() {
                    let id = self.next_request();
                    self.command(BridgeCommand::Select(id, selection))
                } else {
                    Task::none()
                }
            }
            Message::ConfigLoaded(Err(error)) => {
                self.announce(format!("Configuration unavailable: {error}"));
                Task::none()
            }
            Message::CommandFinished(Err(error)) => {
                self.announce(format!("Operation failed: {error}"));
                Task::none()
            }
            Message::CommandFinished(Ok(())) => Task::none(),
            Message::Navigate(route) => {
                self.route = route;
                Task::none()
            }
            Message::ToggleMessages => {
                self.messages_collapsed = !self.messages_collapsed;
                Task::none()
            }
            Message::ClearMessages => {
                self.messages.clear();
                self.announcement = "Message history cleared.".into();
                Task::none()
            }
            Message::SetTextScale(scale) => {
                self.text_scale = scale.clamp(100, 200);
                self.announce(format!(
                    "Text scale set to {}% for this session.",
                    self.text_scale
                ));
                Task::none()
            }
            Message::SetMotion(preference) => {
                self.motion_preference = preference;
                self.announce(match preference {
                    MotionPreference::Normal => "Normal motion enabled for this session.",
                    MotionPreference::Reduced => "Reduced motion enabled for this session.",
                });
                Task::none()
            }
            Message::SetContrast(preference) => {
                self.contrast_preference = preference;
                self.announce(match preference {
                    ContrastPreference::Normal => "Normal contrast enabled for this session.",
                    ContrastPreference::High => "High contrast enabled for this session.",
                });
                Task::none()
            }
            Message::CaptureVisual => {
                if self.capture_destination().is_none() {
                    self.announce("Visual capture is disabled. Set DENON_AVR_CAPTURE_DIR to an explicit output directory.");
                    return Task::none();
                }
                capture_window()
            }
            Message::ScreenshotCaptured(screenshot) => {
                let Some(destination) = self.capture_destination() else {
                    self.announce(
                        "Discarded visual capture because DENON_AVR_CAPTURE_DIR is not set.",
                    );
                    return Task::none();
                };
                Task::perform(
                    async move { capture::write_png(screenshot, &destination) },
                    Message::ScreenshotWritten,
                )
            }
            Message::ScreenshotWritten(Ok(path)) => {
                self.announce(format!("Wrote native PNG capture to {}.", path.display()));
                Task::none()
            }
            Message::ScreenshotWritten(Err(error)) => {
                self.announce(format!("Could not write native PNG capture: {error}"));
                Task::none()
            }
            Message::AddressChanged(value) => {
                self.address = value;
                Task::none()
            }
            Message::NameChanged(value) => {
                self.name = value;
                Task::none()
            }
            Message::Select(selection) => {
                self.selection = Some(selection.clone());
                self.route = Route::Dashboard;
                self.snapshot.invalidate();
                self.invalidate_phase8();
                self.status_confirmed_generation = None;
                let id = self.next_request();
                self.announce(format!(
                    "{} selected; connecting for confirmed status…",
                    selection_label(&selection)
                ));
                self.command(BridgeCommand::Select(id, selection))
            }
            Message::Bridge(event) => {
                let event = *event;
                if event.request_id < self.request_id
                    || (event.generation != 0
                        && self.generation != 0
                        && event.generation < self.generation)
                {
                    return Task::none();
                }
                self.request_id = event.request_id;
                if event.generation > self.generation {
                    self.generation = event.generation;
                }
                match event.event {
                    ReceiverEvent::Lifecycle(lifecycle) => {
                        if matches!(
                            &lifecycle,
                            denon_avr_application::Lifecycle::Selected
                                | denon_avr_application::Lifecycle::Reconnecting { .. }
                                | denon_avr_application::Lifecycle::Disconnected
                        ) {
                            self.invalidate_phase8();
                        }
                        if matches!(
                            &lifecycle,
                            denon_avr_application::Lifecycle::Reconnecting { .. }
                                | denon_avr_application::Lifecycle::Disconnected
                        ) {
                            self.snapshot.invalidate();
                        }
                        if matches!(
                            &lifecycle,
                            denon_avr_application::Lifecycle::Selected
                                | denon_avr_application::Lifecycle::Connected { .. }
                                | denon_avr_application::Lifecycle::Reconnecting { .. }
                                | denon_avr_application::Lifecycle::Disconnected
                        ) {
                            self.status_confirmed_generation = None;
                        }
                        self.lifecycle = lifecycle;
                    }
                    ReceiverEvent::Snapshot(snapshot) => {
                        // Replace the stale "connecting for confirmed status…" message
                        // with a connected/status-confirmed message once core status is available.
                        if let Some(selection) = &self.selection {
                            if snapshot.power.value().is_some()
                                && self.status_confirmed_generation != Some(self.generation)
                            {
                                self.announce(format!(
                                    "{} selected; connected; status confirmed.",
                                    selection_label(selection)
                                ));
                                self.status_confirmed_generation = Some(self.generation);
                            }
                        }
                        self.volume_slider = slider_volume(&snapshot).unwrap_or(self.volume_slider);
                        self.snapshot = snapshot;
                    }
                    ReceiverEvent::QuickSelect(snapshot) => self.quick_select = *snapshot,
                    ReceiverEvent::EqStatus(status) => self.eq_status = *status,
                    ReceiverEvent::QuickSelectRecall(outcome) => {
                        self.announce(feedback::quick_select_recall_message(&outcome))
                    }
                    ReceiverEvent::Control(result) => {
                        self.announce(feedback::control_message(&result))
                    }
                    ReceiverEvent::FieldError { field, error } => {
                        self.announce(format!("{} unavailable: {}", field.name(), error.message))
                    }
                    ReceiverEvent::Diagnostic(diagnostic) => {
                        self.announce(format!("Diagnostic: {diagnostic:?}"))
                    }
                    _ => {}
                }
                Task::none()
            }
            Message::Connect => {
                let id = self.next_request();
                self.command(BridgeCommand::Connect(id))
            }
            Message::Refresh => {
                let id = self.next_request();
                self.command(BridgeCommand::Refresh(id))
            }
            Message::Disconnect => {
                let id = self.next_request();
                self.command(BridgeCommand::Disconnect(id))
            }
            Message::PowerOn => self.control(MainZoneControl::Power(PowerState::On)),
            Message::PowerOff => self.control(MainZoneControl::Power(PowerState::Standby)),
            Message::Mute => self.control(MainZoneControl::Mute(denon_avr_domain::MuteState::On)),
            Message::Unmute => {
                self.control(MainZoneControl::Mute(denon_avr_domain::MuteState::Off))
            }
            Message::VolumeChanged(value) => {
                if value > MAX_VOLUME {
                    self.volume_slider = MAX_VOLUME;
                    self.announce("Volume is limited to 60.0.");
                } else {
                    self.volume_slider = value.clamp(0.0, MAX_VOLUME);
                }
                self.show_volume_value()
            }
            Message::CommitVolume => match volume_level_for_slider(self.volume_slider) {
                Ok(level) => self.control(MainZoneControl::Volume(level)),
                Err(error) => {
                    self.announce(error);
                    Task::none()
                }
            },
            Message::AdjustVolume(delta) => self.adjust_volume(delta),
            Message::HideVolumeValue(request_id) => {
                if request_id == self.volume_value_request_id {
                    self.volume_value = None;
                }
                Task::none()
            }
            Message::SelectListeningModeGroup(group) => {
                self.control(MainZoneControl::ListeningModeGroup(group))
            }
            Message::SelectSurroundMode(value) => {
                match denon_avr_domain::SurroundMode::new(value) {
                    Ok(mode) => self.control(MainZoneControl::SurroundMode(mode)),
                    Err(error) => {
                        self.announce(error);
                        Task::none()
                    }
                }
            }
            Message::OpenSourcePicker => {
                self.source_picker_open = true;
                Task::none()
            }
            Message::CloseSourcePicker => {
                self.source_picker_open = false;
                Task::none()
            }
            Message::SelectInput(value) => {
                self.source_picker_open = false;
                match self.selected_capabilities().input(&value) {
                    Ok(input) => self.control(MainZoneControl::Input(input)),
                    Err(error) => {
                        self.announce(error);
                        Task::none()
                    }
                }
            }
            Message::RefreshPhase8 => {
                let id = self.next_request();
                self.announce("Refreshing EQ status…");
                self.command(BridgeCommand::RefreshPhase8(id))
            }
            Message::RecallQuickSelect(slot) => {
                let id = self.next_request();
                self.announce(format!(
                    "Recalling Main Zone Quick Select {}…",
                    slot.number()
                ));
                self.command(BridgeCommand::RecallQuickSelect(
                    id,
                    slot,
                    Some(self.quick_select.resource_version()),
                ))
            }
            Message::Discover => {
                self.announce("Searching for receivers…");
                let discovery = Arc::clone(&self.discovery);
                Task::perform(
                    async move {
                        discovery
                            .discover(Duration::from_secs(3))
                            .await
                            .map_err(|error| error.to_string())
                    },
                    Message::DiscoveryFinished,
                )
            }
            Message::DiscoveryFinished(Ok(receivers)) => {
                self.discovered = receivers;
                if let [receiver] = self.discovered.as_slice() {
                    let receiver = receiver.clone();
                    self.announce("Receiver discovered; saving it as the current receiver…");
                    self.update(Message::SaveDiscovered(receiver))
                } else {
                    self.announce(format!("Found {} receiver(s).", self.discovered.len()));
                    Task::none()
                }
            }
            Message::DiscoveryFinished(Err(error)) => {
                self.announce(format!("Discovery failed: {error}"));
                Task::none()
            }
            Message::SaveDiscovered(receiver) => {
                let (config, selection) = configuration_for_discovered(&receiver);
                let configuration = Arc::clone(&self.configuration);
                self.announce(format!(
                    "Saving {} as the current receiver…",
                    selection_label(&selection)
                ));
                Task::perform(
                    async move {
                        configuration
                            .save(&config)
                            .await
                            .map(|_| (config, selection))
                            .map_err(|error| error.to_string())
                    },
                    Message::DiscoveredSaved,
                )
            }
            Message::DiscoveredSaved(Ok((config, selection))) => {
                self.configured = config;
                self.announce("Receiver saved; connecting…");
                self.update(Message::Select(selection))
            }
            Message::DiscoveredSaved(Err(error)) => {
                self.announce(format!("Could not save discovered receiver: {error}"));
                Task::none()
            }
            Message::ManualSetup => {
                if self.address.trim().is_empty() {
                    self.announce("Enter a receiver address first.");
                    return Task::none();
                }
                let identity = denon_avr_domain::ReceiverIdentity {
                    host: self.address.trim().to_owned(),
                    model: None,
                    friendly_name: (!self.name.trim().is_empty())
                        .then(|| self.name.trim().to_owned()),
                };
                let name = identity
                    .friendly_name
                    .clone()
                    .unwrap_or_else(|| identity.host.clone());
                let config = ConfiguredReceivers {
                    current: Some(name.clone()),
                    receivers: BTreeMap::from([(name.clone(), identity.clone())]),
                };
                let selection = ReceiverSelection::Saved { name, identity };
                let configuration = Arc::clone(&self.configuration);
                self.announce(format!(
                    "Saving {} as the current receiver…",
                    selection_label(&selection)
                ));
                Task::perform(
                    async move {
                        configuration
                            .save(&config)
                            .await
                            .map(|_| (config, selection))
                            .map_err(|error| error.to_string())
                    },
                    Message::ManualSaved,
                )
            }
            Message::ManualSaved(Ok((config, selection))) => {
                self.configured = config;
                self.announce("Receiver saved; connecting…");
                self.update(Message::Select(selection))
            }
            Message::ManualSaved(Err(error)) => {
                self.announce(format!("Could not save receiver: {error}"));
                Task::none()
            }
            Message::Shutdown => {
                let id = self.next_request();
                self.command(BridgeCommand::Shutdown(id))
            }
        }
    }

    fn control(&mut self, control: MainZoneControl) -> Task<Message> {
        let id = self.next_request();
        self.announce("Command pending; waiting for receiver confirmation…");
        self.command(BridgeCommand::Control(
            id,
            control,
            Some(self.snapshot.resource_version()),
        ))
    }

    fn adjust_volume(&mut self, delta: f32) -> Task<Message> {
        let requested = self.volume_slider + delta;
        let target = requested.clamp(0.0, MAX_VOLUME);
        let changed = target != self.volume_slider;
        self.volume_slider = target;
        let value_task = changed.then(|| self.show_volume_value());
        let task = if changed {
            match volume_level_for_slider(target) {
                Ok(level) => self.control(MainZoneControl::Volume(level)),
                Err(error) => {
                    self.announce(error);
                    Task::none()
                }
            }
        } else {
            Task::none()
        };
        if requested > MAX_VOLUME {
            self.announce("Volume is limited to 60.0.");
        }
        match value_task {
            Some(value_task) => Task::batch([task, value_task]),
            None => task,
        }
    }

    fn show_volume_value(&mut self) -> Task<Message> {
        self.volume_value_request_id = self.volume_value_request_id.saturating_add(1);
        self.volume_value = Some(self.volume_slider);
        let request_id = self.volume_value_request_id;
        Task::perform(
            async move {
                tokio::time::sleep(Duration::from_secs(3)).await;
                request_id
            },
            Message::HideVolumeValue,
        )
    }

    pub fn view(&self) -> Element<'_, Message> {
        let rail = container(
            column![
                text("MAIN ZONE").size(12).color(design::MUTED),
                components::nav(
                    "Dashboard",
                    Route::Dashboard,
                    self.route == Route::Dashboard
                ),
                components::nav(
                    "Receivers",
                    Route::Receivers,
                    self.route == Route::Receivers
                ),
                components::nav("Settings", Route::Settings, self.route == Route::Settings),
                components::nav("Advanced", Route::Advanced, self.route == Route::Advanced),
                components::nav(
                    "Diagnostics",
                    Route::Diagnostics,
                    self.route == Route::Diagnostics
                ),
                space().height(Length::Fill),
                text("Wide console").size(12).color(design::MUTED),
                text("Native keyboard traversal")
                    .size(12)
                    .color(design::MUTED),
            ]
            .spacing(14)
            .padding(20),
        )
        .width(Length::Fixed(design::RAIL))
        .height(Length::Fill)
        .style(design::rail);
        let toolbar: Element<'_, Message> = if self.route == Route::Dashboard {
            space().height(Length::Shrink).into()
        } else {
            row![column![
                text(route_title(self.route)).size(28),
                text("Main Zone receiver context")
                    .size(13)
                    .color(design::MUTED)
            ]
            .spacing(4)
            .width(Length::Fill),]
            .align_y(iced::Alignment::Center)
            .spacing(16)
            .into()
        };
        let base_body: Element<'_, Message> = match self.route {
            Route::Dashboard => self.dashboard().into(),
            Route::Receivers => self.receivers().into(),
            Route::Settings => self.settings().into(),
            Route::Advanced => self.advanced().into(),
            Route::Diagnostics => self.diagnostics().into(),
        };
        let body: Element<'_, Message> = base_body;
        let messages: Element<'_, Message> = if self.messages_collapsed {
            container(
                row![
                    text("Message history collapsed")
                        .size(13)
                        .color(design::MUTED),
                    components::quiet_action("Expand", Message::ToggleMessages)
                ]
                .spacing(12)
                .align_y(iced::Alignment::Center),
            )
            .width(Length::Fill)
            .into()
        } else {
            let items = self
                .messages
                .iter()
                .fold(column![].spacing(5), |column, message| {
                    column.push(text(message).size(13))
                });
            container(
                column![
                    row![
                        text("Session messages").size(16),
                        space().width(Length::Fill),
                        components::quiet_icon_action("⌫", Message::ClearMessages),
                        components::quiet_icon_action("⌃", Message::ToggleMessages)
                    ]
                    .spacing(12),
                    scrollable(items)
                        .width(Length::Fill)
                        .height(Length::Fixed(112.0))
                        .anchor_bottom()
                ]
                .spacing(10)
                .padding(14),
            )
            .width(Length::Fill)
            .style(design::panel)
            .into()
        };
        container(
            row![
                rail,
                container(
                    column![toolbar, scrollable(body).height(Length::Fill), messages]
                        .spacing(18)
                        .padding(28)
                        .width(Length::Fill)
                )
                .width(Length::Fill)
                .height(Length::Fill)
            ]
            .height(Length::Fill)
            .width(Length::Fill),
        )
        .style(|_| iced::widget::container::Style {
            background: Some(iced::Background::Color(design::CANVAS)),
            text_color: Some(design::TEXT),
            ..Default::default()
        })
        .into()
    }

    fn settings(&self) -> iced::widget::Column<'_, Message> {
        let scale_controls = [
            (100_u8, "100%"),
            (125, "125%"),
            (150, "150%"),
            (175, "175%"),
            (200, "200%"),
        ]
        .into_iter()
        .fold(row![].spacing(8), |row, (scale, label)| {
            row.push(components::toggle_action(
                "Aa",
                label,
                self.text_scale == scale,
                Message::SetTextScale(scale),
            ))
        });
        column![
            text("Settings").size(32),
            components::panel("Appearance", column![
                text("Dark console theme").size(16),
                text("Session accessibility overrides are never persisted. When a reliable host preference is available it is the starting point; these controls take precedence.").color(design::MUTED),
                text("Text scale").color(design::MUTED),
                scale_controls,
                text("Motion").color(design::MUTED),
                row![
                    components::toggle_action("◌", "Normal", self.motion_preference == MotionPreference::Normal, Message::SetMotion(MotionPreference::Normal)),
                    components::toggle_action("◐", "Reduced", self.motion_preference == MotionPreference::Reduced, Message::SetMotion(MotionPreference::Reduced)),
                ].spacing(8),
                text("Contrast").color(design::MUTED),
                row![
                    components::toggle_action("◒", "Normal", self.contrast_preference == ContrastPreference::Normal, Message::SetContrast(ContrastPreference::Normal)),
                    components::toggle_action("◑", "High", self.contrast_preference == ContrastPreference::High, Message::SetContrast(ContrastPreference::High)),
                ].spacing(8),
                components::quiet_action("Capture current screen", Message::CaptureVisual),
                text("Capture is enabled only when DENON_AVR_CAPTURE_DIR names an explicit directory. Files are native RGBA PNGs organized by platform, route, and text scale.").color(design::MUTED),
                text("Screen-reader semantic support is release-blocked pending an Iced native accessibility bridge and three-platform audit.").color(design::MUTED),
            ].spacing(10)),
            components::panel("Configuration", column![text("YAML configuration is managed by the desktop host."), text("Quick Select slot editing requires validated receiver support.").color(design::MUTED)])
        ].spacing(18)
    }

    fn advanced(&self) -> iced::widget::Column<'_, Message> {
        column![components::panel(
            "Receiver status",
            column![
                text("Request a fresh authoritative Main Zone status snapshot.")
                    .color(design::MUTED),
                components::action("Refresh Status", Message::Refresh),
            ]
            .spacing(12),
        ),]
        .spacing(18)
    }

    fn diagnostics(&self) -> iced::widget::Column<'_, Message> {
        column![
            text("Diagnostics").size(32),
            components::panel(
                "Connection",
                column![
                    components::state_row("Lifecycle", lifecycle_label(&self.lifecycle).into()),
                    components::state_row("Generation", self.generation.to_string()),
                    components::state_row(
                        "Selected receiver",
                        self.selection
                            .as_ref()
                            .map(|s| s.identity().host.clone())
                            .unwrap_or_else(|| "None".into())
                    )
                ]
            ),
            components::panel(
                "Observed state",
                column![
                    components::state_row(
                        "Snapshot authority",
                        format!("{:?}", self.snapshot.authority)
                    ),
                    components::state_row(
                        "Quick Select freshness",
                        format!("{:?}", self.quick_select.freshness)
                    ),
                    text(feedback::eq_summary(&self.eq_status)),
                    text(feedback::eq_evidence_summary(&self.eq_status)).color(design::MUTED),
                    text(
                        "Unknown, unavailable, and not-applicable states remain distinct from Off."
                    )
                    .color(design::MUTED)
                ]
            )
        ]
        .spacing(18)
    }

    fn dashboard(&self) -> iced::widget::Column<'_, Message> {
        let phase8_capabilities = self.selected_capabilities();
        let writable = phase8_capabilities.writable;
        let power_action = if writable {
            match self.snapshot.power.value() {
                Some(PowerState::On) => Some(Message::PowerOff),
                Some(PowerState::Standby) => Some(Message::PowerOn),
                None => None,
            }
        } else {
            None
        };
        let header = dashboard_header(self.snapshot.power.value(), power_action);
        if self.snapshot.power.value() == Some(&PowerState::Standby) {
            return column![container(stack![
                container(text("POWER OFF").size(36).color(design::MUTED))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .center(Length::Fill),
                container(header).width(Length::Fill)
            ])
            .width(Length::Fill)
            .height(Length::Fixed(600.0))];
        }
        if self.snapshot.power.value() != Some(&PowerState::On) {
            let (title, detail, action) = power_recovery(&self.lifecycle, &self.snapshot);
            let action: Element<'_, Message> = action.map_or_else(
                || space().into(),
                |(label, message)| components::action(label, message).into(),
            );
            return column![
                header,
                container(
                    column![
                        text(title).size(28).color(design::MUTED),
                        text(detail).size(15).color(design::MUTED),
                        action,
                    ]
                    .spacing(14)
                    .align_x(iced::Alignment::Center)
                )
                .width(Length::Fill)
                .height(Length::Fixed(520.0))
                .center(Length::Fill)
            ]
            .spacing(18);
        }

        let input = self
            .snapshot
            .input
            .value()
            .map(ToString::to_string)
            .unwrap_or_else(|| "Unavailable".into());
        let quick_select_supported = phase8_capabilities.quick_select_recall;
        let phase8_status_supported = phase8_capabilities.eq_status;
        let group_controls = if writable {
            ListeningModeGroup::ALL
                .into_iter()
                .fold(row![].spacing(12), |row, group| {
                    row.push(components::quiet_action(
                        format!("{}  {}", mode_icon(group), group.as_str()),
                        Message::SelectListeningModeGroup(group),
                    ))
                })
        } else {
            row![text("Mode groups unavailable for this receiver.")]
        };
        let mute_controls: Element<'_, Message> = if writable {
            let muted = self.snapshot.mute.value() == Some(&denon_avr_domain::MuteState::On);
            components::toggle_action(
                if muted { "🔇" } else { "🔊" },
                "Mute",
                muted,
                if muted {
                    Message::Unmute
                } else {
                    Message::Mute
                },
            )
            .into()
        } else {
            text("Controls unavailable: receiver model is not validated for writes.")
                .color(design::MUTED)
                .into()
        };
        let input_layout = self
            .snapshot
            .audio_context
            .input_channels
            .value
            .known()
            .map(|v| v.as_str());
        let output_layout = self
            .snapshot
            .audio_context
            .output_channels
            .value
            .known()
            .map(|v| v.as_str());
        let eq = phase8_status_supported
            .then(|| eq_summary_if_reported(&self.eq_status))
            .flatten()
            .unwrap_or_default();
        let eq_refresh: Element<'_, Message> = if phase8_status_supported {
            components::quiet_action("Refresh", Message::RefreshPhase8).into()
        } else {
            space().into()
        };
        column![
            header,
            dashboard_context_line(&input, self.source_picker_open, phase8_capabilities),
            row![
                container(
                    column![
                        container(text("INPUT").size(14).color(iced::Color::WHITE))
                            .width(Length::Fill)
                            .padding(iced::Padding::ZERO.top(10))
                            .center_x(Length::Fill),
                        container(input_channel_grid(input_layout))
                            .width(Length::Fill)
                            .height(Length::Fill)
                            .padding(iced::Padding::ZERO.bottom(12))
                            .center(Length::Fill)
                    ]
                    .height(Length::Fill)
                )
                .width(Length::FillPortion(2))
                .height(Length::Fixed(190.0))
                .style(design::panel),
                container(
                    column![
                        container(text("OUTPUT").size(14).color(iced::Color::WHITE))
                            .width(Length::Fill)
                            .padding(iced::Padding::ZERO.top(10))
                            .center_x(Length::Fill),
                        container(output_channel_grid(output_layout))
                            .width(Length::Fill)
                            .height(Length::Fill)
                            .padding(iced::Padding::ZERO.bottom(12))
                            .center(Length::Fill)
                    ]
                    .height(Length::Fill)
                )
                .width(Length::FillPortion(2))
                .height(Length::Fixed(190.0))
                .style(design::panel),
                container(column![
                    container(text("AUDYSSEY").size(14).color(iced::Color::WHITE))
                        .width(Length::Fill)
                        .padding(iced::Padding::ZERO.top(10))
                        .center_x(Length::Fill),
                    text(eq).size(13),
                    eq_refresh,
                ])
                .width(Length::FillPortion(1))
                .height(Length::Fixed(190.0))
                .style(design::panel)
            ]
            .spacing(16),
            column![
                container(volume_slider(self.volume_slider, self.volume_value,))
                    .width(Length::Fill)
                    .padding([8, 0]),
                row![
                    row![
                        components::quiet_action("≪", Message::AdjustVolume(-10.0)),
                        components::quiet_action("−", Message::AdjustVolume(-0.5)),
                    ]
                    .spacing(8),
                    space().width(Length::Fill),
                    row![
                        components::quiet_action("+", Message::AdjustVolume(0.5)),
                        components::quiet_action("≫", Message::AdjustVolume(10.0)),
                    ]
                    .spacing(8),
                ]
                .width(Length::Fill)
            ]
            .spacing(8),
            row![mute_controls, group_controls]
                .spacing(14)
                .align_y(iced::Alignment::Center),
            quick_select_bar(&self.quick_select, quick_select_supported),
        ]
        .spacing(18)
    }

    fn receivers(&self) -> iced::widget::Column<'_, Message> {
        let mut page = column![text("Receivers").size(32), text("Saved receivers")].spacing(12);
        for (name, identity) in &self.configured.receivers {
            page = page.push(
                row![
                    text(format!("{name} · {}", identity.host)),
                    components::quiet_action(
                        "Select",
                        Message::Select(ReceiverSelection::Saved {
                            name: name.clone(),
                            identity: identity.clone()
                        })
                    )
                ]
                .spacing(12),
            );
        }
        for receiver in &self.discovered {
            page = page.push(
                row![
                    column![
                        text(receiver.model.as_deref().unwrap_or("Unknown model")).size(16),
                        text(format!("Address · {}:{}", receiver.address.host, receiver.address.port)).color(design::MUTED),
                        text("Discovered on the local network; selecting opens the Main Zone console.").size(12).color(design::MUTED)
                    ].spacing(4).width(Length::Fill),
                    components::action("Save and open console", Message::SaveDiscovered(receiver.clone()))
                ]
                .spacing(16)
                .align_y(iced::Alignment::Center),
            );
        }
        page.push(text("Find a receiver"))
            .push(components::quiet_action("Discover", Message::Discover))
            .push(
                row![
                    text_input("Receiver address", &self.address)
                        .on_input(Message::AddressChanged)
                        .style(design::text_field),
                    text_input("Name (optional)", &self.name)
                        .on_input(Message::NameChanged)
                        .style(design::text_field),
                    components::action("Save and connect", Message::ManualSetup)
                ]
                .spacing(8),
            )
    }
}

const MAX_VOLUME: f32 = 60.0;
const VOLUME_INDICATOR_HEIGHT: f32 = 28.0;

fn slider_volume(snapshot: &MainZoneSnapshot) -> Option<f32> {
    snapshot
        .volume
        .value()
        // AVR volume codes are absolute volume in 0.5-step units. Display
        // that direct receiver value instead of the domain's normalized
        // 0–100 control value, which can round 24.5 up to 25.0.
        .map(|volume| (volume.native_code() as f32 / 10.0).min(MAX_VOLUME))
}

fn volume_level_for_slider(value: f32) -> Result<denon_avr_domain::VolumeLevel, String> {
    if value > MAX_VOLUME {
        return Err("Volume is limited to 60.0.".into());
    }
    let native_code = (value.clamp(0.0, MAX_VOLUME) * 10.0).round() as u16;
    denon_avr_domain::VolumeLevel::from_native_code(native_code).map_err(str::to_owned)
}

fn volume_slider<'a>(value: f32, shown_value: Option<f32>) -> Element<'a, Message> {
    let bubble: Element<'a, Message> = shown_value.map_or_else(
        || space().into(),
        |shown_value| {
            let steps = (shown_value.clamp(0.0, MAX_VOLUME) * 2.0).round() as u16;
            let leading = steps.max(1);
            let trailing = (120_u16.saturating_sub(steps)).max(1);
            row![
                space().width(Length::FillPortion(leading)),
                container(text(format!("{shown_value:.1}")).size(13))
                    .padding([3, 7])
                    .style(design::panel),
                space().width(Length::FillPortion(trailing)),
            ]
            .width(Length::Fill)
            .align_y(iced::Alignment::Center)
            .into()
        },
    );
    column![
        // Always reserve the bubble lane. Changing the slider's vertical
        // position while a pointer drag is in progress causes visible jitter.
        container(bubble)
            .width(Length::Fill)
            .height(Length::Fixed(VOLUME_INDICATOR_HEIGHT)),
        slider(0.0..=MAX_VOLUME, value, Message::VolumeChanged)
            .step(0.5_f32)
            .on_release(Message::CommitVolume)
            .width(Length::Fill),
    ]
    .spacing(2)
    .into()
}

fn power_recovery(
    lifecycle: &denon_avr_application::Lifecycle,
    snapshot: &MainZoneSnapshot,
) -> (&'static str, String, Option<(&'static str, Message)>) {
    let error = match &snapshot.power {
        denon_avr_domain::FieldStatus::Unavailable(error) => error.message.as_str(),
        denon_avr_domain::FieldStatus::Value(_) => "power status is not currently available",
    };
    match lifecycle {
        denon_avr_application::Lifecycle::Connecting
        | denon_avr_application::Lifecycle::Selected => (
            "CONNECTING",
            "Connecting to the selected receiver and requesting its status.".into(),
            None,
        ),
        denon_avr_application::Lifecycle::Reconnecting { .. } => (
            "RECONNECTING",
            "The receiver connection was interrupted; status refresh is being retried.".into(),
            None,
        ),
        denon_avr_application::Lifecycle::Disconnected => (
            "RECEIVER UNAVAILABLE",
            format!("Could not read power status: {error}"),
            Some(("Retry Status", Message::Refresh)),
        ),
        denon_avr_application::Lifecycle::NoReceiver => (
            "NO RECEIVER SELECTED",
            "Choose a receiver before requesting Main Zone status.".into(),
            Some(("Choose Receiver", Message::Navigate(Route::Receivers))),
        ),
        denon_avr_application::Lifecycle::Connected { .. } => (
            "POWER STATUS UNAVAILABLE",
            format!("Could not read power status: {error}"),
            Some(("Retry Status", Message::Refresh)),
        ),
        denon_avr_application::Lifecycle::Stopping | denon_avr_application::Lifecycle::Stopped => (
            "CONNECTION CLOSED",
            "The receiver session is stopping or has stopped.".into(),
            None,
        ),
    }
}

fn dashboard_header(
    power: Option<&PowerState>,
    power_action: Option<Message>,
) -> Element<'static, Message> {
    let (icon, color) = match power {
        Some(PowerState::On) => ("⏻", design::SUCCESS),
        Some(PowerState::Standby) => ("⏻", design::MUTED),
        None => ("?", design::WARNING),
    };
    let power_button = match power_action {
        Some(action) => button(text(icon).size(24).color(color))
            .padding([6, 10])
            .style(design::secondary)
            .on_press(action),
        None => button(text(icon).size(24).color(color))
            .padding([6, 10])
            .style(design::secondary),
    };
    row![
        container(power_button).align_right(Length::Fill),
        container(text("RECEIVER").size(13).color(design::MUTED)).center_x(Length::Fixed(120.0)),
        space().width(Length::Fill)
    ]
    .align_y(iced::Alignment::Center)
    .into()
}

fn dashboard_context_line(
    source: &str,
    source_picker_open: bool,
    capabilities: ModelCapabilities,
) -> Element<'static, Message> {
    let context = row![
        button(
            row![
                text(source_icon(source)).size(18).color(design::ACCENT),
                text(source_label(source)).size(15)
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center)
        )
        .padding([6, 10])
        .style(design::secondary)
        .on_press(Message::OpenSourcePicker),
        container(space()).width(Length::Fill),
        container(text("Main Zone").size(14).color(design::MUTED)).center_x(Length::Fixed(120.0)),
        space().width(Length::Fill)
    ]
    .align_y(iced::Alignment::Center);

    if source_picker_open {
        column![context, source_picker_popup(capabilities)]
            .spacing(8)
            .into()
    } else {
        context.into()
    }
}

fn source_icon(source: &str) -> &'static str {
    match source.to_ascii_uppercase().as_str() {
        "TV" | "TV AUDIO" => "▣",
        "BT" | "BLUETOOTH" => "ᛒ",
        "HEOS" | "NETWORK" => "◉",
        "PHONO" => "◌",
        "CD" => "◍",
        "GAME" => "◇",
        _ => "●",
    }
}

fn source_label(source: &str) -> String {
    match source.to_ascii_uppercase().as_str() {
        "TV" | "TV AUDIO" => "TV Audio".into(),
        "BT" | "BLUETOOTH" => "Bluetooth".into(),
        "HEOS" | "NETWORK" => "Network".into(),
        _ => source.to_owned(),
    }
}

fn source_picker_popup(capabilities: ModelCapabilities) -> Element<'static, Message> {
    let choices: Element<'static, Message> = if capabilities.inputs.is_empty() {
        text("Source selection is unavailable for this receiver.")
            .color(design::MUTED)
            .into()
    } else {
        capabilities
            .inputs
            .iter()
            .fold(column![].spacing(7), |column, source| {
                column.push(
                    components::quiet_action(
                        source_label(source),
                        Message::SelectInput((*source).into()),
                    )
                    .width(Length::Fill),
                )
            })
            .into()
    };
    container(
        column![
            row![
                text("Change source").size(18),
                space().width(Length::Fill),
                components::quiet_icon_action("×", Message::CloseSourcePicker),
            ]
            .align_y(iced::Alignment::Center),
            scrollable(choices).height(Length::Fixed(440.0)),
        ]
        .spacing(12)
        .padding(14),
    )
    .width(Length::Fixed(300.0))
    .style(design::panel)
    .into()
}

fn input_channel_grid<'a>(layout: Option<&str>) -> iced::widget::Column<'a, Message> {
    column![
        channel_grid_row(
            &[Some("FHL"), Some("LEF"), None, Some("EXT"), Some("FHR")],
            layout
        ),
        channel_grid_row(
            &[Some("FWL"), Some("FL"), Some("C"), Some("FR"), Some("FWR")],
            layout
        ),
        channel_grid_row(&[None, Some("SL"), None, Some("SR"), None], layout),
        channel_grid_row(&[None, Some("SBL"), Some("SB"), Some("SBR"), None], layout),
    ]
    .spacing(6)
}

fn output_channel_grid<'a>(layout: Option<&str>) -> iced::widget::Column<'a, Message> {
    column![
        channel_grid_row(&[Some("FL"), Some("C"), Some("FR")], layout),
        channel_grid_row(&[Some("SL"), None, Some("SR")], layout),
    ]
    .spacing(6)
}

fn channel_grid_row<'a>(
    channels: &[Option<&'static str>],
    layout: Option<&str>,
) -> iced::widget::Row<'a, Message> {
    channels.iter().fold(row![].spacing(6), |row, channel| {
        row.push(match *channel {
            Some(label) => channel_box(label, channel_state(layout, label)),
            None => space()
                .width(Length::Fixed(44.0))
                .height(Length::Fixed(32.0))
                .into(),
        })
    })
}

fn channel_box<'a>(label: &'static str, state: &'static str) -> Element<'a, Message> {
    let active = state == "ON";
    let foreground = if active {
        design::SUCCESS
    } else {
        design::MUTED
    };
    let background = if active {
        iced::Color {
            a: 0.28,
            ..design::SUCCESS
        }
    } else {
        design::ACTIVE
    };
    let shadow = if active {
        iced::Shadow {
            color: iced::Color {
                a: 0.45,
                ..design::SUCCESS
            },
            offset: iced::Vector::new(0.0, 0.0),
            blur_radius: 10.0,
        }
    } else {
        iced::Shadow::default()
    };
    container(text(label).size(10).color(foreground))
        .center_x(Length::Fixed(44.0))
        .center_y(Length::Fixed(32.0))
        .style(move |_| iced::widget::container::Style {
            background: Some(iced::Background::Color(background)),
            text_color: Some(foreground),
            border: iced::Border {
                radius: 5.0.into(),
                width: 1.0,
                color: foreground,
            },
            shadow,
            ..Default::default()
        })
        .into()
}

fn channel_state(layout: Option<&str>, channel: &str) -> &'static str {
    let Some(layout) = layout else {
        return "UNKNOWN";
    };
    let tokens = layout
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_uppercase)
        .collect::<Vec<_>>();
    if !tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "FHL"
                | "LEF"
                | "LFE"
                | "EXT"
                | "FHR"
                | "FWL"
                | "FL"
                | "C"
                | "FR"
                | "FWR"
                | "SL"
                | "SR"
                | "SBL"
                | "SB"
                | "SBR"
        )
    }) {
        return "UNKNOWN";
    }
    let channel_matches = |token: &str| match channel {
        "LEF" => matches!(token, "LEF" | "LFE"),
        _ => token == channel,
    };
    if tokens.iter().any(|token| channel_matches(token)) {
        "ON"
    } else {
        "OFF"
    }
}

fn mode_icon(group: ListeningModeGroup) -> &'static str {
    match group {
        ListeningModeGroup::Movie => "▣",
        ListeningModeGroup::Music => "♫",
        ListeningModeGroup::Game => "◇",
    }
}

fn quick_select_bar<'a>(snapshot: &QuickSelectSnapshot, supported: bool) -> Element<'a, Message> {
    let content: Element<'a, Message> = if supported {
        container(
            QuickSelectSlot::ALL
                .into_iter()
                .fold(row![].spacing(8), |row, slot| {
                    row.push(
                        button(text(quick_select_slot_label(snapshot, slot)).size(12))
                            .padding([7, 10])
                            .style(design::secondary)
                            .on_press(Message::RecallQuickSelect(slot)),
                    )
                }),
        )
        .width(Length::Fill)
        .center_x(Length::Fill)
        .into()
    } else {
        text("Quick Select names are unavailable until protocol evidence is validated.")
            .size(12)
            .color(design::MUTED)
            .into()
    };
    container(content)
        .width(Length::Fill)
        .padding(10)
        .style(design::panel)
        .into()
}

fn quick_select_slot_label(snapshot: &QuickSelectSnapshot, slot: QuickSelectSlot) -> String {
    let name = match snapshot.preset(slot) {
        Some(preset) if preset.available => preset
            .name
            .as_ref()
            .map(|name| name.as_str())
            .map(str::to_owned),
        _ => None,
    };
    name.map_or_else(
        || slot.number().to_string(),
        |name| format!("{} {name}", slot.number()),
    )
}

/// Unknown EQ observations carry no usable display value. Keep that state
/// distinct internally, but leave the dashboard summary blank until at least
/// one receiver-reported value is available.
fn eq_summary_if_reported(status: &EqStatus) -> Option<String> {
    denon_avr_domain::EqFeature::ALL
        .into_iter()
        .any(|feature| !matches!(status.state(feature), denon_avr_domain::EqState::Unknown))
        .then(|| feedback::eq_summary(status))
}

fn route_title(route: Route) -> &'static str {
    match route {
        Route::Dashboard => "Dashboard",
        Route::Receivers => "Receiver setup",
        Route::Settings => "Settings",
        Route::Advanced => "Advanced",
        Route::Diagnostics => "Diagnostics",
    }
}

fn selection_label(selection: &ReceiverSelection) -> String {
    let identity = selection.identity();
    match (identity.model, identity.friendly_name) {
        (Some(model), Some(name)) => format!("{name} ({model})"),
        (Some(model), None) => model,
        (None, Some(name)) => name,
        (None, None) => identity.host,
    }
}

fn configuration_for_discovered(
    receiver: &denon_avr_domain::DiscoveredReceiver,
) -> (ConfiguredReceivers, ReceiverSelection) {
    let name = receiver
        .model
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| receiver.address.host.clone());
    let mut identity = receiver.identity();
    identity.friendly_name = Some(name.clone());
    let config = ConfiguredReceivers {
        current: Some(name.clone()),
        receivers: BTreeMap::from([(name.clone(), identity.clone())]),
    };
    let selection = ReceiverSelection::Saved { name, identity };
    (config, selection)
}

fn lifecycle_label(lifecycle: &denon_avr_application::Lifecycle) -> &'static str {
    match lifecycle {
        denon_avr_application::Lifecycle::Connected { .. } => "connected",
        denon_avr_application::Lifecycle::Reconnecting { .. } => "reconnecting",
        denon_avr_application::Lifecycle::Disconnected => "disconnected",
        denon_avr_application::Lifecycle::Selected => "selected",
        denon_avr_application::Lifecycle::NoReceiver => "no receiver",
        _ => "waiting",
    }
}

pub fn boot_with_services(services: GuiServices) -> (Gui, Task<Message>) {
    let controller_config = denon_avr_application::ControllerConfig::default();
    let mut gui = Gui::new(ControllerBridge::new_with_config(
        services.factory,
        controller_config,
    ));
    gui.discovery = services.discovery;
    gui.configuration = Arc::clone(&services.configuration);
    if let Some(scenario) = std::env::var_os("DENON_AVR_CAPTURE_SCENARIO") {
        let scenario = scenario.to_string_lossy();
        let message = match gui.configure_capture_scenario(&scenario) {
            Ok(()) => format!("Capture mode enabled for scenario {scenario:?}."),
            Err(error) => format!("Capture mode configuration error: {error}"),
        };
        gui.announce(message);
        if let Some(scale) = std::env::var_os("DENON_AVR_CAPTURE_SCALE") {
            match scale.to_string_lossy().parse::<u8>() {
                Ok(scale @ (100 | 125 | 150 | 175 | 200)) => gui.text_scale = scale,
                _ => {
                    gui.announce("Ignored DENON_AVR_CAPTURE_SCALE; use 100, 125, 150, 175, or 200.")
                }
            }
        }
        let task = if gui.capture_directory.is_some() {
            capture_window()
        } else {
            Task::none()
        };
        return (gui, task);
    }
    let repository = services.configuration;
    (
        gui,
        Task::perform(
            async move { repository.load().await.map_err(|error| error.to_string()) },
            Message::ConfigLoaded,
        ),
    )
}

fn capture_window() -> Task<Message> {
    iced::window::latest().then(|id| match id {
        Some(id) => iced::window::screenshot(id).map(Message::ScreenshotCaptured),
        None => Task::done(Message::ScreenshotWritten(Err(
            "could not capture: no application window exists".into(),
        ))),
    })
}

pub fn update(gui: &mut Gui, message: Message) -> Task<Message> {
    gui.update(message)
}
pub fn app_theme(gui: &Gui) -> iced::Theme {
    design::theme(gui.contrast_preference == ContrastPreference::High)
}
pub fn app_scale(gui: &Gui) -> f32 {
    f32::from(gui.text_scale) / 100.0
}
pub fn view(gui: &Gui) -> Element<'_, Message> {
    gui.view()
}
pub fn subscription(gui: &Gui) -> Subscription<Message> {
    gui.bridge
        .subscription()
        .map(|event| Message::Bridge(Box::new(event)))
}

struct NoopDiscovery;
impl AsyncReceiverDiscovery for NoopDiscovery {
    fn discover(
        &self,
        _timeout: Duration,
    ) -> denon_avr_application::BoxFuture<
        '_,
        Result<Vec<denon_avr_domain::DiscoveredReceiver>, denon_avr_application::OperationError>,
    > {
        Box::pin(async { Ok(Vec::new()) })
    }
}

struct NoopConfiguration;
impl denon_avr_application::AsyncConfigRepository for NoopConfiguration {
    fn load(
        &self,
    ) -> denon_avr_application::BoxFuture<
        '_,
        Result<ConfiguredReceivers, denon_avr_application::OperationError>,
    > {
        Box::pin(async { Ok(ConfiguredReceivers::default()) })
    }

    fn save<'a>(
        &'a self,
        _config: &'a ConfiguredReceivers,
    ) -> denon_avr_application::BoxFuture<'a, Result<(), denon_avr_application::OperationError>>
    {
        Box::pin(async { Ok(()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use denon_avr_infrastructure::AvrSessionFactory;

    #[tokio::test]
    async fn accessibility_overrides_are_session_only_and_update_theme_state() {
        let bridge = ControllerBridge::new(AvrSessionFactory::default());
        let mut gui = Gui::new(bridge);
        let _ = gui.update(Message::SetTextScale(200));
        let _ = gui.update(Message::SetMotion(MotionPreference::Reduced));
        let _ = gui.update(Message::SetContrast(ContrastPreference::High));

        assert_eq!(gui.text_scale, 200);
        assert_eq!(app_scale(&gui), 2.0);
        assert_eq!(gui.motion_preference, MotionPreference::Reduced);
        assert_eq!(gui.contrast_preference, ContrastPreference::High);
        assert!(matches!(app_theme(&gui), iced::Theme::Custom(_)));
    }

    #[tokio::test]
    async fn fixed_capture_scenarios_do_not_need_a_receiver_connection() {
        let bridge = ControllerBridge::new(AvrSessionFactory::default());
        let mut gui = Gui::new(bridge);
        gui.configure_capture_scenario("source-picker").unwrap();
        assert_eq!(
            gui.lifecycle,
            denon_avr_application::Lifecycle::Connected { generation: 1 }
        );
        assert_eq!(gui.snapshot.power.value(), Some(&PowerState::On));
        assert!(gui.source_picker_open);

        gui.configure_capture_scenario("unavailable").unwrap();
        assert_eq!(
            gui.lifecycle,
            denon_avr_application::Lifecycle::Disconnected
        );
        assert!(gui.snapshot.power.value().is_none());

        gui.configure_capture_scenario("diagnostics").unwrap();
        assert_eq!(gui.route, Route::Diagnostics);
        gui.configure_capture_scenario("messages").unwrap();
        assert!(gui.messages.len() >= 3);
    }
    #[tokio::test]
    async fn stale_bridge_results_are_ignored() {
        // State construction is kept independent of widgets, making ordering
        // and stale-result behavior deterministic in unit tests.
        let bridge = ControllerBridge::new(AvrSessionFactory::default());
        let mut gui = Gui::new(bridge);
        gui.request_id = 4;
        let _ = gui.update(Message::Bridge(Box::new(BridgeEvent {
            request_id: 3,
            generation: 0,
            event: ReceiverEvent::Lifecycle(denon_avr_application::Lifecycle::Connected {
                generation: 1,
            }),
        })));
        assert_eq!(gui.lifecycle, denon_avr_application::Lifecycle::NoReceiver);
    }

    #[tokio::test]
    async fn disconnected_lifecycle_invalidates_visible_main_zone_status() {
        let bridge = ControllerBridge::new(AvrSessionFactory::default());
        let mut gui = Gui::new(bridge);
        gui.request_id = 1;
        gui.lifecycle = denon_avr_application::Lifecycle::Connected { generation: 1 };
        gui.snapshot.set_value(
            denon_avr_domain::MainZoneValue::Power(denon_avr_domain::PowerState::On),
            denon_avr_domain::StateAuthority::Authoritative,
        );

        let _ = gui.update(Message::Bridge(Box::new(BridgeEvent {
            request_id: 1,
            generation: 1,
            event: ReceiverEvent::Lifecycle(denon_avr_application::Lifecycle::Disconnected),
        })));

        assert_eq!(
            gui.lifecycle,
            denon_avr_application::Lifecycle::Disconnected
        );
        assert!(gui.snapshot.power.value().is_none());
        assert_eq!(
            gui.snapshot.freshness,
            denon_avr_domain::Freshness::Invalidated
        );
    }

    #[test]
    fn quick_select_bar_labels_slot_and_reported_name() {
        let slot = QuickSelectSlot::new(1).unwrap();
        let mut snapshot = QuickSelectSnapshot::default();
        snapshot.set(denon_avr_domain::QuickSelectPreset {
            slot,
            name: Some(denon_avr_domain::QuickSelectName::new("Cinema").unwrap()),
            available: true,
            summary: Default::default(),
        });
        assert_eq!(quick_select_slot_label(&snapshot, slot), "1 Cinema");
        assert_eq!(
            quick_select_slot_label(&QuickSelectSnapshot::default(), slot),
            "1"
        );
    }

    #[test]
    fn dashboard_omits_unknown_eq_summary() {
        assert_eq!(eq_summary_if_reported(&EqStatus::default()), None);
        let status = EqStatus {
            dynamic_eq: denon_avr_domain::EqState::Off,
            ..EqStatus::default()
        };
        assert!(eq_summary_if_reported(&status).is_some());
    }

    #[test]
    fn volume_slider_is_capped_at_sixty() {
        assert_eq!(
            volume_level_for_slider(MAX_VOLUME)
                .unwrap()
                .to_native_code(),
            600
        );
        assert_eq!(
            volume_level_for_slider(60.5).unwrap_err(),
            "Volume is limited to 60.0."
        );
    }

    #[test]
    fn volume_slider_uses_the_receiver_absolute_volume_scale() {
        let mut snapshot = MainZoneSnapshot::default();
        snapshot.set_value(
            denon_avr_domain::MainZoneValue::Volume(denon_avr_domain::Volume::from_parts(
                "245", -555,
            )),
            denon_avr_domain::StateAuthority::Authoritative,
        );

        assert_eq!(slider_volume(&snapshot), Some(24.5));
        assert_eq!(volume_level_for_slider(25.0).unwrap().to_native_code(), 250);
    }

    #[tokio::test]
    async fn volume_step_caps_at_sixty_and_explains_the_limit() {
        let bridge = ControllerBridge::new(AvrSessionFactory::default());
        let mut gui = Gui::new(bridge);
        gui.volume_slider = 55.0;

        let _ = gui.update(Message::AdjustVolume(10.0));

        assert_eq!(gui.volume_slider, MAX_VOLUME);
        assert_eq!(gui.volume_value, Some(MAX_VOLUME));
        assert_eq!(gui.announcement, "Volume is limited to 60.0.");

        let current_request = gui.volume_value_request_id;
        let _ = gui.update(Message::HideVolumeValue(current_request.saturating_sub(1)));
        assert_eq!(gui.volume_value, Some(MAX_VOLUME));
        let _ = gui.update(Message::HideVolumeValue(current_request));
        assert_eq!(gui.volume_value, None);
    }

    #[tokio::test]
    async fn source_status_opens_and_selects_from_the_picker() {
        let bridge = ControllerBridge::new(AvrSessionFactory::default());
        let mut gui = Gui::new(bridge);
        gui.selection = Some(ReceiverSelection::ExplicitHost(
            denon_avr_domain::ReceiverIdentity {
                host: "receiver.local".into(),
                model: Some("Denon AVC-X3800H".into()),
                friendly_name: None,
            },
        ));

        let _ = gui.update(Message::OpenSourcePicker);
        assert!(gui.source_picker_open);
        let _ = gui.update(Message::SelectInput("TV AUDIO".into()));
        assert!(!gui.source_picker_open);
    }

    #[tokio::test]
    async fn confirmed_status_is_logged_once_per_connection_generation() {
        let bridge = ControllerBridge::new(AvrSessionFactory::default());
        let mut gui = Gui::new(bridge);
        gui.selection = Some(ReceiverSelection::ExplicitHost(
            denon_avr_domain::ReceiverIdentity::ad_hoc("receiver.local"),
        ));
        gui.request_id = 1;
        gui.generation = 1;
        let mut snapshot = MainZoneSnapshot::default();
        snapshot.set_value(
            denon_avr_domain::MainZoneValue::Power(denon_avr_domain::PowerState::On),
            denon_avr_domain::StateAuthority::Authoritative,
        );

        for _ in 0..2 {
            let _ = gui.update(Message::Bridge(Box::new(BridgeEvent {
                request_id: 1,
                generation: 0,
                event: ReceiverEvent::Snapshot(snapshot.clone()),
            })));
        }

        assert_eq!(
            gui.messages
                .iter()
                .filter(|message| message.contains("status confirmed"))
                .count(),
            1
        );
    }

    #[test]
    fn unavailable_power_offers_a_recovery_action() {
        let (title, detail, action) = power_recovery(
            &denon_avr_application::Lifecycle::Disconnected,
            &MainZoneSnapshot::default(),
        );

        assert_eq!(title, "RECEIVER UNAVAILABLE");
        assert!(detail.contains("not queried"));
        assert!(matches!(action, Some(("Retry Status", Message::Refresh))));
    }

    #[tokio::test]
    async fn gui_uses_explicit_phase8_validation_for_known_models() {
        let bridge = ControllerBridge::new_with_config(
            AvrSessionFactory::default(),
            denon_avr_application::ControllerConfig {
                validated_phase8: Some(denon_avr_domain::ValidatedPhase8Capabilities {
                    quick_select_recall: true,
                    eq_status: true,
                }),
                ..denon_avr_application::ControllerConfig::default()
            },
        );
        let mut gui = Gui::new(bridge);
        gui.selection = Some(ReceiverSelection::ExplicitHost(
            denon_avr_domain::ReceiverIdentity {
                host: "receiver.local".into(),
                model: Some("AVR-X3800H".into()),
                friendly_name: None,
            },
        ));
        let capabilities = gui.selected_capabilities();
        assert!(capabilities.quick_select_recall);
        assert!(capabilities.eq_status);
    }

    #[test]
    fn recall_feedback_is_actionable_and_main_zone_specific() {
        let slot = QuickSelectSlot::new(3).unwrap();
        assert_eq!(
            feedback::quick_select_recall_message(
                &denon_avr_domain::QuickSelectRecallOutcome::Confirmed { slot }
            ),
            "Main Zone Quick Select 3 recalled and confirmed."
        );
        assert!(feedback::quick_select_recall_message(
            &denon_avr_domain::QuickSelectRecallOutcome::Conflict {
                expected: 1,
                current: 2,
            }
        )
        .contains("retry"));
    }

    #[tokio::test]
    async fn message_panel_is_bounded_and_preserves_chronological_order() {
        let bridge = ControllerBridge::new(AvrSessionFactory::default());
        let mut gui = Gui::new(bridge);
        gui.record_message("same message");
        gui.record_message("same message");
        assert_eq!(gui.messages.len(), 1);
        for index in 0..=design::MAX_SESSION_MESSAGES {
            gui.record_message(format!("message {index}"));
        }
        assert_eq!(gui.messages.len(), design::MAX_SESSION_MESSAGES);
        assert_eq!(gui.messages.front().map(String::as_str), Some("message 1"));
        assert_eq!(gui.messages.back().map(String::as_str), Some("message 100"));
        let _ = gui.update(Message::ToggleMessages);
        assert!(gui.messages_collapsed);
        let _ = gui.update(Message::ClearMessages);
        assert!(gui.messages.is_empty());
    }

    #[tokio::test]
    async fn discovered_receiver_selection_opens_the_console_and_records_context() {
        let bridge = ControllerBridge::new(AvrSessionFactory::default());
        let mut gui = Gui::new(bridge);
        gui.route = Route::Receivers;
        let receiver = denon_avr_domain::DiscoveredReceiver {
            address: denon_avr_domain::ReceiverEndpoint {
                host: "192.168.0.8".into(),
                port: 23,
            },
            location: None,
            server: None,
            model: Some("Denon AVC-X3800H".into()),
            search_target: None,
            unique_service_name: None,
        };

        let _ = gui.update(Message::Select(ReceiverSelection::Discovered(receiver)));

        assert_eq!(gui.route, Route::Dashboard);
        assert_eq!(
            gui.selection.as_ref().map(|s| s.identity().host),
            Some("192.168.0.8".into())
        );
        assert!(gui
            .messages
            .back()
            .is_some_and(|message| message.contains("AVC-X3800H")));
    }

    #[test]
    fn speaker_state_keeps_unobserved_channels_unknown() {
        assert_eq!(channel_state(None, "FL"), "UNKNOWN");
        assert_eq!(channel_state(Some("FL C FR SL SR SBL SBR LFE"), "FL"), "ON");
        assert_eq!(
            channel_state(Some("FL C FR SL SR SBL SBR LFE"), "TML"),
            "OFF"
        );
    }

    #[test]
    fn discovered_receiver_becomes_the_persisted_current_receiver() {
        let receiver = denon_avr_domain::DiscoveredReceiver {
            address: denon_avr_domain::ReceiverEndpoint {
                host: "192.168.0.8".into(),
                port: 23,
            },
            location: None,
            server: None,
            model: Some("Denon AVC-X3800H".into()),
            search_target: None,
            unique_service_name: None,
        };
        let (config, selection) = configuration_for_discovered(&receiver);

        assert_eq!(config.current.as_deref(), Some("Denon AVC-X3800H"));
        assert_eq!(
            config.current().map(|(_, identity)| identity.host.as_str()),
            Some("192.168.0.8")
        );
        assert_eq!(
            selection.identity().friendly_name.as_deref(),
            Some("Denon AVC-X3800H")
        );
    }
}
