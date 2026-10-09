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
use iced::widget::{container, row, scrollable, space, stack, text, text_input};
use iced::{Element, Length, Subscription, Task};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

mod bridge;
mod capture;
mod capture_scenario;
pub mod components;
mod controls;
mod dashboard;
pub mod design;
mod feedback;
mod keyboard;
mod messages;
mod preferences;
pub mod projection;
mod receiver_setup;
mod session;
mod settings_diagnostics;
mod setup;
mod shell;
mod sound_mode;
mod state;
mod supplemental;
mod update;
mod views;

use bridge::BridgeCommand;
pub use bridge::{BridgeEvent, ControlReport, GuiServices, PortBridge, PortEvent, ShutdownHook};
use capture_scenario::capture_window;
use dashboard::*;
pub use messages::Message;
use state::{CaptureSettings, LaunchProgress, SoundModeUi, SupplementalReads, VolumeControl};

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
    volume: VolumeControl,
    source_picker_open: bool,
    power_popup: Option<PowerPopup>,
    pub announcement: String,
    pub address: String,
    pub name: String,
    pub request_id: u64,
    pub generation: u64,
    pub quick_select: QuickSelectSnapshot,
    pub source_catalog: SourceCatalog,
    sound_mode: SoundModeUi,
    status_confirmed_generation: Option<u64>,
    /// Chronological, bounded command and lifecycle feedback for the global
    /// message panel. The panel is the only outcome surface in the shell.
    pub messages: VecDeque<String>,
    pub messages_collapsed: bool,
    pub text_scale: u8,
    pub motion_preference: MotionPreference,
    pub contrast_preference: ContrastPreference,
    capture: CaptureSettings,
    pub launch_ready: bool,
    launch: LaunchProgress,
    /// The newest receiver state the views were built from. It is what a
    /// control's guard is captured against.
    state: Option<ReceiverState>,
    /// The `Select` request the displayed receiver was chosen with. Events from
    /// an earlier one belong to a receiver the user has left.
    subscription: u64,
    supplemental: SupplementalReads,
    bridge: PortBridge,
    control: SharedOperatorControl,
    /// Closes the receiver connection. Supplied by whoever composed the service.
    shutdown: ShutdownHook,
    /// The window is closing: the receiver connection is being closed first, and
    /// what the closing connection reports is not news.
    closing: bool,
}

impl Gui {
    pub fn new(services: GuiServices) -> Self {
        let control = std::sync::Arc::clone(&services.control);
        let shutdown = std::sync::Arc::clone(&services.shutdown);
        let bridge = PortBridge::new(std::sync::Arc::clone(&services.control));
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
            volume: VolumeControl {
                value: None,
                value_request_id: 0,
                command_pending: false,
                baseline_initialized: false,
                command_request_id: None,
            },
            source_picker_open: false,
            power_popup: None,
            announcement: "Loading configuration…".into(),
            address: String::new(),
            name: String::new(),
            request_id: 0,
            generation: 0,
            quick_select: QuickSelectSnapshot::default(),
            source_catalog: SourceCatalog::default(),
            sound_mode: SoundModeUi {
                category_filter: None,
                request_id: None,
                save_in_flight: false,
                pending_config: None,
            },
            status_confirmed_generation: None,
            messages: VecDeque::new(),
            messages_collapsed: false,
            text_scale: 100,
            motion_preference: MotionPreference::Normal,
            contrast_preference: ContrastPreference::Normal,
            capture: CaptureSettings {
                directory: std::env::var_os("DENON_AVR_CAPTURE_DIR").map(PathBuf::from),
                scenario: None,
            },
            launch_ready: false,
            launch: LaunchProgress {
                frame: 0,
                status_wait_ticks: 0,
            },
            state: None,
            subscription: 0,
            supplemental: SupplementalReads {
                http_read_in_flight: false,
                http_read_again: false,
                catalog_read_in_flight: false,
                names_read_in_flight: false,
                catalog_auto_generation: None,
            },
            bridge,
            control,
            shutdown,
            closing: false,
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
        self.launch.status_wait_ticks < STATUS_WAIT_LIMIT_TICKS
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
        let task = if gui.capture.directory.is_some() {
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
        iced::window::close_requests().map(Message::CloseRequested),
        if gui.launch_ready && !gui.status_waiting() && gui.sound_mode.request_id.is_none() {
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

#[cfg(test)]
mod tests;
