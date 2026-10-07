//! The GUI against the real control service and a fake receiver.
//!
//! Nothing here touches a socket. A fake connector hands the service a fake
//! canonical session, and a headless driver feeds the bridge's events back to the
//! reducer as messages, as the Iced subscription does in the application.

use denon_avr_application::ports::{
    AsyncConfigRepository, AsyncReceiverDiscovery, BoxFuture, OperationError, OperationErrorKind,
    ReceiverConnector,
};
use denon_avr_application::{
    CanonicalReceiverSession, ControlService, OperationRequest, Readiness, ServiceConfig,
    SharedReceiverSession, StateSubscription,
};
use denon_avr_domain::{
    ConfiguredReceivers, CoreFrame, DiscoveredReceiver, DispatchCertainty, Epoch, FrameSeq,
    MasterVolume, MonotonicMillis, MuteState, ObservationOrigin, OperationOutcome, PowerState,
    ReceiverEndpoint, ReceiverId, ReceiverIdentity, ReceiverIntent, ReceiverState, SoundModeStatus,
    SourceId, SystemPower, ZonePower,
};
use denon_avr_gui_lib::{
    boot_with_services, update, BridgeEvent, Gui, GuiServices, Lifecycle, Message, Selection,
};
use iced::futures::StreamExt;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;

#[derive(Default)]
struct Calls {
    loads: AtomicUsize,
    saves: AtomicUsize,
    discoveries: AtomicUsize,
    connects: AtomicUsize,
    closes: AtomicUsize,
    synchronizes: AtomicUsize,
    hosts: Mutex<Vec<String>>,
    operations: Mutex<Vec<ReceiverIntent>>,
    sessions: Mutex<Vec<Arc<FakeSession>>>,
    /// When set, the next connection attempts fail.
    refuse: AtomicBool,
}

struct FakeSession {
    receiver: ReceiverId,
    states: watch::Sender<ReceiverState>,
    calls: Arc<Calls>,
    seq: AtomicUsize,
}

impl FakeSession {
    fn frame(&self, epoch: u64, frame: CoreFrame) {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) as u64 + 1;
        self.states.send_modify(|state| {
            state.reduce(
                &self.receiver,
                Epoch(epoch),
                FrameSeq(seq),
                MonotonicMillis(seq),
                MonotonicMillis(u64::MAX / 2),
                ObservationOrigin::ReceiverFrame,
                frame,
            );
        });
    }

    fn read_everything(&self) {
        let epoch = self.states.borrow().epoch.map_or(1, |epoch| epoch.0);
        for frame in [
            CoreFrame::SystemPower(SystemPower::On),
            CoreFrame::MainZonePower(ZonePower::On),
            CoreFrame::Zone2Power(ZonePower::Off),
            CoreFrame::Source(SourceId::new("CD").unwrap()),
            CoreFrame::Volume(MasterVolume::db_half_steps(-60).unwrap()),
            CoreFrame::Mute(MuteState::Off),
            CoreFrame::SoundMode(SoundModeStatus {
                id: "STEREO".into(),
                raw: "MSSTEREO".into(),
            }),
        ] {
            self.frame(epoch, frame);
        }
    }

    /// The receiver drops the connection and the transport reconnects.
    fn reconnect(&self, epoch: u64) {
        self.states.send_modify(|state| {
            state.mark_disconnected();
            state.establish_epoch(Epoch(epoch));
        });
        self.read_everything();
    }

    fn the_receiver_changes_volume_by_itself(&self, half_steps: i16) {
        let epoch = self.states.borrow().epoch.map_or(1, |epoch| epoch.0);
        self.frame(
            epoch,
            CoreFrame::Volume(MasterVolume::db_half_steps(half_steps).unwrap()),
        );
    }
}

impl CanonicalReceiverSession for FakeSession {
    fn state(&self) -> StateSubscription {
        StateSubscription::new(self.states.subscribe())
    }

    fn synchronize(&self) -> BoxFuture<'_, Result<Readiness, OperationError>> {
        Box::pin(async move {
            self.calls.synchronizes.fetch_add(1, Ordering::SeqCst);
            self.read_everything();
            Ok(Readiness {
                ready: true,
                degraded: false,
                detail: "fake".into(),
            })
        })
    }

    fn operate(&self, request: OperationRequest) -> BoxFuture<'_, OperationOutcome> {
        Box::pin(async move {
            self.calls
                .operations
                .lock()
                .unwrap()
                .push(request.intent.clone());
            let epoch = self.states.borrow().epoch.map_or(1, |epoch| epoch.0);
            let frame = match &request.intent {
                ReceiverIntent::Mute(mute) => Some(CoreFrame::Mute(*mute)),
                ReceiverIntent::Volume(volume) => Some(CoreFrame::Volume(*volume)),
                ReceiverIntent::MainZonePower(power) => Some(CoreFrame::MainZonePower(*power)),
                ReceiverIntent::Zone2Power(power) => Some(CoreFrame::Zone2Power(*power)),
                _ => None,
            };
            if let Some(frame) = frame {
                self.frame(epoch, frame);
            }
            OperationOutcome::ObservedRequestedValue {
                operation: request.id,
                dispatch: DispatchCertainty::CompleteWrite,
                observation: "fake".into(),
            }
        })
    }

    fn close(&self) -> BoxFuture<'_, Result<(), OperationError>> {
        Box::pin(async move {
            self.calls.closes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

struct FakeConnector {
    calls: Arc<Calls>,
}

impl ReceiverConnector for FakeConnector {
    fn connect<'a>(
        &'a self,
        receiver: &'a ReceiverId,
        identity: &'a ReceiverIdentity,
    ) -> BoxFuture<'a, Result<SharedReceiverSession, OperationError>> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            self.calls.connects.fetch_add(1, Ordering::SeqCst);
            self.calls.hosts.lock().unwrap().push(identity.host.clone());
            if self.calls.refuse.load(Ordering::SeqCst) {
                return Err(OperationError::new(
                    OperationErrorKind::Connection,
                    "connecting",
                    "refused",
                ));
            }
            let mut initial = ReceiverState::new(receiver.clone());
            initial.establish_epoch(Epoch(1));
            let session = Arc::new(FakeSession {
                receiver: receiver.clone(),
                states: watch::channel(initial).0,
                calls: Arc::clone(&self.calls),
                seq: AtomicUsize::new(0),
            });
            self.calls
                .sessions
                .lock()
                .unwrap()
                .push(Arc::clone(&session));
            Ok(session as SharedReceiverSession)
        })
    }
}

struct FakeConfiguration {
    calls: Arc<Calls>,
    stored: Mutex<ConfiguredReceivers>,
}

impl AsyncConfigRepository for FakeConfiguration {
    fn load(&self) -> BoxFuture<'_, Result<ConfiguredReceivers, OperationError>> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            self.calls.loads.fetch_add(1, Ordering::SeqCst);
            Ok(self.stored.lock().unwrap().clone())
        })
    }

    fn save<'a>(
        &'a self,
        configuration: &'a ConfiguredReceivers,
    ) -> BoxFuture<'a, Result<(), OperationError>> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            self.calls.saves.fetch_add(1, Ordering::SeqCst);
            *self.stored.lock().unwrap() = configuration.clone();
            Ok(())
        })
    }
}

struct FakeDiscovery {
    calls: Arc<Calls>,
    receiver: DiscoveredReceiver,
}

impl AsyncReceiverDiscovery for FakeDiscovery {
    fn discover(
        &self,
        _timeout: Duration,
    ) -> BoxFuture<'_, Result<Vec<DiscoveredReceiver>, OperationError>> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            self.calls.discoveries.fetch_add(1, Ordering::SeqCst);
            Ok(vec![self.receiver.clone()])
        })
    }
}

struct World {
    calls: Arc<Calls>,
    config: Arc<FakeConfiguration>,
    service: Arc<ControlService>,
}

impl World {
    fn new(stored: ConfiguredReceivers) -> Self {
        let calls = Arc::new(Calls::default());
        let config = Arc::new(FakeConfiguration {
            calls: Arc::clone(&calls),
            stored: Mutex::new(stored),
        });
        let service = Arc::new(ControlService::new(
            Arc::new(FakeConnector {
                calls: Arc::clone(&calls),
            }),
            config.clone(),
            Arc::new(FakeDiscovery {
                calls: Arc::clone(&calls),
                receiver: discovered("192.0.2.10"),
            }),
            ServiceConfig::default(),
        ));
        Self {
            calls,
            config,
            service,
        }
    }

    fn services(&self) -> GuiServices {
        let service = Arc::clone(&self.service);
        GuiServices {
            control: self.service.operator(),
            shutdown: Arc::new(move || {
                let service = Arc::clone(&service);
                Box::pin(async move { service.shutdown().await })
            }),
        }
    }

    fn session(&self, index: usize) -> Arc<FakeSession> {
        Arc::clone(&self.calls.sessions.lock().unwrap()[index])
    }

    fn stored(&self) -> ConfiguredReceivers {
        self.config.stored.lock().unwrap().clone()
    }
}

fn discovered(host: &str) -> DiscoveredReceiver {
    DiscoveredReceiver {
        address: ReceiverEndpoint {
            host: host.into(),
            port: 23,
        },
        location: None,
        server: Some("Denon test receiver".into()),
        model: Some("AVR-X3800H".into()),
        search_target: None,
        unique_service_name: None,
    }
}

fn identity(host: &str) -> ReceiverIdentity {
    ReceiverIdentity {
        host: host.into(),
        model: Some("AVR-X3800H".into()),
        friendly_name: None,
    }
}

fn saved(entries: &[(&str, &str)], current: &str) -> ConfiguredReceivers {
    ConfiguredReceivers {
        current: Some(current.into()),
        receivers: entries
            .iter()
            .map(|(name, host)| ((*name).to_owned(), identity(host)))
            .collect::<BTreeMap<_, _>>(),
        ..ConfiguredReceivers::default()
    }
}

/// Every message a task produces, run to its end.
async fn outputs(task: iced::Task<Message>) -> Vec<Message> {
    let Some(mut actions) = iced_runtime::task::into_stream(task) else {
        return Vec::new();
    };
    let mut messages = Vec::new();
    while let Some(action) = actions.next().await {
        if let iced_runtime::Action::Output(message) = action {
            messages.push(message);
        }
    }
    messages
}

/// Apply `message`, then each message the resulting tasks produce.
async fn drive(gui: &mut Gui, message: Message) {
    let mut queue = VecDeque::from([message]);
    while let Some(message) = queue.pop_front() {
        let task = update(gui, message);
        queue.extend(outputs(task).await);
    }
}

/// Feed the bridge's events to the reducer until `done` holds.
async fn until(gui: &mut Gui, what: &str, done: impl Fn(&Gui) -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !done(gui) {
            let event: BridgeEvent = gui.bridge().recv().await.expect("the bridge ended");
            drive(gui, Message::Bridge(Box::new(event))).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

fn connected(gui: &Gui) -> bool {
    matches!(gui.lifecycle, Lifecycle::Connected { .. }) && gui.snapshot.power.value().is_some()
}

#[tokio::test]
async fn gui_drives_configuration_discovery_and_the_session_through_the_port() {
    let world = World::new(ConfiguredReceivers::default());
    let (mut gui, load) = boot_with_services(world.services());
    for message in outputs(load).await {
        drive(&mut gui, message).await;
    }
    assert_eq!(world.calls.loads.load(Ordering::SeqCst), 1);
    assert!(gui.launch_ready, "no saved receiver opens receiver setup");

    drive(&mut gui, Message::Discover).await;
    // One receiver found is saved and chosen without another click.
    until(&mut gui, "the discovered receiver to connect", connected).await;

    assert_eq!(world.calls.discoveries.load(Ordering::SeqCst), 1);
    assert_eq!(world.calls.saves.load(Ordering::SeqCst), 1);
    assert_eq!(world.calls.connects.load(Ordering::SeqCst), 1);
    assert_eq!(*world.calls.hosts.lock().unwrap(), ["192.0.2.10"]);
    assert_eq!(world.stored(), gui.configured);
    assert_eq!(
        gui.selection.as_ref().map(|selection| selection.identity()),
        world
            .stored()
            .current()
            .map(|(_, identity)| identity.clone())
    );
    assert_eq!(gui.snapshot.power.value(), Some(&PowerState::On));
    assert_eq!(gui.snapshot.input.value().unwrap().as_str(), "CD");
    assert_eq!(gui.zone2.power.value(), Some(&PowerState::Standby));

    drive(&mut gui, Message::Shutdown).await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while world.calls.closes.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("shutdown closes the receiver connection");
}

#[tokio::test]
async fn a_saved_receiver_connects_at_launch_and_a_control_is_confirmed() {
    let world = World::new(saved(&[("living-room", "192.0.2.20")], "living-room"));
    let (mut gui, load) = boot_with_services(world.services());
    for message in outputs(load).await {
        drive(&mut gui, message).await;
    }
    until(&mut gui, "the launch connection", connected).await;
    assert!(gui.launch_ready);
    assert_eq!(*world.calls.hosts.lock().unwrap(), ["192.0.2.20"]);

    drive(&mut gui, Message::Mute).await;
    until(&mut gui, "the mute to be confirmed", |gui| {
        gui.announcement == "Command confirmed by the receiver."
    })
    .await;
    assert_eq!(
        *world.calls.operations.lock().unwrap(),
        [ReceiverIntent::Mute(MuteState::On)]
    );
    until(&mut gui, "the muted state", |gui| {
        gui.snapshot.mute.value() == Some(&MuteState::On)
    })
    .await;
}

#[tokio::test]
async fn manual_setup_keeps_other_receivers_and_favorites_and_connects() {
    let mut stored = saved(&[("den", "192.0.2.30")], "den");
    stored
        .toggle_current_sound_mode_favorite("STEREO")
        .expect("a favorite for the current receiver");
    let world = World::new(stored.clone());
    let (mut gui, load) = boot_with_services(world.services());
    for message in outputs(load).await {
        drive(&mut gui, message).await;
    }
    until(&mut gui, "the saved receiver", connected).await;

    drive(
        &mut gui,
        Message::AddressChanged("  receiver.example  ".into()),
    )
    .await;
    drive(&mut gui, Message::NameChanged("Living room".into())).await;
    drive(&mut gui, Message::ManualSetup).await;
    until(&mut gui, "the new receiver to connect", |gui| {
        connected(gui)
            && gui
                .selection
                .as_ref()
                .is_some_and(|selection| selection.name == "Living room")
    })
    .await;

    let after = world.stored();
    assert_eq!(after.current.as_deref(), Some("Living room"));
    assert!(
        after.receivers.contains_key("den"),
        "the other receiver is kept"
    );
    assert_eq!(after.receivers["Living room"].host, "receiver.example");
    assert_eq!(
        after.sound_mode_favorites, stored.sound_mode_favorites,
        "the favorites are kept"
    );
    assert_eq!(
        world.calls.hosts.lock().unwrap().last().unwrap(),
        "receiver.example"
    );
    assert_eq!(gui.configured, after);
}

#[tokio::test]
async fn saving_a_receiver_again_with_a_new_address_reconnects_at_the_new_address() {
    let world = World::new(saved(&[("living-room", "192.0.2.20")], "living-room"));
    let (mut gui, load) = boot_with_services(world.services());
    for message in outputs(load).await {
        drive(&mut gui, message).await;
    }
    until(&mut gui, "the first connection", connected).await;

    // DHCP moved the receiver; the user enters its new address under the old name.
    drive(&mut gui, Message::AddressChanged("192.0.2.99".into())).await;
    drive(&mut gui, Message::NameChanged("living-room".into())).await;
    drive(&mut gui, Message::ManualSetup).await;
    until(&mut gui, "the reconnection", |gui| {
        connected(gui)
            && gui
                .selection
                .as_ref()
                .is_some_and(|selection| selection.identity.host == "192.0.2.99")
    })
    .await;

    assert_eq!(
        *world.calls.hosts.lock().unwrap(),
        ["192.0.2.20", "192.0.2.99"]
    );
    assert_eq!(
        world.calls.closes.load(Ordering::SeqCst),
        1,
        "the old connection was closed, so only one is ever open"
    );
}

#[tokio::test]
async fn a_control_from_a_stale_display_is_refused_and_nothing_is_sent() {
    let world = World::new(saved(&[("living-room", "192.0.2.20")], "living-room"));
    let (mut gui, load) = boot_with_services(world.services());
    for message in outputs(load).await {
        drive(&mut gui, message).await;
    }
    until(&mut gui, "the connection", connected).await;

    // The receiver's volume changes by itself and the new state is on its way
    // to the window, but the user acts on what the window still shows.
    world.session(0).the_receiver_changes_volume_by_itself(-20);
    drive(&mut gui, Message::AdjustVolume(10.0)).await;
    until(&mut gui, "the refusal", |gui| {
        gui.announcement == "Receiver state changed before the command; retry."
    })
    .await;
    assert!(world.calls.operations.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_reconnect_recovers_from_the_sessions_own_read_and_the_gui_asks_for_none() {
    let world = World::new(saved(&[("living-room", "192.0.2.20")], "living-room"));
    let (mut gui, load) = boot_with_services(world.services());
    for message in outputs(load).await {
        drive(&mut gui, message).await;
    }
    until(&mut gui, "the connection", connected).await;
    let before = world.calls.synchronizes.load(Ordering::SeqCst);

    // The transport loses the receiver.
    world.session(0).states.send_modify(|state| {
        state.mark_disconnected();
    });
    until(&mut gui, "the reconnecting screen", |gui| {
        matches!(gui.lifecycle, Lifecycle::Reconnecting { .. })
    })
    .await;
    assert!(gui.snapshot.power.value().is_none());

    // It reconnects, and the session reads every field again, as the real one
    // does; the window follows the state.
    world.session(0).reconnect(2);
    until(&mut gui, "the reconnected state", |gui| {
        matches!(gui.lifecycle, Lifecycle::Connected { generation: 2 }) && connected(gui)
    })
    .await;
    assert_eq!(
        world.calls.synchronizes.load(Ordering::SeqCst),
        before,
        "the GUI leaves the re-read to the session"
    );

    // A reconnect seen only as a higher epoch is one too.
    world.session(0).reconnect(3);
    until(&mut gui, "a second reconnect", |gui| {
        matches!(gui.lifecycle, Lifecycle::Connected { generation: 3 })
    })
    .await;
}

#[tokio::test]
async fn an_unreachable_receiver_shows_the_unavailable_screen_and_retry_connects() {
    let world = World::new(saved(&[("living-room", "192.0.2.20")], "living-room"));
    world.calls.refuse.store(true, Ordering::SeqCst);
    let (mut gui, load) = boot_with_services(world.services());
    for message in outputs(load).await {
        drive(&mut gui, message).await;
    }
    until(&mut gui, "the failure", |gui| {
        gui.lifecycle == Lifecycle::Disconnected
    })
    .await;
    assert!(gui.launch_ready, "the unavailable screen, not the spinner");

    world.calls.refuse.store(false, Ordering::SeqCst);
    drive(&mut gui, Message::Refresh).await;
    until(&mut gui, "the retry to connect", connected).await;
    assert_eq!(world.calls.connects.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn selecting_another_receiver_moves_the_window_to_it() {
    let world = World::new(saved(
        &[("den", "192.0.2.30"), ("living-room", "192.0.2.20")],
        "living-room",
    ));
    let (mut gui, load) = boot_with_services(world.services());
    for message in outputs(load).await {
        drive(&mut gui, message).await;
    }
    until(&mut gui, "the first connection", connected).await;

    let den = Selection {
        name: "den".into(),
        identity: identity("192.0.2.30"),
    };
    drive(&mut gui, Message::Select(den)).await;
    until(&mut gui, "the second connection", |gui| {
        connected(gui)
            && gui
                .selection
                .as_ref()
                .is_some_and(|selection| selection.name == "den")
    })
    .await;
    // The first receiver speaking now must not change what the window shows.
    world.session(0).the_receiver_changes_volume_by_itself(10);
    tokio::time::sleep(Duration::from_millis(50)).await;
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(50), gui.bridge().recv()).await
    {
        drive(&mut gui, Message::Bridge(Box::new(event))).await;
    }
    assert_eq!(gui.snapshot.volume.value().unwrap().db_tenths(), -300);
    assert_eq!(world.calls.connects.load(Ordering::SeqCst), 2);
}

/// What a window needs to be true for its next click to be fair: when the
/// service reports that a control finished, the state that control produced has
/// already been delivered. Otherwise the next click is built on a stale display
/// and refused as a conflict.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_control_report_never_overtakes_the_state_it_produced() {
    let world = World::new(saved(&[("living-room", "192.0.2.20")], "living-room"));
    let (mut gui, load) = boot_with_services(world.services());
    for message in outputs(load).await {
        drive(&mut gui, message).await;
    }
    until(&mut gui, "the connection", connected).await;

    let mut seen_mute: Option<MuteState> = None;
    for round in 0..30 {
        let (message, wanted) = if round % 2 == 0 {
            (Message::Mute, MuteState::On)
        } else {
            (Message::Unmute, MuteState::Off)
        };
        drive(&mut gui, message).await;
        loop {
            let event = tokio::time::timeout(Duration::from_secs(5), gui.bridge().recv())
                .await
                .expect("the control finished")
                .expect("the bridge ended");
            let report = match &event.event {
                denon_avr_gui_lib::PortEvent::State(state) => {
                    seen_mute = state.main_zone.mute.last_good.as_ref().map(|o| o.value);
                    None
                }
                denon_avr_gui_lib::PortEvent::Control(report) => Some(report.clone()),
                _ => None,
            };
            if let Some(report) = &report {
                assert_eq!(
                    seen_mute,
                    Some(wanted),
                    "round {round}: the control finished before its state arrived: {report:?}"
                );
            }
            drive(&mut gui, Message::Bridge(Box::new(event))).await;
            if report.is_some() {
                break;
            }
        }
        assert!(
            !gui.announcement.contains("state changed"),
            "round {round}: a click on a current display was refused: {}",
            gui.announcement
        );
    }
}
