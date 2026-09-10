//! Iced presentation layer for the desktop client.
//!
//! This module deliberately contains no AVR protocol knowledge.  All receiver
//! policy, confirmation, retry, and lifecycle decisions remain in the typed
//! application controller.

use denon_avr_application::ports::{
    AsyncConfigRepository, AsyncReceiverDiscovery, BoxFuture, OperationError,
};
use denon_avr_application::{ReceiverEvent, ReceiverSelection};
use denon_avr_domain::{
    ChannelSlot, ChannelSlotState, ConfiguredReceivers, EqStatus, FieldStatus, Input,
    MainZoneControl, MainZoneSnapshot, MainZoneValue, Model, ModelCapabilities, MuteState,
    PowerState, QuickSelectEqCapabilities, QuickSelectSlot, QuickSelectSnapshot, ReceiverIdentity,
    SourceCatalog, SourceCatalogCapabilities, SourceVisibility, StateAuthority, SurroundMode,
    Volume, Zone2Snapshot,
};
use iced::widget::{column, container, row, scrollable, space, stack, text, text_input};
use iced::{Element, Length, Subscription, Task};
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

mod bridge;
mod capture;
pub mod components;
mod dashboard;
pub mod design;
mod feedback;
mod messages;
mod receiver_setup;
mod settings_diagnostics;
mod views;

use bridge::BridgeCommand;
pub use bridge::{BridgeEvent, ControllerBridge, GuiServices};
use dashboard::*;
pub use messages::Message;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PowerPopup {
    MainZone,
    Zone2,
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

pub struct Gui {
    pub route: Route,
    pub window_class: WindowClass,
    pub configured: ConfiguredReceivers,
    pub discovered: Vec<denon_avr_domain::DiscoveredReceiver>,
    pub selection: Option<ReceiverSelection>,
    pub lifecycle: denon_avr_application::Lifecycle,
    pub snapshot: MainZoneSnapshot,
    pub zone2: Zone2Snapshot,
    pub volume_slider: f32,
    volume_value: Option<f32>,
    volume_value_request_id: u64,
    source_picker_open: bool,
    power_popup: Option<PowerPopup>,
    pub announcement: String,
    pub address: String,
    pub name: String,
    pub request_id: u64,
    pub generation: u64,
    pub quick_select: QuickSelectSnapshot,
    pub eq_status: EqStatus,
    pub source_catalog: SourceCatalog,
    volume_command_pending: bool,
    /// Request identity is separate from the GUI-wide freshness counter:
    /// background catalog/status reads may legitimately advance that counter
    /// while a volume confirmation is still in flight.
    volume_command_request_id: Option<u64>,
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
    launch_ready: bool,
    launch_frame: u8,
    validated_quick_select_eq: Option<QuickSelectEqCapabilities>,
    validated_source_catalog: Option<SourceCatalogCapabilities>,
    bridge: ControllerBridge,
    discovery: Arc<dyn AsyncReceiverDiscovery>,
    configuration: Arc<dyn AsyncConfigRepository>,
}

impl Gui {
    pub fn new(bridge: ControllerBridge) -> Self {
        let validated_quick_select_eq = bridge.validated_quick_select_eq;
        let validated_source_catalog = bridge.validated_source_catalog;
        Self {
            route: Route::Dashboard,
            window_class: WindowClass::Wide,
            configured: ConfiguredReceivers::default(),
            discovered: Vec::new(),
            selection: None,
            lifecycle: denon_avr_application::Lifecycle::NoReceiver,
            snapshot: MainZoneSnapshot::default(),
            zone2: Zone2Snapshot::default(),
            volume_slider: 0.0,
            volume_value: None,
            volume_value_request_id: 0,
            source_picker_open: false,
            power_popup: None,
            announcement: "Loading configuration…".into(),
            address: String::new(),
            name: String::new(),
            request_id: 0,
            generation: 0,
            quick_select: QuickSelectSnapshot::default(),
            eq_status: EqStatus::default(),
            source_catalog: SourceCatalog::default(),
            volume_command_pending: false,
            volume_command_request_id: None,
            status_confirmed_generation: None,
            messages: VecDeque::new(),
            messages_collapsed: false,
            text_scale: 100,
            motion_preference: MotionPreference::Normal,
            contrast_preference: ContrastPreference::Normal,
            capture_directory: std::env::var_os("DENON_AVR_CAPTURE_DIR").map(PathBuf::from),
            capture_scenario: None,
            launch_ready: false,
            launch_frame: 0,
            validated_quick_select_eq,
            validated_source_catalog,
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
            MainZoneValue::Volume(Volume::from_parts("35", -450)),
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
        self.zone2 = Zone2Snapshot::default();
        self.zone2
            .set_power(PowerState::Standby, StateAuthority::Authoritative);
        self.volume_slider = -45.0;
        self.source_picker_open = false;
        self.power_popup = None;
        self.launch_ready = true;
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
            .with_validated_quick_select_eq(self.validated_quick_select_eq.unwrap_or_default())
            .with_validated_source_catalog(self.validated_source_catalog.unwrap_or_default())
    }

    fn invalidate_quick_select_eq(&mut self) {
        self.quick_select.invalidate();
        self.eq_status.invalidate();
        self.quick_select.generation = self.generation;
        self.eq_status.generation = self.generation;
        self.source_catalog.invalidate(self.generation);
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
                    // Configuration loading is complete and there is no
                    // receiver to connect. Show the receiver setup UI rather
                    // than leaving the startup animation active forever.
                    self.launch_ready = true;
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
                // A configuration error must still leave the app usable so
                // the user can add a receiver manually.
                self.launch_ready = true;
                self.announce(format!("Configuration unavailable: {error}"));
                Task::none()
            }
            Message::CommandFinished(Err(error)) => {
                self.volume_command_pending = false;
                self.volume_command_request_id = None;
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
                if self.capture_scenario.is_some() {
                    close_latest_window()
                } else {
                    Task::none()
                }
            }
            Message::ScreenshotWritten(Err(error)) => {
                self.announce(format!("Could not write native PNG capture: {error}"));
                Task::none()
            }
            Message::Keyboard(event) => {
                #[cfg(target_os = "macos")]
                if is_close_window_shortcut(&event) {
                    return close_latest_window();
                }
                match tab_direction(&event) {
                    Some(TabDirection::Forward) => iced::widget::operation::focus_next(),
                    Some(TabDirection::Backward) => iced::widget::operation::focus_previous(),
                    None => Task::none(),
                }
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
                self.zone2.invalidate();
                self.invalidate_quick_select_eq();
                self.volume_command_pending = false;
                self.volume_command_request_id = None;
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
                let event_request_id = event.request_id;
                // A control confirmation for the in-flight volume command is
                // terminal even if a newer, unrelated read request has since
                // advanced the global request counter.
                if matches!(&event.event, ReceiverEvent::Control(_))
                    && self.volume_command_request_id == Some(event_request_id)
                {
                    self.volume_command_pending = false;
                    self.volume_command_request_id = None;
                }
                // Supplemental receiver reads run after the core status reply.
                // A core snapshot can immediately start a newer source-catalog
                // request, while the Quick Select-name event from the earlier
                // refresh is still in transit. Those observations are tagged
                // with their connection generation and must not be discarded
                // merely because an unrelated GUI request has a newer ID.
                let supplemental_state = matches!(
                    &event.event,
                    ReceiverEvent::QuickSelect(_)
                        | ReceiverEvent::QuickSelectNames(_)
                        | ReceiverEvent::EqStatus(_)
                        | ReceiverEvent::SourceCatalog(_)
                );
                if (event.request_id < self.request_id && !supplemental_state)
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
                            self.invalidate_quick_select_eq();
                        }
                        if matches!(
                            &lifecycle,
                            denon_avr_application::Lifecycle::Reconnecting { .. }
                                | denon_avr_application::Lifecycle::Disconnected
                        ) {
                            self.snapshot.invalidate();
                            self.zone2.invalidate();
                            self.volume_command_pending = false;
                            self.volume_command_request_id = None;
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
                        self.complete_launch_if_ready();
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
                        self.complete_launch_if_ready();
                        if self.launch_ready
                            && self.selected_capabilities().source_catalog_read
                            && matches!(
                                self.source_catalog.freshness,
                                denon_avr_domain::Freshness::Unknown
                                    | denon_avr_domain::Freshness::Invalidated
                            )
                            && self.snapshot.power.value().is_some()
                        {
                            let id = self.next_request();
                            self.announce("Refreshing source list…");
                            return self.command(BridgeCommand::RefreshSourceCatalog(id));
                        }
                    }
                    ReceiverEvent::Zone2Snapshot(snapshot) => self.zone2 = snapshot,
                    ReceiverEvent::QuickSelect(snapshot) => self.quick_select = *snapshot,
                    ReceiverEvent::EqStatus(status) => self.eq_status = *status,
                    ReceiverEvent::SourceCatalog(observation) => {
                        if observation.catalog.generation == self.generation || self.generation == 0
                        {
                            self.source_catalog = observation.catalog;
                            self.announce("Source list refreshed.");
                        }
                    }
                    ReceiverEvent::QuickSelectNames(observation) => {
                        if observation.generation == self.generation || self.generation == 0 {
                            self.announce("Quick Select names refreshed.");
                        }
                    }
                    ReceiverEvent::QuickSelectRecall(outcome) => {
                        self.announce(feedback::quick_select_recall_message(&outcome))
                    }
                    ReceiverEvent::Control(result) => {
                        if self.volume_command_request_id == Some(event_request_id) {
                            self.volume_command_pending = false;
                            self.volume_command_request_id = None;
                        }
                        self.announce(feedback::control_message(&result))
                    }
                    ReceiverEvent::FieldError { field, error } => {
                        self.announce(format!("{} unavailable: {}", field.name(), error.message))
                    }
                    ReceiverEvent::Diagnostic(diagnostic) => {
                        self.volume_command_pending = false;
                        self.volume_command_request_id = None;
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
            Message::ToggleMainZonePower => main_zone_power_control(self.snapshot.power.value())
                .map_or_else(Task::none, |control| self.control(control)),
            Message::ToggleZone2Power => {
                let power = match self.zone2.power.value() {
                    Some(PowerState::On) => PowerState::Standby,
                    Some(PowerState::Standby) => PowerState::On,
                    None => return Task::none(),
                };
                let id = self.next_request();
                self.command(BridgeCommand::ControlZone2(id, power))
            }
            Message::OpenMainZonePowerPopup => {
                self.power_popup = self
                    .snapshot
                    .power
                    .value()
                    .is_some()
                    .then_some(PowerPopup::MainZone);
                Task::none()
            }
            Message::OpenZone2PowerPopup => {
                self.power_popup = self
                    .zone2
                    .power
                    .value()
                    .is_some()
                    .then_some(PowerPopup::Zone2);
                Task::none()
            }
            Message::ClosePowerPopup => {
                self.power_popup = None;
                Task::none()
            }
            Message::SetMainZonePower(power) => {
                self.power_popup = None;
                self.control(MainZoneControl::Power(power))
            }
            Message::SetZone2Power(power) => {
                self.power_popup = None;
                let id = self.next_request();
                self.command(BridgeCommand::ControlZone2(id, power))
            }
            Message::Mute => self.control(MainZoneControl::Mute(denon_avr_domain::MuteState::On)),
            Message::Unmute => {
                self.control(MainZoneControl::Mute(denon_avr_domain::MuteState::Off))
            }
            Message::VolumeChanged(value) => {
                if self.volume_is_interactive() {
                    self.volume_slider = value.clamp(MIN_VOLUME_DB, MAX_VOLUME_DB);
                    self.show_volume_value()
                } else {
                    Task::none()
                }
            }
            Message::CommitVolume => {
                if !self.volume_is_interactive() {
                    return Task::none();
                }
                match volume_level_for_slider(self.volume_slider) {
                    Ok(level) => self.submit_volume(level),
                    Err(error) => {
                        self.announce(error);
                        Task::none()
                    }
                }
            }
            Message::AdjustVolume(delta) => self.adjust_volume(delta),
            Message::HideVolumeValue(request_id) => {
                if request_id == self.volume_value_request_id {
                    self.volume_value = None;
                }
                Task::none()
            }
            Message::LaunchTick => {
                if !self.launch_ready {
                    self.launch_frame = self.launch_frame.wrapping_add(1);
                }
                Task::none()
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
            Message::ToggleSoundModeFavorite(mode) => {
                let mut config = self.configured.clone();
                let favorite = match config.toggle_current_sound_mode_favorite(&mode) {
                    Ok(favorite) => favorite,
                    Err(error) => {
                        self.announce(error);
                        return Task::none();
                    }
                };
                let configuration = Arc::clone(&self.configuration);
                self.announce(format!(
                    "{} {} favorite…",
                    if favorite { "Saving" } else { "Removing" },
                    mode
                ));
                Task::perform(
                    async move {
                        configuration
                            .save(&config)
                            .await
                            .map(|_| config)
                            .map_err(|error| error.to_string())
                    },
                    Message::SoundModeFavoritesSaved,
                )
            }
            Message::SoundModeFavoritesSaved(Ok(config)) => {
                self.configured = config;
                self.announce("Sound mode favorites saved.");
                Task::none()
            }
            Message::SoundModeFavoritesSaved(Err(error)) => {
                self.announce(format!("Could not save sound mode favorites: {error}"));
                Task::none()
            }
            Message::OpenSourcePicker => {
                self.source_picker_open = true;
                if self.selected_capabilities().source_catalog_read
                    && matches!(
                        self.source_catalog.freshness,
                        denon_avr_domain::Freshness::Unknown
                            | denon_avr_domain::Freshness::Invalidated
                    )
                {
                    let id = self.next_request();
                    self.announce("Refreshing source list…");
                    self.command(BridgeCommand::RefreshSourceCatalog(id))
                } else {
                    Task::none()
                }
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
            Message::RefreshQuickSelectEq => {
                let id = self.next_request();
                self.announce("Refreshing EQ status…");
                self.command(BridgeCommand::RefreshQuickSelectEq(id))
            }
            Message::RefreshSourceCatalog => {
                let id = self.next_request();
                self.announce("Refreshing source list…");
                self.command(BridgeCommand::RefreshSourceCatalog(id))
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
                    ..ConfiguredReceivers::default()
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
        if !self.volume_is_interactive() {
            return Task::none();
        }
        let requested = self.volume_slider + delta;
        let target = requested.clamp(MIN_VOLUME_DB, MAX_VOLUME_DB);
        let changed = target != self.volume_slider;
        self.volume_slider = target;
        let value_task = changed.then(|| self.show_volume_value());
        let task = if changed {
            match volume_level_for_slider(target) {
                Ok(level) => self.submit_volume(level),
                Err(error) => {
                    self.announce(error);
                    Task::none()
                }
            }
        } else {
            Task::none()
        };
        match value_task {
            Some(value_task) => Task::batch([task, value_task]),
            None => task,
        }
    }

    fn volume_is_interactive(&self) -> bool {
        self.selection.is_some()
            && self.snapshot.volume.value().is_some()
            && !self.volume_command_pending
    }

    fn submit_volume(&mut self, level: denon_avr_domain::VolumeLevel) -> Task<Message> {
        self.volume_command_pending = true;
        let id = self.next_request();
        self.volume_command_request_id = Some(id);
        self.announce("Command pending; waiting for receiver confirmation…");
        self.command(BridgeCommand::Control(
            id,
            MainZoneControl::Volume(level),
            Some(self.snapshot.resource_version()),
        ))
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

    fn complete_launch_if_ready(&mut self) {
        let saved_receiver = matches!(self.selection, Some(ReceiverSelection::Saved { .. }));
        let connected = matches!(
            self.lifecycle,
            denon_avr_application::Lifecycle::Connected { .. }
        );
        // Connection establishment is sufficient to open the dashboard. Core
        // status and HTTP/Quick Select reads are supplemental: a receiver can
        // accept the session while an individual query is delayed,
        // unsupported, or still timing out. Keeping those reads out of the
        // launch gate prevents the saved-receiver screen from waiting forever.
        self.launch_ready |= saved_receiver && connected;
    }

    pub fn view(&self) -> Element<'_, Message> {
        if !self.launch_ready {
            return launch_waiting_animation(self.launch_frame);
        }
        let rail = container(
            column![
                text("MAIN ZONE")
                    .width(Length::Fill)
                    .align_x(iced::Alignment::Center)
                    .size(12)
                    .color(design::MUTED),
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
        let dashboard_popup = (self.route == Route::Dashboard)
            .then_some(self.power_popup)
            .flatten();
        let source_picker_open = self.route == Route::Dashboard && self.source_picker_open;
        let body: Element<'_, Message> = match (source_picker_open, dashboard_popup) {
            // Keep overlays separate from the dashboard column so opening one
            // never reflows cards or controls below the source line.
            (true, None) => stack![
                base_body,
                source_picker_overlay(self.selected_capabilities(), self.source_catalog.clone()),
            ]
            .into(),
            (false, Some(popup)) => stack![
                base_body,
                power_popup_overlay(popup, self.snapshot.power.value(), self.zone2.power.value(),),
            ]
            .into(),
            (true, Some(popup)) => stack![
                base_body,
                source_picker_overlay(self.selected_capabilities(), self.source_catalog.clone()),
                power_popup_overlay(popup, self.snapshot.power.value(), self.zone2.power.value(),),
            ]
            .into(),
            (false, None) => base_body,
        };
        let messages: Element<'_, Message> = if self.messages_collapsed {
            container(
                components::message_icon_action("⌄", Message::ToggleMessages)
                    .width(Length::Shrink)
                    .padding([2, 6]),
            )
            .width(Length::Fill)
            .style(design::panel)
            .into()
        } else {
            let items = self
                .messages
                .iter()
                .rev()
                .take(3)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .fold(column![].spacing(5), |column, message| {
                    column.push(text(message).size(12))
                });
            container(
                row![
                    scrollable(items)
                        .width(Length::Fill)
                        .height(Length::Fixed(54.0))
                        .anchor_bottom(),
                    components::message_icon_action("⌫", Message::ClearMessages),
                    components::message_icon_action("⌃", Message::ToggleMessages)
                ]
                .spacing(6)
                .align_y(iced::Alignment::Center),
            )
            .width(Length::Fill)
            .padding(6)
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
        ..ConfiguredReceivers::default()
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

fn close_latest_window() -> Task<Message> {
    iced::window::latest().then(|id| match id {
        Some(id) => iced::window::close(id),
        None => Task::none(),
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

fn launch_waiting_animation(frame: u8) -> Element<'static, Message> {
    const FRAMES: [&str; 4] = ["◐", "◓", "◑", "◒"];
    container(
        text(FRAMES[usize::from(frame) % FRAMES.len()])
            .size(48)
            .color(design::ACCENT),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center(Length::Fill)
    .into()
}

pub fn view(gui: &Gui) -> Element<'_, Message> {
    gui.view()
}
pub fn subscription(gui: &Gui) -> Subscription<Message> {
    Subscription::batch([
        gui.bridge
            .subscription()
            .map(|event| Message::Bridge(Box::new(event))),
        iced::keyboard::listen().map(Message::Keyboard),
        if gui.launch_ready {
            Subscription::none()
        } else {
            iced::time::every(Duration::from_millis(140)).map(|_| Message::LaunchTick)
        },
    ])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabDirection {
    Forward,
    Backward,
}

fn tab_direction(event: &iced::keyboard::Event) -> Option<TabDirection> {
    match event {
        iced::keyboard::Event::KeyPressed { key, modifiers, .. }
            if *key == iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab) =>
        {
            Some(if modifiers.shift() {
                TabDirection::Backward
            } else {
                TabDirection::Forward
            })
        }
        _ => None,
    }
}

/// Command-W is the standard macOS command for closing the active window.
/// Use the physical key as a Latin fallback so it also works with non-Latin
/// keyboard layouts.
#[cfg(target_os = "macos")]
fn is_close_window_shortcut(event: &iced::keyboard::Event) -> bool {
    matches!(
        event,
        iced::keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            ..
        } if modifiers.command()
            && matches!(key.to_latin(*physical_key), Some('w' | 'W'))
    )
}

struct NoopDiscovery;
impl AsyncReceiverDiscovery for NoopDiscovery {
    fn discover(
        &self,
        _timeout: Duration,
    ) -> BoxFuture<'_, Result<Vec<denon_avr_domain::DiscoveredReceiver>, OperationError>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

struct NoopConfiguration;
impl AsyncConfigRepository for NoopConfiguration {
    fn load(&self) -> BoxFuture<'_, Result<ConfiguredReceivers, OperationError>> {
        Box::pin(async { Ok(ConfiguredReceivers::default()) })
    }

    fn save<'a>(
        &'a self,
        _config: &'a ConfiguredReceivers,
    ) -> BoxFuture<'a, Result<(), OperationError>> {
        Box::pin(async { Ok(()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use denon_avr_application::ports::{ReceiverSession, SessionFactory};
    use denon_avr_domain::ReceiverIdentity;

    #[derive(Clone, Default)]
    struct TestSessionFactory;

    impl SessionFactory for TestSessionFactory {
        fn connect(
            &self,
            _identity: ReceiverIdentity,
        ) -> BoxFuture<'_, Result<Box<dyn ReceiverSession>, OperationError>> {
            Box::pin(async {
                Err(OperationError::new(
                    denon_avr_application::ports::OperationErrorKind::Connection,
                    "test session",
                    "no receiver session is required by GUI reducer tests",
                ))
            })
        }
    }

    #[tokio::test]
    async fn accessibility_overrides_are_session_only_and_update_theme_state() {
        let bridge = ControllerBridge::new(TestSessionFactory);
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
    async fn power_popups_are_independent_and_only_open_for_known_state() {
        let bridge = ControllerBridge::new(TestSessionFactory);
        let mut gui = Gui::new(bridge);
        gui.snapshot.set_value(
            MainZoneValue::Power(PowerState::On),
            StateAuthority::Authoritative,
        );
        gui.zone2
            .set_power(PowerState::Standby, StateAuthority::Authoritative);

        let _ = gui.update(Message::OpenMainZonePowerPopup);
        assert_eq!(gui.power_popup, Some(PowerPopup::MainZone));
        let _ = gui.update(Message::OpenZone2PowerPopup);
        assert_eq!(gui.power_popup, Some(PowerPopup::Zone2));
        let _ = gui.update(Message::ClosePowerPopup);
        assert_eq!(gui.power_popup, None);
    }

    #[test]
    fn compact_source_picker_omits_aux_3_and_later() {
        assert!(source_is_picker_entry("AUX1"));
        assert!(source_is_picker_entry("AUX2"));
        assert!(!source_is_picker_entry("AUX3"));
        assert!(!source_is_picker_entry("AUX7"));
    }

    #[tokio::test]
    async fn launch_opens_receiver_setup_when_no_receiver_is_saved() {
        let bridge = ControllerBridge::new(TestSessionFactory);
        let mut gui = Gui::new(bridge);

        let _ = gui.update(Message::ConfigLoaded(Ok(ConfiguredReceivers::default())));

        assert!(gui.launch_ready);
        assert!(gui.selection.is_none());
        assert_eq!(gui.lifecycle, denon_avr_application::Lifecycle::NoReceiver);
    }

    #[tokio::test]
    async fn launch_opens_receiver_setup_when_configuration_fails() {
        let bridge = ControllerBridge::new(TestSessionFactory);
        let mut gui = Gui::new(bridge);

        let _ = gui.update(Message::ConfigLoaded(Err(
            "configuration unavailable".into()
        )));

        assert!(gui.launch_ready);
    }

    #[tokio::test]
    async fn launch_opens_after_saved_receiver_connection() {
        let bridge = ControllerBridge::new(TestSessionFactory);
        let mut gui = Gui::new(bridge);
        gui.selection = Some(ReceiverSelection::Saved {
            name: "Living room".into(),
            identity: ReceiverIdentity {
                host: "receiver.local".into(),
                model: Some("AVR-X3800H".into()),
                friendly_name: None,
            },
        });
        gui.lifecycle = denon_avr_application::Lifecycle::Connected { generation: 1 };

        gui.complete_launch_if_ready();
        assert!(gui.launch_ready);
    }

    #[tokio::test]
    async fn connected_lifecycle_event_opens_saved_receiver_without_status_snapshot() {
        let bridge = ControllerBridge::new(TestSessionFactory);
        let mut gui = Gui::new(bridge);
        gui.selection = Some(ReceiverSelection::Saved {
            name: "Living room".into(),
            identity: ReceiverIdentity {
                host: "receiver.local".into(),
                model: Some("AVR-X3800H".into()),
                friendly_name: None,
            },
        });

        let _ = gui.update(Message::Bridge(Box::new(BridgeEvent {
            request_id: 1,
            generation: 1,
            event: ReceiverEvent::Lifecycle(denon_avr_application::Lifecycle::Connected {
                generation: 1,
            }),
        })));

        assert!(gui.launch_ready);
    }

    #[tokio::test]
    async fn fixed_capture_scenarios_do_not_need_a_receiver_connection() {
        let bridge = ControllerBridge::new(TestSessionFactory);
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

    #[test]
    fn tab_keys_have_explicit_forward_and_backward_focus_directions() {
        let event = |modifiers| iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab),
            modified_key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab),
            physical_key: iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::Tab),
            location: iced::keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        };
        assert_eq!(
            tab_direction(&event(iced::keyboard::Modifiers::NONE)),
            Some(TabDirection::Forward)
        );
        assert_eq!(
            tab_direction(&event(iced::keyboard::Modifiers::SHIFT)),
            Some(TabDirection::Backward)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn command_w_is_the_mac_window_close_shortcut() {
        let event = iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Character("w".into()),
            modified_key: iced::keyboard::Key::Character("w".into()),
            physical_key: iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::KeyW),
            location: iced::keyboard::Location::Standard,
            modifiers: iced::keyboard::Modifiers::LOGO,
            text: Some("w".into()),
            repeat: false,
        };
        assert!(is_close_window_shortcut(&event));
    }
    #[tokio::test]
    async fn stale_bridge_results_are_ignored() {
        // State construction is kept independent of widgets, making ordering
        // and stale-result behavior deterministic in unit tests.
        let bridge = ControllerBridge::new(TestSessionFactory);
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
        let bridge = ControllerBridge::new(TestSessionFactory);
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
        assert_eq!(
            quick_select_slot_label(&snapshot, slot, &SourceCatalog::default()),
            "1 Cinema"
        );
        assert_eq!(
            quick_select_slot_label(
                &QuickSelectSnapshot::default(),
                slot,
                &SourceCatalog::default()
            ),
            "1"
        );
    }

    #[test]
    fn quick_select_slot_label_uses_receiver_source_name() {
        let slot = QuickSelectSlot::new(1).unwrap();
        let mut snapshot = QuickSelectSnapshot::default();
        snapshot.set(denon_avr_domain::QuickSelectPreset {
            slot,
            name: None,
            available: true,
            summary: denon_avr_domain::QuickSelectSummary {
                input: denon_avr_domain::Registered::Included(
                    denon_avr_domain::Input::new("GAME").unwrap(),
                ),
                ..Default::default()
            },
        });
        let catalog = SourceCatalog {
            entries: vec![denon_avr_domain::SourceEntry {
                id: denon_avr_domain::SourceId::new("GAME").unwrap(),
                display_name: Some("PlayStation 5".into()),
                visibility: SourceVisibility::Shown,
            }],
            ..SourceCatalog::default()
        };
        assert_eq!(
            quick_select_slot_label(&snapshot, slot, &catalog),
            "1 · PlayStation 5"
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
    fn volume_slider_spans_the_validated_receiver_decibel_range() {
        assert_eq!(
            volume_level_for_slider(MIN_VOLUME_DB)
                .unwrap()
                .to_native_code(),
            0
        );
        assert_eq!(volume_level_for_slider(0.0).unwrap().to_native_code(), 800);
        assert_eq!(
            volume_level_for_slider(MAX_VOLUME_DB)
                .unwrap()
                .to_native_code(),
            985
        );
        assert_eq!(
            volume_level_for_slider(19.0).unwrap_err(),
            "Volume must be between -80.0 and +18.5 dB."
        );
    }

    #[test]
    fn volume_slider_uses_the_receiver_decibel_scale() {
        let mut snapshot = MainZoneSnapshot::default();
        snapshot.set_value(
            denon_avr_domain::MainZoneValue::Volume(denon_avr_domain::Volume::from_parts(
                "245", -555,
            )),
            denon_avr_domain::StateAuthority::Authoritative,
        );

        assert_eq!(slider_volume(&snapshot), Some(-55.5));
        assert_eq!(
            volume_level_for_slider(-55.0).unwrap().to_native_code(),
            250
        );
    }

    #[test]
    fn power_toggle_only_produces_main_zone_commands() {
        assert_eq!(
            main_zone_power_control(Some(&PowerState::On)),
            Some(MainZoneControl::Power(PowerState::Standby))
        );
        assert_eq!(
            main_zone_power_control(Some(&PowerState::Standby)),
            Some(MainZoneControl::Power(PowerState::On))
        );
        assert_eq!(main_zone_power_control(None), None);
    }

    #[tokio::test]
    async fn volume_step_respects_the_receiver_ceiling_and_blocks_duplicates() {
        let bridge = ControllerBridge::new(TestSessionFactory);
        let mut gui = Gui::new(bridge);
        gui.selection = Some(ReceiverSelection::ExplicitHost(
            denon_avr_domain::ReceiverIdentity {
                host: "receiver.local".into(),
                model: Some("AVR-X3800H".into()),
                friendly_name: None,
            },
        ));
        gui.snapshot.set_value(
            MainZoneValue::Volume(Volume::from_parts("95", 150)),
            StateAuthority::Authoritative,
        );
        gui.volume_slider = 15.0;

        let _ = gui.update(Message::AdjustVolume(10.0));

        assert_eq!(gui.volume_slider, MAX_VOLUME_DB);
        assert_eq!(gui.volume_value, Some(MAX_VOLUME_DB));
        assert!(gui.volume_command_pending);

        let _ = gui.update(Message::AdjustVolume(-0.5));
        assert_eq!(gui.volume_slider, MAX_VOLUME_DB);

        let current_request = gui.volume_value_request_id;
        let _ = gui.update(Message::HideVolumeValue(current_request.saturating_sub(1)));
        assert_eq!(gui.volume_value, Some(MAX_VOLUME_DB));
        let _ = gui.update(Message::HideVolumeValue(current_request));
        assert_eq!(gui.volume_value, None);
    }

    #[tokio::test]
    async fn unavailable_volume_does_not_create_a_low_volume_command() {
        let bridge = ControllerBridge::new(TestSessionFactory);
        let mut gui = Gui::new(bridge);
        gui.volume_slider = MIN_VOLUME_DB;

        let _ = gui.update(Message::AdjustVolume(0.5));
        let _ = gui.update(Message::CommitVolume);

        assert_eq!(gui.volume_slider, MIN_VOLUME_DB);
        assert!(!gui.volume_command_pending);
    }

    #[tokio::test]
    async fn bridge_failure_reenables_volume_controls() {
        let bridge = ControllerBridge::new(TestSessionFactory);
        let mut gui = Gui::new(bridge);
        gui.volume_command_pending = true;

        let _ = gui.update(Message::Bridge(Box::new(BridgeEvent {
            request_id: 1,
            generation: 0,
            event: ReceiverEvent::Diagnostic(denon_avr_application::Diagnostic::Timeout {
                context: "setting volume".into(),
            }),
        })));

        assert!(!gui.volume_command_pending);
    }

    #[tokio::test]
    async fn stale_volume_confirmation_still_reenables_the_slider() {
        let bridge = ControllerBridge::new(TestSessionFactory);
        let mut gui = Gui::new(bridge);
        gui.volume_command_pending = true;
        gui.volume_command_request_id = Some(4);
        // Simulate an unrelated catalog/status request begun after the volume
        // command but before its receiver confirmation arrives.
        gui.request_id = 5;

        let _ = gui.update(Message::Bridge(Box::new(BridgeEvent {
            request_id: 4,
            generation: 0,
            event: ReceiverEvent::Control(denon_avr_application::ControlResult::Cancelled),
        })));

        assert!(!gui.volume_command_pending);
        assert_eq!(gui.volume_command_request_id, None);
    }

    #[tokio::test]
    async fn source_status_opens_and_selects_from_the_picker() {
        let bridge = ControllerBridge::new(TestSessionFactory);
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

    #[test]
    fn receiver_source_label_overrides_the_canonical_fallback() {
        let catalog = SourceCatalog {
            entries: vec![denon_avr_domain::SourceEntry {
                id: denon_avr_domain::SourceId::new("GAME").unwrap(),
                display_name: Some("PlayStation 5".into()),
                visibility: SourceVisibility::Shown,
            }],
            ..SourceCatalog::default()
        };
        assert_eq!(catalog_entry_label("GAME", &catalog), "PlayStation 5");
        assert_eq!(catalog_entry_label("TV AUDIO", &catalog), "TV Audio");
    }

    #[test]
    fn source_picker_excludes_receiver_hidden_entries() {
        let catalog = SourceCatalog {
            entries: vec![denon_avr_domain::SourceEntry {
                id: denon_avr_domain::SourceId::new("GAME").unwrap(),
                display_name: Some("Console".into()),
                visibility: SourceVisibility::Hidden,
            }],
            ..SourceCatalog::default()
        };
        assert!(!source_is_visible(&catalog, "GAME"));
        assert!(source_is_visible(&catalog, "TV AUDIO"));
    }

    #[tokio::test]
    async fn confirmed_status_is_logged_once_per_connection_generation() {
        let bridge = ControllerBridge::new(TestSessionFactory);
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
    async fn gui_uses_explicit_quick_select_eq_validation_for_known_models() {
        let bridge = ControllerBridge::new_with_config(
            TestSessionFactory,
            denon_avr_application::ControllerConfig {
                validated_quick_select_eq: Some(denon_avr_domain::QuickSelectEqCapabilities {
                    quick_select_recall: true,
                    quick_select_names: true,
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
        let bridge = ControllerBridge::new(TestSessionFactory);
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
        let bridge = ControllerBridge::new(TestSessionFactory);
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
