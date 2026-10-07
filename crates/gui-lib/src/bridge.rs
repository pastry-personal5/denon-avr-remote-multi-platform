//! Serialized adapter between Iced subscriptions and the control-service port.
//!
//! One task owns the port handle and the state subscription for the selected
//! receiver. Commands from the GUI arrive in order. A control, a refresh, and a
//! supplemental read each run on a task of their own, so a slow receiver never
//! holds back state updates; their results come back as events tagged with the
//! subscription they were started under, and the GUI drops the ones that belong
//! to a receiver it has since left.

use denon_avr_application::ports::{BoxFuture, OperationError, OperationErrorKind};
use denon_avr_application::{
    ControlError, OperationSnapshot, OperationSubmission, SharedOperatorControl, StateSubscription,
};
use denon_avr_domain::{
    FieldBaseline, HttpInformationSnapshot, QuickSelectNameObservation, ReceiverId, ReceiverIntent,
    ReceiverState, SourceCatalogObservation,
};
use iced::futures::SinkExt;
use iced::Subscription;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};

/// How long one wait for an operation may last, and how many waits are made
/// before the bridge reports that the receiver never answered. The service caps
/// a single wait at 30 s.
const OPERATION_WAIT: Duration = Duration::from_secs(30);
const OPERATION_WAITS: u32 = 2;

/// Supplied by the composition root. The GUI never names the service that owns
/// the receiver connection.
pub type ShutdownHook = Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>;

/// Presentation-facing services supplied by the desktop composition root.
#[derive(Clone)]
pub struct GuiServices {
    pub control: SharedOperatorControl,
    pub shutdown: ShutdownHook,
}

/// How a control ended, as far as the GUI is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlReport {
    /// The service resolved the operation. The snapshot says how.
    Finished(Box<OperationSnapshot>),
    /// What the user saw when they acted is no longer what the receiver shows.
    /// Nothing was submitted.
    Conflict,
    /// The operation could not be submitted, or its outcome could not be read.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortEvent {
    /// The newest state of the selected receiver. The first one follows a
    /// connect and synchronization, so it is complete.
    State(Box<ReceiverState>),
    /// The session behind the subscription closed. The bridge is subscribing
    /// again; a `State` follows if that works.
    SessionEnded,
    ConnectFailed(String),
    /// A refresh finished. The state it read arrives as `State`.
    Refreshed,
    Control(ControlReport),
    /// Reads are stamped with the connection generation they were started in,
    /// so one that finishes after a reconnect can be told from a current one.
    SourceCatalog {
        generation: u64,
        result: Result<Box<SourceCatalogObservation>, OperationError>,
    },
    QuickSelectNames {
        generation: u64,
        result: Result<Box<QuickSelectNameObservation>, OperationError>,
    },
    HttpInformation {
        generation: u64,
        result: Result<Box<HttpInformationSnapshot>, OperationError>,
    },
    /// A call failed in a way that has no event of its own.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeEvent {
    /// The GUI request that caused the event, or the one that selected the
    /// receiver for events nobody asked for.
    pub request_id: u64,
    /// The `Select` request the receiver was chosen with.
    pub subscription: u64,
    pub event: PortEvent,
}

#[derive(Debug, Clone)]
pub(crate) enum BridgeCommand {
    Select(u64, ReceiverId),
    Refresh(u64),
    Control {
        id: u64,
        intent: ReceiverIntent,
        /// What the user saw of the target field when they acted. The control
        /// is refused if the receiver now shows something else.
        guard: Option<FieldBaseline>,
    },
    ReadSourceCatalog(u64, u64),
    ReadQuickSelectNames(u64, u64),
    ReadHttpInformation(u64, u64),
    Shutdown,
}

/// A single serialized owner of the port handle.
#[derive(Clone)]
pub struct PortBridge {
    commands: mpsc::Sender<BridgeCommand>,
    events: Arc<Mutex<mpsc::Receiver<BridgeEvent>>>,
}

impl PortBridge {
    pub fn new(services: GuiServices) -> Self {
        let (commands, command_rx) = mpsc::channel(16);
        let (event_sender, event_rx) = mpsc::channel(32);
        tokio::spawn(run_bridge(services, command_rx, event_sender));
        Self {
            commands,
            events: Arc::new(Mutex::new(event_rx)),
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

    /// The next event, for a driver that does not run the Iced subscription
    /// (headless tests). The subscription and this share one queue, so use one.
    pub async fn recv(&self) -> Option<BridgeEvent> {
        self.events.lock().await.recv().await
    }

    pub(crate) async fn send(&self, command: BridgeCommand) -> Result<(), String> {
        self.commands
            .send(command)
            .await
            .map_err(|_| "receiver bridge stopped".into())
    }
}

#[derive(Clone)]
struct SubscriptionData(Arc<Mutex<mpsc::Receiver<BridgeEvent>>>);

impl Hash for SubscriptionData {
    fn hash<H: Hasher>(&self, state: &mut H) {
        0x5d_u8.hash(state);
    }
}

enum Step {
    Command(Option<BridgeCommand>),
    State(Result<Box<ReceiverState>, OperationError>),
}

/// The receiver the GUI has chosen, and the `Select` request that chose it.
#[derive(Clone)]
struct Selected {
    receiver: ReceiverId,
    subscription: u64,
}

async fn next_state(
    active: &mut Option<StateSubscription>,
) -> Result<Box<ReceiverState>, OperationError> {
    match active {
        Some(subscription) => subscription.changed().await.map(Box::new),
        None => std::future::pending().await,
    }
}

async fn run_bridge(
    services: GuiServices,
    mut commands: mpsc::Receiver<BridgeCommand>,
    events: mpsc::Sender<BridgeEvent>,
) {
    let mut selected: Option<Selected> = None;
    let mut active: Option<StateSubscription> = None;
    loop {
        let step = tokio::select! {
            command = commands.recv() => Step::Command(command),
            next = next_state(&mut active) => Step::State(next),
        };
        match step {
            Step::Command(None) => break,
            Step::Command(Some(command)) => {
                tracing::debug!(command = command_name(&command), "processing GUI command");
                let stop = matches!(command, BridgeCommand::Shutdown);
                handle(&services, command, &mut selected, &mut active, &events).await;
                if stop {
                    tracing::info!("receiver bridge stopped");
                    break;
                }
            }
            Step::State(Ok(state)) => {
                if let Some(chosen) = &selected {
                    send(
                        &events,
                        chosen.subscription,
                        chosen.subscription,
                        PortEvent::State(state),
                    )
                    .await;
                }
            }
            Step::State(Err(error)) => {
                // The service closed the session: its entry changed, or it was
                // shut down. Subscribe again, which connects at the address now
                // in the file.
                tracing::info!(%error, "receiver session ended; subscribing again");
                active = None;
                if let Some(chosen) = selected.clone() {
                    send(
                        &events,
                        chosen.subscription,
                        chosen.subscription,
                        PortEvent::SessionEnded,
                    )
                    .await;
                    active = subscribe(&services, &chosen, &events).await;
                }
            }
        }
    }
}

async fn handle(
    services: &GuiServices,
    command: BridgeCommand,
    selected: &mut Option<Selected>,
    active: &mut Option<StateSubscription>,
    events: &mpsc::Sender<BridgeEvent>,
) {
    match command {
        BridgeCommand::Select(id, receiver) => {
            // Dropping the old subscription lets the service release the old
            // receiver after its idle time.
            *active = None;
            let chosen = Selected {
                receiver,
                subscription: id,
            };
            *selected = Some(chosen.clone());
            *active = subscribe(services, &chosen, events).await;
        }
        BridgeCommand::Refresh(id) => {
            let Some(chosen) = selected.clone() else {
                send(
                    events,
                    id,
                    0,
                    PortEvent::Failed("no receiver selected".into()),
                )
                .await;
                return;
            };
            if active.is_none() {
                // The connection failed or ended: retrying is connecting again.
                *active = subscribe(services, &chosen, events).await;
                return;
            }
            let control = Arc::clone(&services.control);
            let events = events.clone();
            tokio::spawn(async move {
                let event = match control.refresh(&chosen.receiver).await {
                    Ok(_) => PortEvent::Refreshed,
                    Err(error) => PortEvent::Failed(error.to_string()),
                };
                send(&events, id, chosen.subscription, event).await;
            });
        }
        BridgeCommand::Control { id, intent, guard } => {
            let Some(chosen) = selected.clone() else {
                send(
                    events,
                    id,
                    0,
                    PortEvent::Failed("no receiver selected".into()),
                )
                .await;
                return;
            };
            if let (Some(baseline), Some(subscription)) = (&guard, active.as_ref()) {
                if !baseline.holds_in(&subscription.latest()) {
                    let report = PortEvent::Control(ControlReport::Conflict);
                    send(events, id, chosen.subscription, report).await;
                    return;
                }
            }
            let control = Arc::clone(&services.control);
            let events = events.clone();
            tokio::spawn(async move {
                let report = run_control(&control, &chosen.receiver, intent).await;
                send(&events, id, chosen.subscription, PortEvent::Control(report)).await;
            });
        }
        BridgeCommand::ReadSourceCatalog(id, generation) => {
            spawn_read(services, events, selected, id, move |control, receiver| {
                Box::pin(async move {
                    let result = control
                        .source_catalog(&receiver)
                        .await
                        .map(Box::new)
                        .map_err(operation_error);
                    PortEvent::SourceCatalog { generation, result }
                })
            })
        }
        BridgeCommand::ReadQuickSelectNames(id, generation) => {
            spawn_read(services, events, selected, id, move |control, receiver| {
                Box::pin(async move {
                    let result = control
                        .quick_select_names(&receiver)
                        .await
                        .map(Box::new)
                        .map_err(operation_error);
                    PortEvent::QuickSelectNames { generation, result }
                })
            })
        }
        BridgeCommand::ReadHttpInformation(id, generation) => {
            spawn_read(services, events, selected, id, move |control, receiver| {
                Box::pin(async move {
                    let result = control
                        .http_information(&receiver)
                        .await
                        .map(Box::new)
                        .map_err(operation_error);
                    PortEvent::HttpInformation { generation, result }
                })
            })
        }
        BridgeCommand::Shutdown => {
            *active = None;
            (services.shutdown)().await;
        }
    }
}

fn spawn_read(
    services: &GuiServices,
    events: &mpsc::Sender<BridgeEvent>,
    selected: &Option<Selected>,
    id: u64,
    read: impl FnOnce(SharedOperatorControl, ReceiverId) -> BoxFuture<'static, PortEvent>
        + Send
        + 'static,
) {
    let Some(chosen) = selected.clone() else {
        return;
    };
    let control = Arc::clone(&services.control);
    let events = events.clone();
    tokio::spawn(async move {
        let event = read(control, chosen.receiver).await;
        send(&events, id, chosen.subscription, event).await;
    });
}

/// Subscribe to the chosen receiver, which connects and synchronizes it first.
async fn subscribe(
    services: &GuiServices,
    chosen: &Selected,
    events: &mpsc::Sender<BridgeEvent>,
) -> Option<StateSubscription> {
    match services.control.state(&chosen.receiver).await {
        Ok(subscription) => {
            let initial = subscription.latest();
            send(
                events,
                chosen.subscription,
                chosen.subscription,
                PortEvent::State(Box::new(initial)),
            )
            .await;
            Some(subscription)
        }
        Err(error) => {
            send(
                events,
                chosen.subscription,
                chosen.subscription,
                PortEvent::ConnectFailed(error.to_string()),
            )
            .await;
            None
        }
    }
}

/// Submit an operation and wait for it to end.
async fn run_control(
    control: &SharedOperatorControl,
    receiver: &ReceiverId,
    intent: ReceiverIntent,
) -> ControlReport {
    let mut snapshot = match control
        .submit(receiver, OperationSubmission::new(intent))
        .await
    {
        Ok(snapshot) => snapshot,
        Err(error) => return ControlReport::Failed(error.to_string()),
    };
    for _ in 0..OPERATION_WAITS {
        if snapshot.status.is_terminal() {
            return ControlReport::Finished(Box::new(snapshot));
        }
        snapshot = match control.operation(snapshot.id, Some(OPERATION_WAIT)).await {
            Ok(snapshot) => snapshot,
            Err(error) => return ControlReport::Failed(error.to_string()),
        };
    }
    if snapshot.status.is_terminal() {
        ControlReport::Finished(Box::new(snapshot))
    } else {
        ControlReport::Failed("the receiver did not finish the operation in time".into())
    }
}

async fn send(
    events: &mpsc::Sender<BridgeEvent>,
    request_id: u64,
    subscription: u64,
    event: PortEvent,
) {
    let _ = events
        .send(BridgeEvent {
            request_id,
            subscription,
            event,
        })
        .await;
}

fn operation_error(error: ControlError) -> OperationError {
    match error {
        ControlError::Receiver(error) => error,
        other => OperationError::new(
            OperationErrorKind::Connection,
            "receiver",
            other.to_string(),
        ),
    }
}

fn command_name(command: &BridgeCommand) -> &'static str {
    match command {
        BridgeCommand::Select(..) => "select",
        BridgeCommand::Refresh(..) => "refresh",
        BridgeCommand::Control { .. } => "control",
        BridgeCommand::ReadSourceCatalog(..) => "read_source_catalog",
        BridgeCommand::ReadQuickSelectNames(..) => "read_quick_select_names",
        BridgeCommand::ReadHttpInformation(..) => "read_http_information",
        BridgeCommand::Shutdown => "shutdown",
    }
}
