//! Serialized adapter between Iced subscriptions and the control-service port.
//!
//! One task owns the port handle and the state subscription for the selected
//! receiver. Commands from the GUI arrive in order. A control, a refresh, and a
//! supplemental read each run on a task of their own, so a slow receiver never
//! holds back state updates; their results come back as events tagged with the
//! subscription they were started under, and the GUI drops the ones that belong
//! to a receiver it has since left.
//!
//! Order matters in one place. A control's state is written before the control
//! is reported finished, and a window that is told the control finished may
//! accept the next click. So the bridge forwards any newer state before it
//! forwards a report, and a spawned task never sends to the GUI itself: it hands
//! its result to the loop, which decides the order.

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
}

/// A single serialized owner of the port handle.
#[derive(Clone)]
pub struct PortBridge {
    commands: mpsc::Sender<BridgeCommand>,
    events: Arc<Mutex<mpsc::Receiver<BridgeEvent>>>,
}

impl PortBridge {
    pub fn new(control: SharedOperatorControl) -> Self {
        let (commands, command_rx) = mpsc::channel(16);
        let (event_sender, event_rx) = mpsc::channel(32);
        tokio::spawn(run_bridge(control, command_rx, event_sender));
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
    /// A spawned task finished and has an event for the GUI.
    Done(BridgeEvent),
}

/// Which state the GUI has last been sent, so a newer one is sent first and the
/// same one is never sent twice.
type Forwarded = Option<(
    Option<denon_avr_domain::Epoch>,
    denon_avr_domain::StateRevision,
)>;

fn stamp(
    state: &ReceiverState,
) -> (
    Option<denon_avr_domain::Epoch>,
    denon_avr_domain::StateRevision,
) {
    (state.epoch, state.revision)
}

/// Where a spawned task leaves its result for the loop.
type Done = mpsc::UnboundedSender<BridgeEvent>;

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
    control: SharedOperatorControl,
    mut commands: mpsc::Receiver<BridgeCommand>,
    events: mpsc::Sender<BridgeEvent>,
) {
    let mut selected: Option<Selected> = None;
    let mut active: Option<StateSubscription> = None;
    let mut forwarded: Forwarded = None;
    let (done, mut finished) = mpsc::unbounded_channel::<BridgeEvent>();
    loop {
        let step = tokio::select! {
            command = commands.recv() => Step::Command(command),
            next = next_state(&mut active) => Step::State(next),
            Some(event) = finished.recv() => Step::Done(event),
        };
        match step {
            Step::Command(None) => break,
            Step::Command(Some(command)) => {
                tracing::debug!(command = command_name(&command), "processing GUI command");
                let mut context = Context {
                    control: &control,
                    selected: &mut selected,
                    active: &mut active,
                    forwarded: &mut forwarded,
                    events: &events,
                    done: &done,
                };
                handle(command, &mut context).await;
            }
            Step::State(Ok(state)) => {
                if let Some(chosen) = &selected {
                    forward_state(&events, chosen, &mut forwarded, *state).await;
                }
            }
            Step::State(Err(error)) => {
                // The service closed the session: its entry changed, or it was
                // shut down. Subscribe again, which connects at the address now
                // in the file.
                tracing::info!(%error, "receiver session ended; subscribing again");
                active = None;
                forwarded = None;
                if let Some(chosen) = selected.clone() {
                    send(
                        &events,
                        chosen.subscription,
                        chosen.subscription,
                        PortEvent::SessionEnded,
                    )
                    .await;
                    active = subscribe(&control, &chosen, &events, &mut forwarded).await;
                }
            }
            Step::Done(event) => {
                // Whatever the task did to the receiver is in the state by now.
                // Send it first, so the report never reaches a window that has
                // not yet seen its effect.
                if let (Some(chosen), Some(subscription)) = (&selected, &active) {
                    let latest = subscription.latest();
                    if forwarded != Some(stamp(&latest)) {
                        forward_state(&events, chosen, &mut forwarded, latest).await;
                    }
                }
                let _ = events.send(event).await;
            }
        }
    }
}

/// Everything a command may need to read or change.
struct Context<'a> {
    control: &'a SharedOperatorControl,
    selected: &'a mut Option<Selected>,
    active: &'a mut Option<StateSubscription>,
    forwarded: &'a mut Forwarded,
    events: &'a mpsc::Sender<BridgeEvent>,
    done: &'a Done,
}

async fn forward_state(
    events: &mpsc::Sender<BridgeEvent>,
    chosen: &Selected,
    forwarded: &mut Forwarded,
    state: ReceiverState,
) {
    let stamp = stamp(&state);
    if *forwarded == Some(stamp) {
        return;
    }
    *forwarded = Some(stamp);
    send(
        events,
        chosen.subscription,
        chosen.subscription,
        PortEvent::State(Box::new(state)),
    )
    .await;
}

async fn handle(command: BridgeCommand, context: &mut Context<'_>) {
    let control = context.control;
    match command {
        BridgeCommand::Select(id, receiver) => {
            // Dropping the old subscription lets the service release the old
            // receiver after its idle time.
            *context.active = None;
            *context.forwarded = None;
            let chosen = Selected {
                receiver,
                subscription: id,
            };
            *context.selected = Some(chosen.clone());
            *context.active = subscribe(control, &chosen, context.events, context.forwarded).await;
        }
        BridgeCommand::Refresh(id) => {
            let Some(chosen) = context.selected.clone() else {
                send(
                    context.events,
                    id,
                    0,
                    PortEvent::Failed("no receiver selected".into()),
                )
                .await;
                return;
            };
            if context.active.is_none() {
                // The connection failed or ended: retrying is connecting again.
                *context.active =
                    subscribe(control, &chosen, context.events, context.forwarded).await;
                return;
            }
            let control = Arc::clone(control);
            let done = context.done.clone();
            tokio::spawn(async move {
                let event = match control.refresh(&chosen.receiver).await {
                    Ok(_) => PortEvent::Refreshed,
                    Err(error) => PortEvent::Failed(error.to_string()),
                };
                finish(&done, id, chosen.subscription, event);
            });
        }
        BridgeCommand::Control { id, intent, guard } => {
            let Some(chosen) = context.selected.clone() else {
                send(
                    context.events,
                    id,
                    0,
                    PortEvent::Failed("no receiver selected".into()),
                )
                .await;
                return;
            };
            if let (Some(baseline), Some(subscription)) = (&guard, context.active.as_ref()) {
                if !baseline.holds_in(&subscription.latest()) {
                    let report = PortEvent::Control(ControlReport::Conflict);
                    send(context.events, id, chosen.subscription, report).await;
                    return;
                }
            }
            let control = Arc::clone(control);
            let done = context.done.clone();
            tokio::spawn(async move {
                let report = run_control(&control, &chosen.receiver, intent).await;
                finish(&done, id, chosen.subscription, PortEvent::Control(report));
            });
        }
        BridgeCommand::ReadSourceCatalog(id, generation) => {
            spawn_read(context, id, move |control, receiver| {
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
            spawn_read(context, id, move |control, receiver| {
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
            spawn_read(context, id, move |control, receiver| {
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
    }
}

/// Leave a spawned task's result for the loop to forward in order.
fn finish(done: &Done, request_id: u64, subscription: u64, event: PortEvent) {
    let _ = done.send(BridgeEvent {
        request_id,
        subscription,
        event,
    });
}

fn spawn_read(
    context: &Context<'_>,
    id: u64,
    read: impl FnOnce(SharedOperatorControl, ReceiverId) -> BoxFuture<'static, PortEvent>
        + Send
        + 'static,
) {
    let Some(chosen) = context.selected.clone() else {
        return;
    };
    let control = Arc::clone(context.control);
    let done = context.done.clone();
    tokio::spawn(async move {
        let event = read(control, chosen.receiver).await;
        finish(&done, id, chosen.subscription, event);
    });
}

/// Subscribe to the chosen receiver, which connects and synchronizes it first.
async fn subscribe(
    control: &SharedOperatorControl,
    chosen: &Selected,
    events: &mpsc::Sender<BridgeEvent>,
    forwarded: &mut Forwarded,
) -> Option<StateSubscription> {
    match control.state(&chosen.receiver).await {
        Ok(subscription) => {
            *forwarded = None;
            forward_state(events, chosen, forwarded, subscription.latest()).await;
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use denon_avr_application::{
        OperationControl, OperationEvents, OperationStatus, OperatorAdmin, ReceiverReads,
        ReceiverSummary,
    };
    use denon_avr_domain::{
        ConfiguredReceivers, CoreFrame, DiscoveredReceiver, DispatchCertainty, Epoch, FrameSeq,
        HttpInformationSnapshot, MonotonicMillis, MuteState, ObservationOrigin, OperationId,
        QuickSelectNameObservation, ReceiverIdentity, SourceCatalogObservation,
    };
    use tokio::sync::watch;

    fn unavailable<T: Send + 'static>() -> BoxFuture<'static, Result<T, ControlError>> {
        Box::pin(async { Err(ControlError::Unavailable("not part of this test".into())) })
    }

    /// A port whose `submit` writes the new mute value into the state and
    /// resolves the operation in the same breath, as a fast receiver does. The
    /// bridge's state loop has had no chance to run in between.
    struct FastPort {
        states: watch::Sender<ReceiverState>,
    }

    impl FastPort {
        fn set_mute(&self, mute: MuteState, seq: u64) {
            let receiver = self.states.borrow().receiver.clone();
            self.states.send_modify(|state| {
                state.reduce(
                    &receiver,
                    Epoch(1),
                    FrameSeq(seq),
                    MonotonicMillis(seq),
                    MonotonicMillis(1_000_000),
                    ObservationOrigin::ReceiverFrame,
                    CoreFrame::Mute(mute),
                );
            });
        }
    }

    impl ReceiverReads for FastPort {
        fn receivers(&self) -> BoxFuture<'_, Result<Vec<ReceiverSummary>, ControlError>> {
            unavailable()
        }
        fn state<'a>(
            &'a self,
            _: &'a ReceiverId,
        ) -> BoxFuture<'a, Result<StateSubscription, ControlError>> {
            Box::pin(async move { Ok(StateSubscription::new(self.states.subscribe())) })
        }
        fn source_catalog<'a>(
            &'a self,
            _: &'a ReceiverId,
        ) -> BoxFuture<'a, Result<SourceCatalogObservation, ControlError>> {
            unavailable()
        }
        fn health(
            &self,
        ) -> BoxFuture<'_, Result<denon_avr_application::ServiceHealth, ControlError>> {
            unavailable()
        }
    }

    impl OperationControl for FastPort {
        fn submit<'a>(
            &'a self,
            receiver: &'a ReceiverId,
            submission: OperationSubmission,
        ) -> BoxFuture<'a, Result<OperationSnapshot, ControlError>> {
            Box::pin(async move {
                let ReceiverIntent::Mute(mute) = submission.intent.clone() else {
                    return Err(ControlError::Unavailable("mute only".into()));
                };
                let seq = self.states.borrow().revision.0 + 1;
                self.set_mute(mute, seq);
                Ok(OperationSnapshot {
                    id: OperationId(seq),
                    receiver: receiver.clone(),
                    intent: submission.intent,
                    status: OperationStatus::Completed,
                    dispatch: DispatchCertainty::CompleteWrite,
                    confirmed: true,
                    reason: None,
                    observation: None,
                })
            })
        }
        fn operation(
            &self,
            _: OperationId,
            _: Option<Duration>,
        ) -> BoxFuture<'_, Result<OperationSnapshot, ControlError>> {
            unavailable()
        }
        fn cancel(&self, _: OperationId) -> BoxFuture<'_, Result<OperationSnapshot, ControlError>> {
            unavailable()
        }
        fn operation_events(&self) -> BoxFuture<'_, Result<OperationEvents, ControlError>> {
            unavailable()
        }
        fn dry_run<'a>(
            &'a self,
            _: &'a ReceiverId,
            _: ReceiverIntent,
        ) -> BoxFuture<'a, Result<denon_avr_application::DryRun, ControlError>> {
            unavailable()
        }
    }

    impl OperatorAdmin for FastPort {
        fn discover(
            &self,
            _: Duration,
        ) -> BoxFuture<'_, Result<Vec<DiscoveredReceiver>, ControlError>> {
            unavailable()
        }
        fn register_ad_hoc(
            &self,
            _: ReceiverIdentity,
        ) -> BoxFuture<'_, Result<ReceiverId, ControlError>> {
            unavailable()
        }
        fn configuration(&self) -> BoxFuture<'_, Result<ConfiguredReceivers, ControlError>> {
            unavailable()
        }
        fn save_configuration<'a>(
            &'a self,
            _: &'a ConfiguredReceivers,
        ) -> BoxFuture<'a, Result<(), ControlError>> {
            unavailable()
        }
        fn quick_select_names<'a>(
            &'a self,
            _: &'a ReceiverId,
        ) -> BoxFuture<'a, Result<QuickSelectNameObservation, ControlError>> {
            unavailable()
        }
        fn http_information<'a>(
            &'a self,
            _: &'a ReceiverId,
        ) -> BoxFuture<'a, Result<HttpInformationSnapshot, ControlError>> {
            unavailable()
        }
        fn refresh<'a>(
            &'a self,
            _: &'a ReceiverId,
        ) -> BoxFuture<'a, Result<denon_avr_application::Readiness, ControlError>> {
            unavailable()
        }
        fn dry_run_as<'a>(
            &'a self,
            _: denon_avr_application::AgentLabel,
            _: &'a ReceiverId,
            _: ReceiverIntent,
        ) -> BoxFuture<'a, Result<denon_avr_application::DryRun, ControlError>> {
            unavailable()
        }
        fn policy(&self) -> BoxFuture<'_, Result<denon_avr_application::PolicyView, ControlError>> {
            unavailable()
        }
        fn reload_policy(
            &self,
        ) -> BoxFuture<'_, Result<denon_avr_application::PolicyView, ControlError>> {
            unavailable()
        }
        fn audit(
            &self,
            _: denon_avr_application::AuditQuery,
        ) -> BoxFuture<'_, Result<denon_avr_application::AuditPage, ControlError>> {
            unavailable()
        }
        fn issue_token(
            &self,
            _: denon_avr_application::AgentLabel,
        ) -> BoxFuture<'_, Result<denon_avr_application::IssuedToken, ControlError>> {
            unavailable()
        }
        fn tokens(
            &self,
        ) -> BoxFuture<'_, Result<Vec<denon_avr_application::TokenRecord>, ControlError>> {
            unavailable()
        }
        fn revoke_token(
            &self,
            _: denon_avr_application::TokenId,
        ) -> BoxFuture<'_, Result<denon_avr_application::TokenRecord, ControlError>> {
            unavailable()
        }
    }

    fn bridge_over_a_fast_port() -> PortBridge {
        let receiver = ReceiverId::new("living-room").unwrap();
        let mut initial = ReceiverState::new(receiver);
        initial.establish_epoch(Epoch(1));
        PortBridge::new(Arc::new(FastPort {
            states: watch::channel(initial).0,
        }))
    }

    fn mute_of(state: &ReceiverState) -> Option<MuteState> {
        state.main_zone.mute.last_good.as_ref().map(|o| o.value)
    }

    /// The report of a finished control arrives after the state it produced, in
    /// every interleaving the scheduler picks.
    #[tokio::test(flavor = "current_thread")]
    async fn a_control_report_never_overtakes_the_state_it_produced() {
        let bridge = bridge_over_a_fast_port();
        let room = ReceiverId::new("living-room").unwrap();
        bridge
            .send(BridgeCommand::Select(1, room.clone()))
            .await
            .unwrap();
        // The initial state.
        assert!(matches!(
            bridge.recv().await.unwrap().event,
            PortEvent::State(_)
        ));

        let mut seen: Option<MuteState> = None;
        for round in 0..200_u64 {
            let wanted = if round % 2 == 0 {
                MuteState::On
            } else {
                MuteState::Off
            };
            bridge
                .send(BridgeCommand::Control {
                    id: 10 + round,
                    intent: ReceiverIntent::Mute(wanted),
                    guard: None,
                })
                .await
                .unwrap();
            loop {
                let event = bridge.recv().await.unwrap();
                match event.event {
                    PortEvent::State(state) => seen = mute_of(&state),
                    PortEvent::Control(report) => {
                        assert_eq!(
                            seen,
                            Some(wanted),
                            "round {round}: {report:?} arrived before its state"
                        );
                        break;
                    }
                    other => panic!("unexpected event {other:?}"),
                }
            }
        }
    }
}
