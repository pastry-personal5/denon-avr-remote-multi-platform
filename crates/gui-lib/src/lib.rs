//! Iced presentation layer for the desktop client.
//!
//! This module deliberately contains no AVR protocol knowledge. It observes and
//! controls the receiver only through the control-service port: it projects the
//! state it is given, and submits operations that the service decides on.

use denon_avr_application::{
    http_information as http_policy, quick_select as names_policy, source_catalog as catalog_policy,
};
use denon_avr_application::{OperationStatus, SharedOperatorControl};
use denon_avr_domain::{
    ChannelSlot, ChannelSlotState, ConfiguredReceivers, FieldBaseline, FieldStatus, Input,
    MainZoneControl, MainZoneSnapshot, MainZoneValue, Model, ModelCapabilities, MuteState,
    PowerState, QuickSelectSlot, QuickSelectSnapshot, ReceiverId, ReceiverIdentity, ReceiverIntent,
    ReceiverState, SoundModeCategory, SourceCatalog, SourceVisibility, StateAuthority,
    SurroundMode, Volume, Zone2Snapshot,
};
use iced::widget::{column, container, row, scrollable, space, stack, text, text_input};
use iced::{Element, Length, Subscription, Task};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

/// Save `config` through the port and hand it back.
async fn save_configuration(
    control: SharedOperatorControl,
    config: ConfiguredReceivers,
) -> Result<ConfiguredReceivers, String> {
    control
        .save_configuration(&config)
        .await
        .map(|_| config)
        .map_err(|error| error.to_string())
}

/// Save the sound mode favorites in `config`, leaving the rest of the stored
/// configuration as it is now on disk.
async fn save_favorites(
    control: SharedOperatorControl,
    config: ConfiguredReceivers,
) -> Result<ConfiguredReceivers, String> {
    let mut stored = control
        .configuration()
        .await
        .map_err(|error| error.to_string())?;
    stored.sound_mode_favorites = config.sound_mode_favorites.clone();
    save_configuration(control, stored).await?;
    Ok(config)
}

/// Add or replace one receiver in the saved configuration and make it current,
/// keeping every other receiver and every sound mode favorite. The file is read
/// first, so an edit made by hand since the last load is not overwritten.
async fn save_receiver(
    control: SharedOperatorControl,
    name: String,
    identity: ReceiverIdentity,
) -> Result<(ConfiguredReceivers, Selection), String> {
    let mut config = control
        .configuration()
        .await
        .map_err(|error| error.to_string())?;
    config.receivers.insert(name.clone(), identity.clone());
    config.current = Some(name.clone());
    let config = save_configuration(control, config).await?;
    Ok((config, Selection { name, identity }))
}

mod bridge;
mod capture;
pub mod components;
mod dashboard;
pub mod design;
mod feedback;
mod messages;
pub mod projection;
mod receiver_setup;
mod settings_diagnostics;
mod views;

use bridge::BridgeCommand;
pub use bridge::{BridgeEvent, ControlReport, GuiServices, PortBridge, PortEvent, ShutdownHook};
use dashboard::*;
pub use messages::Message;

/// A saved receiver the user has chosen. The name is the configuration entry
/// name, which is the receiver's id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub name: String,
    pub identity: ReceiverIdentity,
}

impl Selection {
    pub fn identity(&self) -> ReceiverIdentity {
        self.identity.clone()
    }

    pub fn receiver_id(&self) -> Result<ReceiverId, &'static str> {
        ReceiverId::new(self.name.as_str())
    }
}

/// Where the connection to the selected receiver stands, as the views show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lifecycle {
    NoReceiver,
    Selected,
    Connecting,
    Connected { generation: u64 },
    Reconnecting { generation: u64 },
    Disconnected,
}

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
    pub selection: Option<Selection>,
    pub lifecycle: Lifecycle,
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
    pub source_catalog: SourceCatalog,
    /// Local table filter only. The AVR reports a detailed MS mode, not a
    /// category, so this value never represents receiver-observed state.
    sound_mode_category_filter: Option<SoundModeCategory>,
    sound_mode_request_id: Option<u64>,
    sound_mode_save_in_flight: bool,
    pending_sound_mode_config: Option<ConfiguredReceivers>,
    volume_command_pending: bool,
    /// True after the receiver has reported a volume or the user has
    /// initialized an unknown volume once from the safe minimum.
    volume_baseline_initialized: bool,
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
    pub launch_ready: bool,
    launch_frame: u8,
    status_wait_ticks: u16,
    /// The newest receiver state the views were built from. It is what a
    /// control's guard is captured against.
    state: Option<ReceiverState>,
    /// The `Select` request the displayed receiver was chosen with. Events from
    /// an earlier one belong to a receiver the user has left.
    subscription: u64,
    http_read_in_flight: bool,
    http_read_again: bool,
    catalog_read_in_flight: bool,
    names_read_in_flight: bool,
    /// The connection generation the source catalog was last requested for on
    /// its own, so a receiver that cannot answer is asked once per connection.
    catalog_auto_generation: Option<u64>,
    bridge: PortBridge,
    control: SharedOperatorControl,
}

impl Gui {
    pub fn new(services: GuiServices) -> Self {
        let control = std::sync::Arc::clone(&services.control);
        let bridge = PortBridge::new(services);
        Self {
            route: Route::Dashboard,
            window_class: WindowClass::Wide,
            configured: ConfiguredReceivers::default(),
            discovered: Vec::new(),
            selection: None,
            lifecycle: Lifecycle::NoReceiver,
            snapshot: MainZoneSnapshot::default(),
            zone2: Zone2Snapshot::default(),
            // Keep the visible thumb at the safe minimum until canonical
            // receiver volume arrives. The slider remains non-interactive
            // while the snapshot has no authoritative volume.
            volume_slider: MIN_VOLUME_DB,
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
            source_catalog: SourceCatalog::default(),
            sound_mode_category_filter: None,
            sound_mode_request_id: None,
            sound_mode_save_in_flight: false,
            pending_sound_mode_config: None,
            volume_command_pending: false,
            volume_baseline_initialized: false,
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
            status_wait_ticks: 0,
            state: None,
            subscription: 0,
            http_read_in_flight: false,
            http_read_again: false,
            catalog_read_in_flight: false,
            names_read_in_flight: false,
            catalog_auto_generation: None,
            bridge,
            control,
        }
    }

    /// The bridge to the port, for a headless driver that feeds its events back
    /// as messages.
    pub fn bridge(&self) -> &PortBridge {
        &self.bridge
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
        self.selection = Some(Selection {
            name: "Capture receiver".into(),
            identity: ReceiverIdentity {
                host: "capture.invalid".into(),
                model: Some("AVR-X3800H".into()),
                friendly_name: Some("Capture receiver".into()),
            },
        });
        self.generation = 1;
        self.lifecycle = Lifecycle::Connected { generation: 1 };
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
                self.lifecycle = Lifecycle::Disconnected;
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
    }

    fn status_waiting(&self) -> bool {
        const STATUS_WAIT_LIMIT_TICKS: u16 = 28; // 5 seconds at 180 ms
        fn not_queried<T>(field: &FieldStatus<T>) -> bool {
            matches!(field, FieldStatus::Unavailable(error) if error.message == "not queried")
        }
        self.status_wait_ticks < STATUS_WAIT_LIMIT_TICKS
            && [
                not_queried(&self.snapshot.power),
                not_queried(&self.snapshot.input),
                not_queried(&self.snapshot.volume),
                not_queried(&self.snapshot.mute),
                not_queried(&self.snapshot.surround_mode),
            ]
            .into_iter()
            .any(|waiting| waiting)
    }

    /// Forget what was read over HTTP for a connection that is gone: the Quick
    /// Select names, the source catalog, and the audio, video, and Audyssey
    /// information.
    fn invalidate_supplemental(&mut self) {
        self.quick_select.invalidate();
        self.quick_select.generation = self.generation;
        self.source_catalog.invalidate(self.generation);
        self.snapshot.invalidate_http_information(self.generation);
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ConfigLoaded(Ok(config)) => {
                self.configured = config;
                let message: String = if let Some((name, identity)) = self.configured.current() {
                    self.selection = Some(Selection {
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
                match self.selection.clone() {
                    Some(selection) => self.connect_to(&selection),
                    None => Task::none(),
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
                self.sound_mode_category_filter = None;
                self.sound_mode_request_id = None;
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
                self.volume_slider = MIN_VOLUME_DB;
                self.volume_baseline_initialized = false;
                self.zone2.invalidate();
                self.status_wait_ticks = 0;
                self.invalidate_supplemental();
                self.volume_command_pending = false;
                self.volume_command_request_id = None;
                self.sound_mode_category_filter = None;
                self.sound_mode_request_id = None;
                self.status_confirmed_generation = None;
                self.announce(format!(
                    "{} selected; connecting for confirmed status…",
                    selection_label(&selection)
                ));
                self.connect_to(&selection)
            }
            Message::Bridge(event) => self.on_bridge(*event),
            Message::Refresh => {
                let id = self.next_request();
                if self.lifecycle == Lifecycle::Disconnected {
                    self.lifecycle = Lifecycle::Connecting;
                }
                self.command(BridgeCommand::Refresh(id))
            }
            Message::HttpTick => {
                if self.state.is_some() && matches!(self.lifecycle, Lifecycle::Connected { .. }) {
                    self.request_http_information()
                } else {
                    Task::none()
                }
            }
            Message::ToggleMainZonePower => main_zone_power_control(self.snapshot.power.value())
                .map_or_else(Task::none, |control| self.control(control)),
            Message::ToggleZone2Power => {
                let power = match self.zone2.power.value() {
                    Some(PowerState::On) => PowerState::Standby,
                    Some(PowerState::Standby) => PowerState::On,
                    None => return Task::none(),
                };
                self.control_zone2(power)
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
                self.control_zone2(power)
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
            Message::SoundModeControlTimedOut(request_id) => {
                if self.sound_mode_request_id == Some(request_id) {
                    self.sound_mode_request_id = None;
                    self.announce("Sound Mode request timed out; controls re-enabled.");
                }
                Task::none()
            }
            Message::LaunchTick => {
                if !self.launch_ready
                    || self.status_waiting()
                    || self.sound_mode_request_id.is_some()
                {
                    self.launch_frame = self.launch_frame.wrapping_add(1);
                }
                self.status_wait_ticks = self.status_wait_ticks.saturating_add(1);
                Task::none()
            }
            Message::SelectSoundModeCategory(category) => {
                if self.sound_mode_request_id.is_some() {
                    return Task::none();
                }
                self.sound_mode_category_filter = Some(category);
                if category == SoundModeCategory::Pure {
                    Task::none()
                } else {
                    self.control_sound_mode(MainZoneControl::RecallSoundModeCategory(category))
                }
            }
            Message::SelectSurroundMode(category, value) => {
                if self.sound_mode_request_id.is_some() {
                    return Task::none();
                }
                match denon_avr_domain::SurroundMode::new(value) {
                    Ok(mode) => {
                        self.control_sound_mode(MainZoneControl::SelectSoundMode { category, mode })
                    }
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
                let control = self.control.clone();
                self.announce(format!(
                    "{} {} favorite…",
                    if favorite { "Saving" } else { "Removing" },
                    mode
                ));
                // Publish the new preference immediately so subsequent rapid
                // clicks build on it instead of cloning a stale committed
                // configuration. Persist writes are serialized below.
                self.configured = config.clone();
                if self.sound_mode_save_in_flight {
                    self.pending_sound_mode_config = Some(config);
                    return Task::none();
                }
                self.sound_mode_save_in_flight = true;
                Task::perform(
                    save_favorites(control, config),
                    Message::SoundModeFavoritesSaved,
                )
            }
            Message::SoundModeFavoritesSaved(Ok(config)) => {
                self.configured = config;
                self.announce("Sound mode favorites saved.");
                if let Some(next) = self.pending_sound_mode_config.take() {
                    self.configured = next.clone();
                    let control = self.control.clone();
                    return Task::perform(
                        save_favorites(control, next),
                        Message::SoundModeFavoritesSaved,
                    );
                }
                self.sound_mode_save_in_flight = false;
                Task::none()
            }
            Message::SoundModeFavoritesSaved(Err(error)) => {
                self.sound_mode_save_in_flight = false;
                self.pending_sound_mode_config = None;
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
                    self.announce("Refreshing source list…");
                    self.request_source_catalog()
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
            Message::RefreshSourceCatalog => {
                self.announce("Refreshing source list…");
                self.request_source_catalog()
            }
            Message::Discover => {
                self.announce("Searching for receivers…");
                let control = self.control.clone();
                Task::perform(
                    async move {
                        control
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
                let (name, identity) = receiver_entry_for_discovered(&receiver);
                let control = self.control.clone();
                self.announce(format!(
                    "Saving {} as the current receiver…",
                    identity_label(&identity)
                ));
                Task::perform(
                    save_receiver(control, name, identity),
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
                let control = self.control.clone();
                self.announce(format!(
                    "Saving {} as the current receiver…",
                    identity_label(&identity)
                ));
                Task::perform(save_receiver(control, name, identity), Message::ManualSaved)
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
            Message::Shutdown => self.command(BridgeCommand::Shutdown),
        }
    }

    fn control(&mut self, control: MainZoneControl) -> Task<Message> {
        let intent = match projection::intent_for(&control) {
            Ok(intent) => intent,
            Err(error) => {
                self.announce(error);
                return Task::none();
            }
        };
        let id = self.next_request();
        self.announce("Command pending; waiting for receiver confirmation…");
        self.submit(id, intent)
    }

    fn control_zone2(&mut self, power: PowerState) -> Task<Message> {
        let id = self.next_request();
        self.submit(id, projection::zone2_power_intent(power))
    }

    /// Send an operation to the receiver. A control that depends on what the
    /// user saw, which is every one but a power control, carries the value the
    /// target field showed, so a change since then is reported and not overwritten.
    fn submit(&mut self, id: u64, intent: ReceiverIntent) -> Task<Message> {
        let guard = self.guard_for(&intent);
        self.command(BridgeCommand::Control { id, intent, guard })
    }

    /// Power controls state an absolute target, so an unrelated change must not
    /// stop them.
    fn guard_for(&self, intent: &ReceiverIntent) -> Option<FieldBaseline> {
        match intent {
            ReceiverIntent::SystemPower(_)
            | ReceiverIntent::MainZonePower(_)
            | ReceiverIntent::Zone2Power(_) => None,
            _ => self
                .state
                .as_ref()
                .map(|state| FieldBaseline::capture(state, intent.field())),
        }
    }

    fn control_sound_mode(&mut self, control: MainZoneControl) -> Task<Message> {
        const SOUND_MODE_CONTROL_TIMEOUT: Duration = Duration::from_secs(3);
        let intent = match projection::intent_for(&control) {
            Ok(intent) => intent,
            Err(error) => {
                self.announce(error);
                return Task::none();
            }
        };
        let id = self.next_request();
        self.sound_mode_request_id = Some(id);
        self.announce("Command pending; waiting for receiver confirmation…");
        let command = self.submit(id, intent);
        let timeout = Task::perform(
            async move {
                tokio::time::sleep(SOUND_MODE_CONTROL_TIMEOUT).await;
                id
            },
            Message::SoundModeControlTimedOut,
        );
        Task::batch([command, timeout])
    }

    fn adjust_volume(&mut self, delta: f32) -> Task<Message> {
        if self.selection.is_none() || self.volume_command_pending {
            return Task::none();
        }
        // Until the AVR has supplied a canonical level, the first volume
        // button press establishes a safe minimum baseline. Apply later
        // button presses relative to that baseline, even if the receiver has
        // not yet returned a usable volume observation.
        if !self.volume_baseline_initialized {
            self.volume_slider = MIN_VOLUME_DB;
            self.volume_baseline_initialized = true;
            let minimum = volume_level_for_slider(MIN_VOLUME_DB)
                .expect("minimum volume is inside the validated receiver range");
            let value_task = self.show_volume_value();
            return Task::batch([self.submit_volume(minimum), value_task]);
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
        self.submit(id, ReceiverIntent::Volume(projection::master_volume(level)))
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

    /// Choose `selection` and ask the bridge to connect to it.
    fn connect_to(&mut self, selection: &Selection) -> Task<Message> {
        let receiver = match selection.receiver_id() {
            Ok(receiver) => receiver,
            Err(error) => {
                self.announce(format!(
                    "Cannot use the receiver name {:?}: {error}",
                    selection.name
                ));
                self.launch_ready = true;
                return Task::none();
            }
        };
        let id = self.next_request();
        self.subscription = id;
        self.state = None;
        self.generation = 0;
        self.lifecycle = Lifecycle::Selected;
        self.http_read_in_flight = false;
        self.http_read_again = false;
        self.catalog_read_in_flight = false;
        self.names_read_in_flight = false;
        self.catalog_auto_generation = None;
        self.command(BridgeCommand::Select(id, receiver))
    }

    fn reset_pending_controls(&mut self) {
        self.volume_command_pending = false;
        self.volume_command_request_id = None;
        self.sound_mode_category_filter = None;
        self.sound_mode_request_id = None;
    }

    /// The receiver stopped answering and the transport is reconnecting.
    fn connection_lost(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.lifecycle = Lifecycle::Reconnecting {
            generation: self.generation,
        };
        self.snapshot.invalidate();
        self.zone2.invalidate();
        self.invalidate_supplemental();
        self.status_wait_ticks = 0;
        self.status_confirmed_generation = None;
        self.reset_pending_controls();
    }

    /// Take in the newest receiver state: the connection lifecycle, the display
    /// model, and the reads that follow a connection or a change.
    fn apply_state(&mut self, state: ReceiverState) -> Task<Message> {
        let projected = projection::project(&state);
        let mut tasks: Vec<Task<Message>> = Vec::new();
        let was_connected = matches!(self.lifecycle, Lifecycle::Connected { .. });
        let was_reconnecting = matches!(self.lifecycle, Lifecycle::Reconnecting { .. });
        match state.epoch.map(|epoch| epoch.0) {
            None => {
                if !was_reconnecting {
                    self.connection_lost();
                }
            }
            Some(epoch) => {
                // The channel coalesces, so a disconnect and the reconnect after
                // it can arrive as one step to a higher epoch.
                let reconnected = was_reconnecting
                    || (was_connected && self.generation != 0 && epoch > self.generation);
                // The session reads every field again by itself when it
                // reconnects, so the new values arrive as ordinary state.
                if reconnected && was_connected {
                    self.connection_lost();
                }
                if !was_connected || reconnected {
                    self.generation = epoch;
                    self.lifecycle = Lifecycle::Connected { generation: epoch };
                    self.status_wait_ticks = 0;
                    tasks.push(self.request_quick_select_names());
                }
            }
        }

        let previous = std::mem::replace(&mut self.snapshot, projected.snapshot);
        self.snapshot.http_information = previous.http_information.clone();
        if self.snapshot.power.value() == Some(&PowerState::Standby) {
            self.snapshot.http_information.invalidate(0);
        }
        self.zone2 = projected.zone2;
        // Slide the thumb only when the receiver's level changed, not at every
        // state: the state also changes for reasons that say nothing about the
        // volume, and a drag in progress must not snap back.
        if let Some(volume) = slider_volume(&self.snapshot) {
            let changed = slider_volume(&previous) != Some(volume);
            if changed || !self.volume_baseline_initialized {
                self.volume_slider = volume;
                self.volume_baseline_initialized = true;
            }
        }
        if let Some(selection) = &self.selection {
            if self.snapshot.power.value().is_some()
                && self.status_confirmed_generation != Some(self.generation)
            {
                self.announce(format!(
                    "{} selected; connected; status confirmed.",
                    selection_label(selection)
                ));
                self.status_confirmed_generation = Some(self.generation);
            }
        }
        self.state = Some(state);
        self.complete_launch_if_ready();

        // What the receiver reports over HTTP follows its input, sound mode, and
        // power.
        let powered_on = self.snapshot.power.value() == Some(&PowerState::On)
            && previous.power.value() != Some(&PowerState::On);
        let input_changed = self.snapshot.input.value().is_some()
            && self.snapshot.input.value() != previous.input.value();
        let mode_changed = self.snapshot.surround_mode.value().is_some()
            && self.snapshot.surround_mode.value() != previous.surround_mode.value();
        if powered_on || input_changed || mode_changed {
            tasks.push(self.request_http_information());
        }
        if self.launch_ready
            && self.selected_capabilities().source_catalog_read
            && matches!(
                self.source_catalog.freshness,
                denon_avr_domain::Freshness::Unknown | denon_avr_domain::Freshness::Invalidated
            )
            && self.snapshot.power.value().is_some()
            && self.catalog_auto_generation != Some(self.generation)
        {
            // Once per connection: a receiver that cannot answer must not be
            // asked again at every state.
            self.catalog_auto_generation = Some(self.generation);
            self.announce("Refreshing source list…");
            tasks.push(self.request_source_catalog());
        }
        Task::batch(tasks)
    }

    fn on_bridge(&mut self, event: BridgeEvent) -> Task<Message> {
        // A receiver the user has since left no longer speaks for this window.
        if event.subscription != self.subscription {
            return Task::none();
        }
        let request_id = event.request_id;
        match event.event {
            PortEvent::State(state) => self.apply_state(*state),
            PortEvent::SessionEnded => {
                self.connection_lost();
                Task::none()
            }
            PortEvent::ConnectFailed(message) => {
                self.lifecycle = Lifecycle::Disconnected;
                self.state = None;
                self.snapshot.invalidate();
                self.zone2.invalidate();
                self.invalidate_supplemental();
                self.reset_pending_controls();
                // The unavailable screen, with its retry, is where to be: the
                // launch screen would wait for ever on a receiver that is not there.
                self.launch_ready = true;
                self.announce(format!("Operation failed: {message}"));
                Task::none()
            }
            PortEvent::Refreshed => Task::none(),
            PortEvent::Control(report) => self.on_control(request_id, report),
            PortEvent::SourceCatalog { generation, result } => {
                self.catalog_read_in_flight = false;
                if generation == self.generation {
                    let previous = self.source_catalog.clone();
                    let (catalog, _, outcome) =
                        catalog_policy::merge_refresh(previous, generation, result.map(|r| *r));
                    self.source_catalog = catalog;
                    match outcome {
                        Ok(()) => self.announce("Source list refreshed."),
                        Err(error) => self.announce(format!("Source list unavailable: {error}")),
                    }
                }
                Task::none()
            }
            PortEvent::QuickSelectNames { generation, result } => {
                self.names_read_in_flight = false;
                if generation == self.generation {
                    match result {
                        Ok(observation) => {
                            names_policy::apply_names(
                                &mut self.quick_select,
                                *observation,
                                generation,
                            );
                            self.announce("Quick Select names refreshed.");
                        }
                        Err(error) => {
                            tracing::debug!(%error, "Quick Select names unavailable");
                        }
                    }
                }
                Task::none()
            }
            PortEvent::HttpInformation { generation, result } => {
                self.http_read_in_flight = false;
                let again = std::mem::take(&mut self.http_read_again);
                if generation == self.generation {
                    let previous = self.snapshot.http_information.clone();
                    let (information, _) =
                        http_policy::merge_refresh(previous, generation, result.map(|r| *r));
                    self.snapshot.set_http_information(information);
                }
                if again {
                    self.request_http_information()
                } else {
                    Task::none()
                }
            }
            PortEvent::Failed(message) => {
                self.reset_pending_controls();
                self.announce(format!("Operation failed: {message}"));
                Task::none()
            }
        }
    }

    fn on_control(&mut self, request_id: u64, report: ControlReport) -> Task<Message> {
        // A control's end is terminal for the command that asked for it, even if
        // an unrelated read has since advanced the request counter.
        if self.volume_command_request_id == Some(request_id) {
            self.volume_command_pending = false;
            self.volume_command_request_id = None;
        }
        if self.sound_mode_request_id == Some(request_id) {
            self.sound_mode_request_id = None;
        }
        if matches!(report, ControlReport::Failed(_)) {
            self.sound_mode_category_filter = None;
        }
        self.announce(feedback::control_message(&report));
        if let ControlReport::Finished(snapshot) = &report {
            if snapshot.status == OperationStatus::Completed
                && http_policy::intent_may_have_changed(&snapshot.intent)
            {
                return self.request_http_information();
            }
        }
        Task::none()
    }

    /// Read the receiver's audio, video, and Audyssey information, one read at a
    /// time. A request that arrives while one is running runs once more after it.
    fn request_http_information(&mut self) -> Task<Message> {
        if self.selection.is_none()
            || !http_policy::should_read(&self.selected_capabilities(), true)
        {
            return Task::none();
        }
        if self.snapshot.power.value() != Some(&PowerState::On) {
            self.snapshot.invalidate_http_information(self.generation);
            return Task::none();
        }
        if self.http_read_in_flight {
            self.http_read_again = true;
            return Task::none();
        }
        self.http_read_in_flight = true;
        let id = self.next_request();
        self.command(BridgeCommand::ReadHttpInformation(id, self.generation))
    }

    fn request_source_catalog(&mut self) -> Task<Message> {
        if self.selection.is_none() || self.catalog_read_in_flight {
            return Task::none();
        }
        if !self.selected_capabilities().source_catalog_read {
            self.announce("The selected receiver has no validated source catalog capability.");
            return Task::none();
        }
        self.catalog_read_in_flight = true;
        let id = self.next_request();
        self.command(BridgeCommand::ReadSourceCatalog(id, self.generation))
    }

    fn request_quick_select_names(&mut self) -> Task<Message> {
        if self.selection.is_none()
            || self.names_read_in_flight
            || !self.selected_capabilities().quick_select_names
        {
            return Task::none();
        }
        self.names_read_in_flight = true;
        let id = self.next_request();
        self.command(BridgeCommand::ReadQuickSelectNames(id, self.generation))
    }

    fn complete_launch_if_ready(&mut self) {
        let saved_receiver = self.selection.is_some();
        let connected = matches!(self.lifecycle, Lifecycle::Connected { .. });
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
            Route::Dashboard => self.dashboard(),
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

fn selection_label(selection: &Selection) -> String {
    identity_label(&selection.identity)
}

fn identity_label(identity: &ReceiverIdentity) -> String {
    match (&identity.model, &identity.friendly_name) {
        (Some(model), Some(name)) => format!("{name} ({model})"),
        (Some(model), None) => model.clone(),
        (None, Some(name)) => name.clone(),
        (None, None) => identity.host.clone(),
    }
}

/// The configuration entry a discovered receiver is saved under: its model name,
/// or its address when it reports none.
fn receiver_entry_for_discovered(
    receiver: &denon_avr_domain::DiscoveredReceiver,
) -> (String, ReceiverIdentity) {
    let name = receiver
        .model
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| receiver.address.host.clone());
    let mut identity = receiver.identity();
    identity.friendly_name = Some(name.clone());
    (name, identity)
}

fn lifecycle_label(lifecycle: &Lifecycle) -> &'static str {
    match lifecycle {
        Lifecycle::Connected { .. } => "connected",
        Lifecycle::Reconnecting { .. } => "reconnecting",
        Lifecycle::Disconnected => "disconnected",
        Lifecycle::Selected => "selected",
        Lifecycle::NoReceiver => "no receiver",
        Lifecycle::Connecting => "connecting",
    }
}

pub fn boot_with_services(services: GuiServices) -> (Gui, Task<Message>) {
    let mut gui = Gui::new(services);
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
    let control = gui.control.clone();
    (
        gui,
        Task::perform(
            async move {
                control
                    .configuration()
                    .await
                    .map_err(|error| error.to_string())
            },
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
/// How often the receiver's HTTP information is read while connected.
const HTTP_REFRESH_INTERVAL: Duration = Duration::from_secs(15);

pub fn subscription(gui: &Gui) -> Subscription<Message> {
    Subscription::batch([
        gui.bridge
            .subscription()
            .map(|event| Message::Bridge(Box::new(event))),
        iced::keyboard::listen().map(Message::Keyboard),
        if gui.launch_ready && !gui.status_waiting() && gui.sound_mode_request_id.is_none() {
            Subscription::none()
        } else {
            iced::time::every(Duration::from_millis(180)).map(|_| Message::LaunchTick)
        },
        // The receiver offers no push for what it reports over HTTP, so it is
        // read again on a timer while connected.
        if gui.state.is_some() && matches!(gui.lifecycle, Lifecycle::Connected { .. }) {
            iced::time::every(HTTP_REFRESH_INTERVAL).map(|_| Message::HttpTick)
        } else {
            Subscription::none()
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

#[cfg(test)]
mod tests {
    use super::*;
    use denon_avr_application::ports::{
        AsyncConfigRepository, AsyncReceiverDiscovery, BoxFuture, OperationError,
        OperationErrorKind, ReceiverConnector,
    };
    use denon_avr_application::{ControlService, ServiceConfig, SharedReceiverSession};
    use denon_avr_domain::{
        CoreFrame, DiscoveredReceiver, Epoch, FrameSeq, MasterVolume, MonotonicMillis,
        ObservationOrigin, ReceiverEndpoint, SoundModeStatus, SourceId, ZonePower,
    };
    use std::sync::{Arc, Mutex};

    /// A connector that never reaches a receiver, so reducer tests need none.
    struct Unreachable;

    impl ReceiverConnector for Unreachable {
        fn connect<'a>(
            &'a self,
            _: &'a ReceiverId,
            _: &'a ReceiverIdentity,
        ) -> BoxFuture<'a, Result<SharedReceiverSession, OperationError>> {
            Box::pin(async {
                Err(OperationError::new(
                    OperationErrorKind::Connection,
                    "test session",
                    "no receiver session is required by GUI reducer tests",
                ))
            })
        }
    }

    struct Empty(Mutex<ConfiguredReceivers>);

    impl AsyncConfigRepository for Empty {
        fn load(&self) -> BoxFuture<'_, Result<ConfiguredReceivers, OperationError>> {
            Box::pin(async { Ok(self.0.lock().unwrap().clone()) })
        }
        fn save<'a>(
            &'a self,
            config: &'a ConfiguredReceivers,
        ) -> BoxFuture<'a, Result<(), OperationError>> {
            Box::pin(async move {
                *self.0.lock().unwrap() = config.clone();
                Ok(())
            })
        }
    }

    struct Nobody;

    impl AsyncReceiverDiscovery for Nobody {
        fn discover(
            &self,
            _: Duration,
        ) -> BoxFuture<'_, Result<Vec<DiscoveredReceiver>, OperationError>> {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    fn gui() -> Gui {
        let service = Arc::new(ControlService::new(
            Arc::new(Unreachable),
            Arc::new(Empty(Mutex::new(ConfiguredReceivers::default()))),
            Arc::new(Nobody),
            ServiceConfig::default(),
        ));
        Gui::new(GuiServices {
            control: service.operator(),
            shutdown: Arc::new(|| Box::pin(async {})),
        })
    }

    fn living_room() -> Selection {
        Selection {
            name: "living-room".into(),
            identity: ReceiverIdentity {
                host: "receiver.local".into(),
                model: Some("AVR-X3800H".into()),
                friendly_name: None,
            },
        }
    }

    /// A GUI with the living room chosen, as `Select` leaves it.
    fn selected_gui() -> Gui {
        let mut gui = gui();
        gui.selection = Some(living_room());
        gui.subscription = 1;
        gui.lifecycle = Lifecycle::Selected;
        gui
    }

    fn event(gui: &Gui, request_id: u64, event: PortEvent) -> Message {
        Message::Bridge(Box::new(BridgeEvent {
            request_id,
            subscription: gui.subscription,
            event,
        }))
    }

    /// The receiver's state at `epoch` with the given frames read.
    fn state_at(epoch: Option<u64>, frames: Vec<CoreFrame>) -> ReceiverState {
        let id = ReceiverId::new("living-room").unwrap();
        let mut state = ReceiverState::new(id.clone());
        let at = Epoch(epoch.unwrap_or(1));
        state.establish_epoch(at);
        for (index, frame) in frames.into_iter().enumerate() {
            let seq = index as u64 + 1;
            state.reduce(
                &id,
                at,
                FrameSeq(seq),
                MonotonicMillis(seq),
                MonotonicMillis(10_000),
                ObservationOrigin::ReceiverFrame,
                frame,
            );
        }
        if epoch.is_none() {
            state.mark_disconnected();
        }
        state
    }

    fn powered_on() -> Vec<CoreFrame> {
        vec![
            CoreFrame::MainZonePower(ZonePower::On),
            CoreFrame::Source(SourceId::new("GAME").unwrap()),
            CoreFrame::Volume(MasterVolume::db_half_steps(-90).unwrap()),
            CoreFrame::Mute(MuteState::Off),
            CoreFrame::SoundMode(SoundModeStatus {
                id: "STEREO".into(),
                raw: "MSSTEREO".into(),
            }),
            CoreFrame::Zone2Power(ZonePower::Off),
        ]
    }

    fn state_message(gui: &Gui, state: ReceiverState) -> Message {
        event(gui, gui.subscription, PortEvent::State(Box::new(state)))
    }

    #[tokio::test]
    async fn accessibility_overrides_are_session_only_and_update_theme_state() {
        let mut gui = gui();
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
        let mut gui = gui();
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

    #[tokio::test]
    async fn pure_filters_without_a_receiver_recall_while_movie_starts_one() {
        let mut gui = gui();

        let _ = gui.update(Message::SelectSoundModeCategory(SoundModeCategory::Pure));
        assert_eq!(
            gui.sound_mode_category_filter,
            Some(SoundModeCategory::Pure)
        );
        assert_eq!(gui.sound_mode_request_id, None);

        let _ = gui.update(Message::SelectSoundModeCategory(SoundModeCategory::Movie));
        assert_eq!(
            gui.sound_mode_category_filter,
            Some(SoundModeCategory::Movie)
        );
        assert!(gui.sound_mode_request_id.is_some());
    }

    #[tokio::test]
    async fn launch_opens_receiver_setup_when_no_receiver_is_saved() {
        let mut gui = gui();

        let _ = gui.update(Message::ConfigLoaded(Ok(ConfiguredReceivers::default())));

        assert!(gui.launch_ready);
        assert!(gui.selection.is_none());
        assert_eq!(gui.lifecycle, Lifecycle::NoReceiver);
    }

    #[tokio::test]
    async fn launch_opens_receiver_setup_when_configuration_fails() {
        let mut gui = gui();

        let _ = gui.update(Message::ConfigLoaded(Err(
            "configuration unavailable".into()
        )));

        assert!(gui.launch_ready);
    }

    #[tokio::test]
    async fn a_saved_receiver_is_selected_and_connected_at_launch() {
        let mut gui = gui();
        let config = ConfiguredReceivers {
            current: Some("living-room".into()),
            receivers: [("living-room".into(), living_room().identity)].into(),
            ..ConfiguredReceivers::default()
        };

        let _ = gui.update(Message::ConfigLoaded(Ok(config)));

        assert_eq!(gui.selection, Some(living_room()));
        assert_eq!(gui.lifecycle, Lifecycle::Selected);
        assert!(
            !gui.launch_ready,
            "the launch screen waits for the connection"
        );
        assert_ne!(
            gui.subscription, 0,
            "the receiver was chosen with a request"
        );
    }

    #[tokio::test]
    async fn a_name_the_port_cannot_use_is_reported_not_connected() {
        let mut gui = gui();
        let _ = gui.update(Message::Select(Selection {
            name: "adhoc:192.0.2.1".into(),
            identity: ReceiverIdentity::ad_hoc("192.0.2.1"),
        }));
        assert!(gui.launch_ready);
        assert_eq!(gui.subscription, 0);
        assert!(gui.announcement.starts_with("Cannot use the receiver name"));
    }

    #[tokio::test]
    async fn the_first_state_connects_and_opens_the_saved_receiver() {
        let mut gui = selected_gui();

        let _ = gui.update(state_message(&gui, state_at(Some(1), powered_on())));

        assert_eq!(gui.lifecycle, Lifecycle::Connected { generation: 1 });
        assert_eq!(gui.generation, 1);
        assert!(gui.launch_ready);
        assert_eq!(gui.snapshot.power.value(), Some(&PowerState::On));
        assert_eq!(gui.zone2.power.value(), Some(&PowerState::Standby));
        assert!(gui.state.is_some());
    }

    #[tokio::test]
    async fn fixed_capture_scenarios_do_not_need_a_receiver_connection() {
        let mut gui = gui();
        gui.configure_capture_scenario("source-picker").unwrap();
        assert_eq!(gui.lifecycle, Lifecycle::Connected { generation: 1 });
        assert_eq!(gui.snapshot.power.value(), Some(&PowerState::On));
        assert!(gui.source_picker_open);

        gui.configure_capture_scenario("unavailable").unwrap();
        assert_eq!(gui.lifecycle, Lifecycle::Disconnected);
        assert!(gui.snapshot.power.value().is_none());

        gui.configure_capture_scenario("diagnostics").unwrap();
        assert_eq!(gui.route, Route::Diagnostics);
        gui.configure_capture_scenario("messages").unwrap();
        assert!(gui.messages.len() >= 3);
    }

    #[tokio::test]
    async fn results_from_a_receiver_the_user_left_are_ignored() {
        let mut gui = selected_gui();
        gui.subscription = 4;
        let stale = Message::Bridge(Box::new(BridgeEvent {
            request_id: 3,
            subscription: 3,
            event: PortEvent::State(Box::new(state_at(Some(1), powered_on()))),
        }));
        let _ = gui.update(stale);
        assert_eq!(gui.lifecycle, Lifecycle::Selected);
        assert!(gui.state.is_none());
    }

    #[tokio::test]
    async fn a_lost_connection_invalidates_the_visible_status_and_a_new_epoch_recovers() {
        let mut gui = selected_gui();
        let _ = gui.update(state_message(&gui, state_at(Some(1), powered_on())));

        let _ = gui.update(state_message(&gui, state_at(None, powered_on())));
        assert_eq!(gui.lifecycle, Lifecycle::Reconnecting { generation: 2 });
        assert!(gui.snapshot.power.value().is_none());
        assert_eq!(
            gui.snapshot.freshness,
            denon_avr_domain::Freshness::Invalidated
        );

        let _ = gui.update(state_message(&gui, state_at(Some(2), powered_on())));
        assert_eq!(gui.lifecycle, Lifecycle::Connected { generation: 2 });
        assert_eq!(gui.generation, 2);
        assert_eq!(gui.snapshot.power.value(), Some(&PowerState::On));
    }

    #[tokio::test]
    async fn a_reconnect_seen_only_as_a_higher_epoch_is_still_a_reconnect() {
        let mut gui = selected_gui();
        let _ = gui.update(state_message(&gui, state_at(Some(1), powered_on())));
        gui.status_confirmed_generation = Some(1);
        gui.status_wait_ticks = 99;
        gui.volume_command_pending = true;

        // The channel coalesced the disconnect away.
        let _ = gui.update(state_message(&gui, state_at(Some(3), powered_on())));

        assert_eq!(gui.lifecycle, Lifecycle::Connected { generation: 3 });
        assert_eq!(gui.generation, 3);
        assert!(
            !gui.volume_command_pending,
            "a command from the old connection ended"
        );
        assert_eq!(gui.status_wait_ticks, 0);
        assert!(
            gui.messages
                .iter()
                .filter(|message| message.contains("status confirmed"))
                .count()
                >= 2
        );
    }

    #[tokio::test]
    async fn an_ended_session_reads_as_a_reconnect_until_the_next_state() {
        let mut gui = selected_gui();
        let _ = gui.update(state_message(&gui, state_at(Some(1), powered_on())));

        let _ = gui.update(event(&gui, 1, PortEvent::SessionEnded));
        assert_eq!(gui.lifecycle, Lifecycle::Reconnecting { generation: 2 });
        assert!(gui.snapshot.power.value().is_none());

        let _ = gui.update(state_message(&gui, state_at(Some(1), powered_on())));
        assert_eq!(gui.lifecycle, Lifecycle::Connected { generation: 1 });
    }

    #[tokio::test]
    async fn a_failed_connection_opens_the_unavailable_screen_and_retry_asks_again() {
        let mut gui = selected_gui();
        let _ = gui.update(event(&gui, 1, PortEvent::ConnectFailed("refused".into())));

        assert_eq!(gui.lifecycle, Lifecycle::Disconnected);
        assert!(gui.launch_ready, "the launch screen must not wait for ever");
        assert!(gui.announcement.contains("refused"));
        let (title, _, action) = power_recovery(&gui.lifecycle, &gui.snapshot);
        assert_eq!(title, "RECEIVER UNAVAILABLE");
        assert!(matches!(action, Some(("Retry Status", Message::Refresh))));

        let _ = gui.update(Message::Refresh);
        assert_eq!(gui.lifecycle, Lifecycle::Connecting);
    }

    #[tokio::test]
    async fn the_slider_follows_the_receivers_level_only_when_it_changes() {
        let mut gui = selected_gui();
        let _ = gui.update(state_message(&gui, state_at(Some(1), powered_on())));
        assert_eq!(gui.volume_slider, -45.0);

        // The user drags; a state that says nothing new about volume arrives.
        gui.volume_slider = -30.0;
        let mut frames = powered_on();
        frames.push(CoreFrame::Zone2Power(ZonePower::On));
        let _ = gui.update(state_message(&gui, state_at(Some(1), frames)));
        assert_eq!(
            gui.volume_slider, -30.0,
            "an unrelated change must not snap it back"
        );

        // The receiver's own level changes.
        let mut frames = powered_on();
        frames[2] = CoreFrame::Volume(MasterVolume::db_half_steps(-40).unwrap());
        let _ = gui.update(state_message(&gui, state_at(Some(1), frames)));
        assert_eq!(gui.volume_slider, -20.0);
    }

    #[tokio::test]
    async fn confirmed_status_is_logged_once_per_connection_generation() {
        let mut gui = selected_gui();
        for _ in 0..2 {
            let _ = gui.update(state_message(&gui, state_at(Some(1), powered_on())));
        }
        assert_eq!(
            gui.messages
                .iter()
                .filter(|message| message.contains("status confirmed"))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn a_control_carries_what_the_user_saw_and_a_power_control_carries_nothing() {
        let mut gui = selected_gui();
        let _ = gui.update(state_message(&gui, state_at(Some(1), powered_on())));

        let volume = ReceiverIntent::Volume(MasterVolume::db_half_steps(-80).unwrap());
        assert_eq!(
            gui.guard_for(&volume),
            Some(FieldBaseline::capture(
                gui.state.as_ref().unwrap(),
                denon_avr_domain::CoreField::Volume
            ))
        );
        for power in [
            ReceiverIntent::MainZonePower(ZonePower::Off),
            ReceiverIntent::Zone2Power(ZonePower::On),
        ] {
            assert_eq!(gui.guard_for(&power), None, "{power:?}");
        }
        // Before any state there is nothing the user could have seen.
        assert_eq!(selected_gui().guard_for(&volume), None);
    }

    #[tokio::test]
    async fn an_input_control_becomes_a_source_intent_and_the_picker_closes() {
        let mut gui = selected_gui();
        let _ = gui.update(Message::OpenSourcePicker);
        assert!(gui.source_picker_open);
        let _ = gui.update(Message::SelectInput("TV AUDIO".into()));
        assert!(!gui.source_picker_open);
        assert_eq!(
            projection::intent_for(&MainZoneControl::Input(Input::new("TV AUDIO").unwrap())),
            Ok(ReceiverIntent::Source(SourceId::new("TV AUDIO").unwrap()))
        );
    }

    #[tokio::test]
    async fn http_reads_run_one_at_a_time_and_a_request_during_one_runs_after_it() {
        let mut gui = selected_gui();
        let _ = gui.update(state_message(&gui, state_at(Some(1), powered_on())));
        // The first state asked for one read.
        assert!(gui.http_read_in_flight);
        assert!(!gui.http_read_again);

        let _ = gui.request_http_information();
        assert!(gui.http_read_again, "a second request waits its turn");

        let generation = gui.generation;
        let _ = gui.update(event(
            &gui,
            2,
            PortEvent::HttpInformation {
                generation,
                result: Err(OperationError::new(
                    OperationErrorKind::Timeout,
                    "HTTP information",
                    "late",
                )),
            },
        ));
        assert!(gui.http_read_in_flight, "the waiting request started");
        assert!(!gui.http_read_again);
    }

    #[tokio::test]
    async fn a_read_that_finishes_after_a_reconnect_is_dropped() {
        let mut gui = selected_gui();
        let _ = gui.update(state_message(&gui, state_at(Some(1), powered_on())));
        let _ = gui.update(state_message(&gui, state_at(Some(2), powered_on())));
        assert_eq!(gui.generation, 2);

        let before = gui.snapshot.http_information.clone();
        let _ = gui.update(event(
            &gui,
            2,
            PortEvent::HttpInformation {
                generation: 1,
                result: Ok(Box::new(denon_avr_domain::HttpInformationSnapshot {
                    generation: 1,
                    ..Default::default()
                })),
            },
        ));
        assert_eq!(gui.snapshot.http_information, before);
    }

    #[tokio::test]
    async fn the_source_catalog_is_asked_for_once_per_connection() {
        let mut gui = selected_gui();
        let _ = gui.update(state_message(&gui, state_at(Some(1), powered_on())));
        assert!(gui.catalog_read_in_flight);
        assert_eq!(gui.catalog_auto_generation, Some(1));

        // A failed read leaves the catalog unknown; further states must not ask again.
        let _ = gui.update(event(
            &gui,
            3,
            PortEvent::SourceCatalog {
                generation: 1,
                result: Err(OperationError::new(
                    OperationErrorKind::Connection,
                    "source catalog",
                    "refused",
                )),
            },
        ));
        assert!(!gui.catalog_read_in_flight);
        let _ = gui.update(state_message(&gui, state_at(Some(1), powered_on())));
        assert!(!gui.catalog_read_in_flight, "not asked a second time");
    }

    fn finished(
        status: denon_avr_application::OperationStatus,
        dispatch: denon_avr_domain::DispatchCertainty,
    ) -> ControlReport {
        ControlReport::Finished(Box::new(denon_avr_application::OperationSnapshot {
            id: denon_avr_domain::OperationId(1),
            receiver: ReceiverId::new("living-room").unwrap(),
            intent: ReceiverIntent::Mute(MuteState::On),
            status,
            dispatch,
            confirmed: false,
            reason: Some("because".into()),
            observation: None,
        }))
    }

    #[tokio::test]
    async fn bridge_failure_reenables_volume_controls() {
        let mut gui = selected_gui();
        gui.volume_command_pending = true;

        let _ = gui.update(event(&gui, 1, PortEvent::Failed("setting volume".into())));

        assert!(!gui.volume_command_pending);
        assert!(gui.announcement.contains("setting volume"));
    }

    #[tokio::test]
    async fn unknown_volume_steps_initialize_to_minimum_once_then_adjust() {
        let mut gui = selected_gui();
        assert_eq!(gui.volume_slider, MIN_VOLUME_DB);

        let _ = gui.update(Message::AdjustVolume(0.5));

        assert_eq!(gui.volume_slider, MIN_VOLUME_DB);
        assert!(gui.volume_command_pending);
        assert!(gui.volume_baseline_initialized);

        let request_id = gui.volume_command_request_id.expect("volume was submitted");
        let _ = gui.update(event(
            &gui,
            request_id,
            PortEvent::Control(finished(
                denon_avr_application::OperationStatus::Cancelled,
                denon_avr_domain::DispatchCertainty::NotDispatched,
            )),
        ));
        let _ = gui.update(Message::AdjustVolume(0.5));

        assert_eq!(gui.volume_slider, MIN_VOLUME_DB + 0.5);
        assert!(gui.volume_command_pending);
    }

    #[tokio::test]
    async fn volume_step_respects_the_receiver_ceiling_and_blocks_duplicates() {
        let mut gui = selected_gui();
        gui.snapshot.set_value(
            MainZoneValue::Volume(Volume::from_parts("95", 150)),
            StateAuthority::Authoritative,
        );
        gui.volume_baseline_initialized = true;
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
    async fn a_volume_confirmation_reenables_the_slider_whatever_the_request_counter_says() {
        let mut gui = selected_gui();
        gui.volume_command_pending = true;
        gui.volume_command_request_id = Some(4);
        // An unrelated read begun after the command, before its confirmation.
        gui.request_id = 5;

        let _ = gui.update(event(
            &gui,
            4,
            PortEvent::Control(finished(
                denon_avr_application::OperationStatus::Completed,
                denon_avr_domain::DispatchCertainty::CompleteWrite,
            )),
        ));

        assert!(!gui.volume_command_pending);
        assert_eq!(gui.volume_command_request_id, None);
        assert_eq!(gui.announcement, "Command confirmed by the receiver.");
    }

    #[tokio::test]
    async fn a_sound_mode_confirmation_reenables_mode_controls() {
        let mut gui = selected_gui();
        gui.sound_mode_request_id = Some(4);
        gui.request_id = 5;

        let _ = gui.update(event(&gui, 4, PortEvent::Control(ControlReport::Conflict)));

        assert_eq!(gui.sound_mode_request_id, None);
        assert_eq!(
            gui.announcement,
            "Receiver state changed before the command; retry."
        );
    }

    #[tokio::test]
    async fn a_write_that_went_out_but_was_not_confirmed_is_not_called_a_send_failure() {
        let mut gui = selected_gui();
        let _ = gui.update(event(
            &gui,
            2,
            PortEvent::Control(finished(
                denon_avr_application::OperationStatus::Indeterminate,
                denon_avr_domain::DispatchCertainty::CompleteWrite,
            )),
        ));
        assert!(gui
            .announcement
            .starts_with("Command sent, but confirmation is unavailable"));
    }

    #[tokio::test]
    async fn sound_mode_timeout_only_clears_its_own_pending_control() {
        let mut gui = gui();
        gui.sound_mode_request_id = Some(4);

        let _ = gui.update(Message::SoundModeControlTimedOut(3));
        assert_eq!(gui.sound_mode_request_id, Some(4));

        let _ = gui.update(Message::SoundModeControlTimedOut(4));
        assert_eq!(gui.sound_mode_request_id, None);
        assert_eq!(
            gui.announcement,
            "Sound Mode request timed out; controls re-enabled."
        );
    }

    #[test]
    fn unavailable_power_offers_a_recovery_action() {
        let (title, detail, action) =
            power_recovery(&Lifecycle::Disconnected, &MainZoneSnapshot::default());

        assert_eq!(title, "RECEIVER UNAVAILABLE");
        assert!(detail.contains("not queried"));
        assert!(matches!(action, Some(("Retry Status", Message::Refresh))));
    }

    #[tokio::test]
    async fn message_panel_is_bounded_and_preserves_chronological_order() {
        let mut gui = gui();
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
    async fn selecting_a_receiver_opens_the_console_and_records_context() {
        let mut gui = gui();
        gui.route = Route::Receivers;
        let selection = Selection {
            name: "Denon AVC-X3800H".into(),
            identity: ReceiverIdentity {
                host: "192.168.0.8".into(),
                model: Some("Denon AVC-X3800H".into()),
                friendly_name: Some("Denon AVC-X3800H".into()),
            },
        };

        let _ = gui.update(Message::Select(selection));

        assert_eq!(gui.route, Route::Dashboard);
        assert_eq!(
            gui.selection.as_ref().map(|s| s.identity().host),
            Some("192.168.0.8".into())
        );
        assert_eq!(gui.lifecycle, Lifecycle::Selected);
        assert!(gui
            .messages
            .back()
            .is_some_and(|message| message.contains("AVC-X3800H")));
    }

    #[test]
    fn a_discovered_receiver_is_saved_under_its_model_name() {
        let receiver = DiscoveredReceiver {
            address: ReceiverEndpoint {
                host: "192.168.0.8".into(),
                port: 23,
            },
            location: None,
            server: None,
            model: Some("Denon AVC-X3800H".into()),
            search_target: None,
            unique_service_name: None,
        };
        let (name, identity) = receiver_entry_for_discovered(&receiver);

        assert_eq!(name, "Denon AVC-X3800H");
        assert_eq!(identity.host, "192.168.0.8");
        assert_eq!(identity.friendly_name.as_deref(), Some("Denon AVC-X3800H"));
    }

    #[test]
    fn compact_source_picker_omits_aux_3_and_later() {
        assert!(source_is_picker_entry("AUX1"));
        assert!(source_is_picker_entry("AUX2"));
        assert!(!source_is_picker_entry("AUX3"));
        assert!(!source_is_picker_entry("AUX7"));
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

    #[test]
    fn speaker_state_keeps_unobserved_channels_unknown() {
        assert_eq!(channel_state(None, "FL"), "UNKNOWN");
        assert_eq!(channel_state(Some("FL C FR SL SR SBL SBR LFE"), "FL"), "ON");
        assert_eq!(
            channel_state(Some("FL C FR SL SR SBL SBR LFE"), "TML"),
            "OFF"
        );
    }
}
