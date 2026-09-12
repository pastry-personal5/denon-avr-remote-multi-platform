//! Serialized adapter between Iced subscriptions and the application controller.

use denon_avr_application::ports::AsyncReceiverDiscovery;
use denon_avr_application::ports::{AsyncConfigRepository, OperationError, SessionFactory};
use denon_avr_application::{
    ControllerHandle, ReceiverController, ReceiverEvent, ReceiverSelection,
};
use denon_avr_domain::{
    MainZoneControl, PowerState, QuickSelectEqCapabilities, QuickSelectSlot,
    SourceCatalogCapabilities,
};
use iced::futures::SinkExt;
use iced::Subscription;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeEvent {
    pub request_id: u64,
    pub generation: u64,
    pub event: ReceiverEvent,
}

#[derive(Debug, Clone)]
pub(crate) enum BridgeCommand {
    Select(u64, ReceiverSelection),
    Connect(u64),
    Refresh(u64),
    Disconnect(u64),
    Control(u64, MainZoneControl, Option<u64>),
    ControlZone2(u64, PowerState),
    RefreshQuickSelectEq(u64),
    RefreshSourceCatalog(u64),
    RecallQuickSelect(u64, QuickSelectSlot, Option<u64>),
    Shutdown(u64),
}

/// A single serialized owner of `ControllerHandle`.
#[derive(Clone)]
pub struct ControllerBridge {
    commands: mpsc::Sender<BridgeCommand>,
    events: Arc<Mutex<mpsc::Receiver<BridgeEvent>>>,
    pub(crate) validated_quick_select_eq: Option<QuickSelectEqCapabilities>,
    pub(crate) validated_source_catalog: Option<SourceCatalogCapabilities>,
}

/// Presentation-facing service ports supplied by the desktop composition root.
#[derive(Clone)]
pub struct GuiServices {
    pub factory: Arc<dyn SessionFactory>,
    pub configuration: Arc<dyn AsyncConfigRepository>,
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
        let validated_quick_select_eq = config.validated_quick_select_eq;
        let validated_source_catalog = config.validated_source_catalog;
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
            validated_quick_select_eq,
            validated_source_catalog,
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

    pub(crate) async fn send(&self, command: BridgeCommand) -> Result<(), String> {
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
    let mut handle = handle;
    let mut pending_request_id: Option<u64> = None;
    // Receiver state is authoritative within a connection generation, while
    // GUI request IDs also advance for unrelated work (for example, a source
    // catalog read). Keep the generation on every forwarded event so the GUI
    // can distinguish a late same-connection snapshot from old-receiver data.
    let mut connection_generation: u64 = 0;
    loop {
        tokio::select! {
            Some(command) = commands.recv() => {
                let is_shutdown = matches!(&command, BridgeCommand::Shutdown(_));
                tracing::debug!(command = command_name(&command), "processing GUI command");
                let (request_id, result) = match command {
                    BridgeCommand::Select(id, selection) => {
                        let selected = handle.select(selection).await;
                        if selected.is_ok() { (id, connect_and_refresh(&handle).await) } else { (id, selected) }
                    }
                    BridgeCommand::Connect(id) => (id, connect_and_refresh(&handle).await),
                    BridgeCommand::Refresh(id) => (id, handle.refresh().await),
                    BridgeCommand::Disconnect(id) => (id, handle.disconnect().await),
                    BridgeCommand::Control(id, control, version) => (id, handle.control(control, version).await),
                    BridgeCommand::ControlZone2(id, power) => (id, handle.control_zone2(power).await),
                    BridgeCommand::RefreshQuickSelectEq(id) => (id, handle.refresh_quick_select_eq().await),
                    BridgeCommand::RefreshSourceCatalog(id) => (id, handle.refresh_source_catalog().await),
                    BridgeCommand::RecallQuickSelect(id, slot, version) => (id, handle.recall_quick_select(slot, version).await),
                    BridgeCommand::Shutdown(id) => (id, handle.shutdown().await),
                };
                let event = result
                    .map(|reply| reply.unwrap_or(ReceiverEvent::Cancelled))
                    .unwrap_or_else(|error| ReceiverEvent::Diagnostic(
                        denon_avr_application::Diagnostic::Timeout { context: error.to_string() }
                    ));
                pending_request_id = Some(request_id);
                let mut reply_already_forwarded = false;
                while let Ok(Some(controller_event)) =
                    tokio::time::timeout(Duration::from_millis(1), handle.next_event()).await
                {
                    reply_already_forwarded |= controller_event == event;
                    let generation = bridge_event_generation(
                        &controller_event,
                        &mut connection_generation,
                    );
                    let _ = event_sender.send(BridgeEvent { request_id, generation, event: controller_event }).await;
                }
                if !reply_already_forwarded {
                    let generation = bridge_event_generation(&event, &mut connection_generation);
                    let _ = event_sender.send(BridgeEvent { request_id, generation, event }).await;
                }
                if is_shutdown {
                    tracing::info!("controller bridge stopped");
                    break;
                }
            }
            Some(controller_event) = handle.next_event() => {
                let generation = bridge_event_generation(&controller_event, &mut connection_generation);
                let request_id = pending_request_id.unwrap_or(0);
                let _ = event_sender.send(BridgeEvent { request_id, generation, event: controller_event }).await;
            }
        }
    }
}

fn bridge_event_generation(event: &ReceiverEvent, connection_generation: &mut u64) -> u64 {
    match event {
        ReceiverEvent::Lifecycle(denon_avr_application::Lifecycle::Connected { generation })
        | ReceiverEvent::Lifecycle(denon_avr_application::Lifecycle::Reconnecting { generation }) =>
        {
            *connection_generation = *generation;
            *generation
        }
        // A selection starts a new receiver context. Do not label any
        // selection-era event with the previous receiver's generation.
        ReceiverEvent::Lifecycle(denon_avr_application::Lifecycle::Selected) => {
            *connection_generation = 0;
            0
        }
        _ => *connection_generation,
    }
}

fn command_name(command: &BridgeCommand) -> &'static str {
    match command {
        BridgeCommand::Select(..) => "select",
        BridgeCommand::Connect(..) => "connect",
        BridgeCommand::Refresh(..) => "refresh",
        BridgeCommand::Disconnect(..) => "disconnect",
        BridgeCommand::Control(..) => "control",
        BridgeCommand::ControlZone2(..) => "control_zone2",
        BridgeCommand::RefreshQuickSelectEq(..) => "refresh_quick_select_eq",
        BridgeCommand::RefreshSourceCatalog(..) => "refresh_source_catalog",
        BridgeCommand::RecallQuickSelect(..) => "recall_quick_select",
        BridgeCommand::Shutdown(..) => "shutdown",
    }
}

struct TracingObservability;

impl denon_avr_application::Observability for TracingObservability {
    fn record(&self, diagnostic: denon_avr_application::Diagnostic) {
        match diagnostic {
            denon_avr_application::Diagnostic::ConnectionGeneration(generation) => {
                tracing::info!(generation, "receiver connection established")
            }
            denon_avr_application::Diagnostic::ReconnectAttempt { attempt } => {
                tracing::warn!(attempt, "receiver reconnecting")
            }
            denon_avr_application::Diagnostic::Timeout { context } => {
                tracing::warn!(%context, "receiver operation timed out")
            }
            denon_avr_application::Diagnostic::MalformedFrame { context } => {
                tracing::warn!(%context, "receiver returned a malformed frame")
            }
            denon_avr_application::Diagnostic::QueuePressure { queued } => {
                tracing::warn!(queued, "receiver command queue under pressure")
            }
            denon_avr_application::Diagnostic::Shutdown => {
                tracing::info!("receiver controller shut down")
            }
        }
    }
}

async fn connect_and_refresh(
    handle: &ControllerHandle,
) -> Result<Option<ReceiverEvent>, OperationError> {
    let connected = handle.connect().await;
    if connected.is_ok() {
        handle.refresh().await
    } else {
        connected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use denon_avr_application::Lifecycle;

    #[test]
    fn snapshots_keep_the_current_connection_generation() {
        let mut generation = 0;
        assert_eq!(
            bridge_event_generation(
                &ReceiverEvent::Lifecycle(Lifecycle::Connected { generation: 7 }),
                &mut generation,
            ),
            7
        );
        assert_eq!(
            bridge_event_generation(
                &ReceiverEvent::Snapshot(denon_avr_domain::MainZoneSnapshot::default()),
                &mut generation,
            ),
            7
        );
        assert_eq!(
            bridge_event_generation(
                &ReceiverEvent::Lifecycle(Lifecycle::Selected),
                &mut generation
            ),
            0
        );
    }
}
