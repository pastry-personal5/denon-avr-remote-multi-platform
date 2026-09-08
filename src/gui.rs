//! Iced presentation layer for the desktop client.
//!
//! This module deliberately contains no AVR protocol knowledge.  All receiver
//! policy, confirmation, retry, and lifecycle decisions remain in the typed
//! application controller.

use crate::application::ports::{AsyncReceiverDiscovery, ConfigRepository};
use crate::application::{
    ControlResult, ControllerHandle, ReceiverController, ReceiverEvent, ReceiverSelection,
    SessionFactory,
};
use crate::domain::{
    ConfiguredReceivers, ListeningModeGroup, MainZoneControl, MainZoneSnapshot, Model,
    ModelCapabilities, PowerState,
};
use crate::infrastructure::{AvrSessionFactory, SsdpDiscoveryAdapter, YamlConfigRepository};
use iced::futures::SinkExt;
use iced::widget::{button, column, container, row, text, text_input};
use iced::{Element, Length, Subscription, Task};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Dashboard,
    Receivers,
    Settings,
    Diagnostics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowClass {
    Wide,
    Compact,
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
    Shutdown(u64),
}

/// A single serialized owner of `ControllerHandle`.
#[derive(Clone)]
pub struct ControllerBridge {
    commands: mpsc::Sender<BridgeCommand>,
    events: Arc<Mutex<mpsc::Receiver<BridgeEvent>>>,
}

impl ControllerBridge {
    pub fn new<F: SessionFactory>(factory: F) -> Self {
        let (commands, mut command_rx) = mpsc::channel(16);
        let (event_sender, event_rx) = mpsc::channel(32);
        let events = Arc::new(Mutex::new(event_rx));
        tokio::spawn(async move {
            let handle = ReceiverController::spawn(factory, Default::default());
            run_bridge(handle, &mut command_rx, event_sender).await;
        });
        Self {
            commands,
            events: Arc::clone(&events),
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
    while let Some(command) = commands.recv().await {
        let is_shutdown = matches!(&command, BridgeCommand::Shutdown(_));
        let (request_id, result) = match command {
            BridgeCommand::Select(id, selection) => {
                let selected = handle.select(selection).await;
                if selected.is_ok() {
                    (id, handle.connect().await)
                } else {
                    (id, selected)
                }
            }
            BridgeCommand::Connect(id) => (id, handle.connect().await),
            BridgeCommand::Refresh(id) => (id, handle.refresh().await),
            BridgeCommand::Disconnect(id) => (id, handle.disconnect().await),
            BridgeCommand::Control(id, control, version) => {
                (id, handle.control(control, version).await)
            }
            BridgeCommand::Shutdown(id) => (id, handle.shutdown().await),
        };
        let event = result
            .map(|reply| reply.unwrap_or(ReceiverEvent::Cancelled))
            .unwrap_or_else(|error| {
                ReceiverEvent::Diagnostic(crate::application::Diagnostic::Timeout {
                    context: error.to_string(),
                })
            });
        let generation = match &event {
            ReceiverEvent::Lifecycle(crate::application::Lifecycle::Connected { generation })
            | ReceiverEvent::Lifecycle(crate::application::Lifecycle::Reconnecting {
                generation,
            }) => *generation,
            _ => 0,
        };
        let _ = event_sender
            .send(BridgeEvent {
                request_id,
                generation,
                event,
            })
            .await;
        while let Ok(Some(controller_event)) =
            tokio::time::timeout(Duration::from_millis(1), handle.next_event()).await
        {
            let generation = match &controller_event {
                ReceiverEvent::Lifecycle(crate::application::Lifecycle::Connected {
                    generation,
                })
                | ReceiverEvent::Lifecycle(crate::application::Lifecycle::Reconnecting {
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
        if is_shutdown {
            break;
        }
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
    DiscoveryFinished(Result<Vec<crate::domain::DiscoveredReceiver>, String>),
    ManualSetup,
    Select(ReceiverSelection),
    Connect,
    Refresh,
    Disconnect,
    PowerOn,
    PowerOff,
    Mute,
    Unmute,
    SelectListeningModeGroup(ListeningModeGroup),
    SelectSurroundMode(String),
    Shutdown,
    Bridge(Box<BridgeEvent>),
}

pub struct Gui {
    pub route: Route,
    pub window_class: WindowClass,
    pub configured: ConfiguredReceivers,
    pub discovered: Vec<crate::domain::DiscoveredReceiver>,
    pub selection: Option<ReceiverSelection>,
    pub lifecycle: crate::application::Lifecycle,
    pub snapshot: MainZoneSnapshot,
    pub announcement: String,
    pub address: String,
    pub name: String,
    pub request_id: u64,
    pub generation: u64,
    bridge: ControllerBridge,
}

impl Gui {
    pub fn new(bridge: ControllerBridge) -> Self {
        Self {
            route: Route::Dashboard,
            window_class: WindowClass::Wide,
            configured: ConfiguredReceivers::default(),
            discovered: Vec::new(),
            selection: None,
            lifecycle: crate::application::Lifecycle::NoReceiver,
            snapshot: MainZoneSnapshot::default(),
            announcement: "Loading configuration…".into(),
            address: String::new(),
            name: String::new(),
            request_id: 0,
            generation: 0,
            bridge,
        }
    }

    fn next_request(&mut self) -> u64 {
        self.request_id += 1;
        self.request_id
    }
    fn command(&mut self, command: BridgeCommand) -> Task<Message> {
        let bridge = self.bridge.clone();
        Task::perform(
            async move { bridge.send(command).await },
            Message::CommandFinished,
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ConfigLoaded(Ok(config)) => {
                self.configured = config;
                self.announcement = if let Some((name, identity)) = self.configured.current() {
                    self.selection = Some(ReceiverSelection::Saved {
                        name: name.into(),
                        identity: identity.clone(),
                    });
                    "Saved receiver selected; connecting…".into()
                } else {
                    "Connect a receiver to see its status.".into()
                };
                if let Some(selection) = self.selection.clone() {
                    let id = self.next_request();
                    self.command(BridgeCommand::Select(id, selection))
                } else {
                    Task::none()
                }
            }
            Message::ConfigLoaded(Err(error)) => {
                self.announcement = format!("Configuration unavailable: {error}");
                Task::none()
            }
            Message::CommandFinished(Err(error)) => {
                self.announcement = format!("Operation failed: {error}");
                Task::none()
            }
            Message::CommandFinished(Ok(())) => Task::none(),
            Message::Navigate(route) => {
                self.route = route;
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
                self.snapshot.invalidate();
                let id = self.next_request();
                self.command(BridgeCommand::Select(id, selection))
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
            Message::Mute => self.control(MainZoneControl::Mute(crate::domain::MuteState::On)),
            Message::Unmute => self.control(MainZoneControl::Mute(crate::domain::MuteState::Off)),
            Message::SelectListeningModeGroup(group) => {
                self.control(MainZoneControl::ListeningModeGroup(group))
            }
            Message::SelectSurroundMode(value) => match crate::domain::SurroundMode::new(value) {
                Ok(mode) => self.control(MainZoneControl::SurroundMode(mode)),
                Err(error) => {
                    self.announcement = error.into();
                    Task::none()
                }
            },
            Message::Discover => {
                self.announcement = "Searching for receivers…".into();
                Task::perform(
                    async {
                        SsdpDiscoveryAdapter
                            .discover(
                                crate::infrastructure::discovery_ssdp::DEFAULT_DISCOVERY_TIMEOUT,
                            )
                            .await
                            .map_err(|error| error.to_string())
                    },
                    Message::DiscoveryFinished,
                )
            }
            Message::DiscoveryFinished(Ok(receivers)) => {
                self.discovered = receivers;
                self.announcement = format!("Found {} receiver(s).", self.discovered.len());
                Task::none()
            }
            Message::DiscoveryFinished(Err(error)) => {
                self.announcement = format!("Discovery failed: {error}");
                Task::none()
            }
            Message::ManualSetup => {
                if self.address.trim().is_empty() {
                    self.announcement = "Enter a receiver address first.".into();
                    return Task::none();
                }
                let identity = crate::domain::ReceiverIdentity {
                    host: self.address.trim().to_owned(),
                    model: None,
                    friendly_name: (!self.name.trim().is_empty())
                        .then(|| self.name.trim().to_owned()),
                };
                self.selection = Some(ReceiverSelection::ExplicitHost(identity.clone()));
                let id = self.next_request();
                self.command(BridgeCommand::Select(
                    id,
                    ReceiverSelection::ExplicitHost(identity),
                ))
            }
            Message::Shutdown => {
                let id = self.next_request();
                self.command(BridgeCommand::Shutdown(id))
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
                    ReceiverEvent::Lifecycle(lifecycle) => self.lifecycle = lifecycle,
                    ReceiverEvent::Snapshot(snapshot) => self.snapshot = snapshot,
                    ReceiverEvent::Control(result) => self.announcement = control_message(&result),
                    ReceiverEvent::FieldError { field, error } => {
                        self.announcement =
                            format!("{} unavailable: {}", field.name(), error.message)
                    }
                    ReceiverEvent::Diagnostic(diagnostic) => {
                        self.announcement = format!("Diagnostic: {diagnostic:?}")
                    }
                    _ => {}
                }
                Task::none()
            }
        }
    }

    fn control(&mut self, control: MainZoneControl) -> Task<Message> {
        let id = self.next_request();
        self.announcement = "Command pending; waiting for receiver confirmation…".into();
        self.command(BridgeCommand::Control(
            id,
            control,
            Some(self.snapshot.resource_version()),
        ))
    }

    pub fn view(&self) -> Element<'_, Message> {
        let nav = row![
            button("Dashboard").on_press(Message::Navigate(Route::Dashboard)),
            button("Receivers").on_press(Message::Navigate(Route::Receivers)),
            button("Settings").on_press(Message::Navigate(Route::Settings)),
            button("Diagnostics").on_press(Message::Navigate(Route::Diagnostics))
        ]
        .spacing(8);
        let toolbar = row![
            text("Main Zone").size(20),
            button("Refresh status").on_press(Message::Refresh),
            text(format!("{} · {:?}", self.announcement, self.lifecycle))
        ]
        .spacing(16);
        let body = match self.route {
            Route::Dashboard => self.dashboard(),
            Route::Receivers => self.receivers(),
            Route::Settings => column![
                text("Settings").size(32),
                text("Dark appearance · YAML configuration")
            ]
            .spacing(16),
            Route::Diagnostics => column![
                text("Diagnostics").size(32),
                text(format!("Lifecycle: {:?}", self.lifecycle)),
                text(format!("Snapshot authority: {:?}", self.snapshot.authority))
            ]
            .spacing(16),
        };
        container(column![nav, toolbar, body].spacing(20).padding(24))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn dashboard(&self) -> iced::widget::Column<'_, Message> {
        let value = |label: &str, value: String| {
            row![
                text(label.to_owned()).width(Length::Fixed(150.0)),
                text(value)
            ]
            .spacing(12)
        };
        let power = self
            .snapshot
            .power
            .value()
            .map(ToString::to_string)
            .unwrap_or_else(|| "Unavailable".into());
        let input = self
            .snapshot
            .input
            .value()
            .map(ToString::to_string)
            .unwrap_or_else(|| "Unavailable".into());
        let volume = self
            .snapshot
            .volume
            .value()
            .map(ToString::to_string)
            .unwrap_or_else(|| "Unavailable".into());
        let mute = self
            .snapshot
            .mute
            .value()
            .map(ToString::to_string)
            .unwrap_or_else(|| "Unavailable".into());
        let surround = self
            .snapshot
            .surround_mode
            .value()
            .map(ToString::to_string)
            .unwrap_or_else(|| "Unavailable".into());
        let writable = self
            .selection
            .as_ref()
            .and_then(|selection| selection.identity().model)
            .map(|model| ModelCapabilities::for_model(Model::from_reported(&model)).writable)
            .unwrap_or(false);
        let controls = if writable {
            row![
                button("Mute").on_press(Message::Mute),
                button("Unmute").on_press(Message::Unmute),
                button("Power on").on_press(Message::PowerOn),
                button("Standby").on_press(Message::PowerOff)
            ]
        } else {
            row![text(
                "Read-only: this receiver model is not validated for controls."
            )]
        };
        let group_controls = if writable {
            ListeningModeGroup::ALL
                .into_iter()
                .fold(row![].spacing(8), |row, group| {
                    row.push(
                        button(group.as_str()).on_press(Message::SelectListeningModeGroup(group)),
                    )
                })
        } else {
            row![text("Mode groups unavailable for this receiver.")]
        };
        let mode_controls = if writable {
            // Context is intentionally not guessed from the selected source;
            // until SD?/DC? context is available, individual choices remain disabled.
            row![text(
                "Listening-mode choices unavailable: validated signal and speaker context is not available."
            )]
        } else {
            row![text("Listening modes unavailable for this receiver.")]
        };
        column![
            text("Dashboard").size(32),
            text(format!(
                "{:?} · {:?}",
                self.snapshot.freshness, self.snapshot.authority
            )),
            text("Power and source").size(22),
            value("Power", power),
            value("Source", input),
            text("Volume and mute").size(22),
            value("Volume", volume),
            row![value("Mute", mute), controls],
            text("Sound mode").size(22),
            value("Mode", surround),
            text("Mode group").size(18),
            group_controls,
            mode_controls,
        ]
        .spacing(14)
    }

    fn receivers(&self) -> iced::widget::Column<'_, Message> {
        let mut page = column![text("Receivers").size(32), text("Saved receivers")].spacing(12);
        for (name, identity) in &self.configured.receivers {
            page = page.push(
                row![
                    text(format!("{name} · {}", identity.host)),
                    button("Select").on_press(Message::Select(ReceiverSelection::Saved {
                        name: name.clone(),
                        identity: identity.clone()
                    }))
                ]
                .spacing(12),
            );
        }
        for receiver in &self.discovered {
            page = page.push(
                row![
                    text(format!(
                        "{} · {}",
                        receiver.model.as_deref().unwrap_or("Unknown model"),
                        receiver.address.host
                    )),
                    button("Select").on_press(Message::Select(ReceiverSelection::Discovered(
                        receiver.clone()
                    )))
                ]
                .spacing(12),
            );
        }
        page.push(text("Find a receiver"))
            .push(button("Discover").on_press(Message::Discover))
            .push(
                row![
                    text_input("Receiver address", &self.address).on_input(Message::AddressChanged),
                    text_input("Name (optional)", &self.name).on_input(Message::NameChanged),
                    button("Save and connect").on_press(Message::ManualSetup)
                ]
                .spacing(8),
            )
    }
}

fn control_message(result: &ControlResult) -> String {
    format!("Control outcome: {result:?}")
}

pub fn boot() -> (Gui, Task<Message>) {
    let bridge = ControllerBridge::new(AvrSessionFactory::default());
    let gui = Gui::new(bridge);
    let repository = YamlConfigRepository::default();
    (
        gui,
        Task::perform(
            async move { repository.load().map_err(|error| error.to_string()) },
            Message::ConfigLoaded,
        ),
    )
}

pub fn update(gui: &mut Gui, message: Message) -> Task<Message> {
    gui.update(message)
}
pub fn view(gui: &Gui) -> Element<'_, Message> {
    gui.view()
}
pub fn subscription(gui: &Gui) -> Subscription<Message> {
    gui.bridge
        .subscription()
        .map(|event| Message::Bridge(Box::new(event)))
}

pub fn run() -> iced::Result {
    iced::application(boot, update, view)
        .subscription(subscription)
        .title("Denon AVR Remote")
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;
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
            event: ReceiverEvent::Lifecycle(crate::application::Lifecycle::Connected {
                generation: 1,
            }),
        })));
        assert_eq!(gui.lifecycle, crate::application::Lifecycle::NoReceiver);
    }
}
