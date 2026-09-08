use denon_avr_application::controller::SessionEvent;
use denon_avr_application::{
    AsyncConfigRepository, AsyncReceiverDiscovery, BoxFuture, OperationError, ReceiverSession,
    SessionFactory,
};
use denon_avr_domain::{
    AudioContextSnapshot, ConfiguredReceivers, DiscoveredReceiver, Input, MainZoneControl,
    MainZoneField, MainZoneValue, MuteState, PowerState, ReceiverEndpoint, ReceiverIdentity,
    SurroundMode, Volume,
};
use denon_avr_gui_lib::{boot_with_services, update, ControllerBridge, Gui, GuiServices, Message};
use iced::futures::StreamExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

#[derive(Default)]
struct Calls {
    configuration_loads: AtomicUsize,
    configuration_saves: AtomicUsize,
    discoveries: AtomicUsize,
    connections: AtomicUsize,
    audio_context_queries: AtomicUsize,
    core_refreshes: AtomicUsize,
    closes: AtomicUsize,
    discovery_timeouts: Mutex<Vec<Duration>>,
    saved_configurations: Mutex<Vec<ConfiguredReceivers>>,
    connected_identities: Mutex<Vec<ReceiverIdentity>>,
    queried_fields: Mutex<Vec<MainZoneField>>,
    refresh_finished: Notify,
    close_finished: Notify,
}

struct FakeConfiguration {
    calls: Arc<Calls>,
}

impl AsyncConfigRepository for FakeConfiguration {
    fn load(&self) -> BoxFuture<'_, Result<ConfiguredReceivers, OperationError>> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            self.calls
                .configuration_loads
                .fetch_add(1, Ordering::SeqCst);
            Ok(ConfiguredReceivers::default())
        })
    }

    fn save<'a>(
        &'a self,
        configuration: &'a ConfiguredReceivers,
    ) -> BoxFuture<'a, Result<(), OperationError>> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            self.calls
                .configuration_saves
                .fetch_add(1, Ordering::SeqCst);
            self.calls
                .saved_configurations
                .lock()
                .expect("saved configuration lock")
                .push(configuration.clone());
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
        timeout: Duration,
    ) -> BoxFuture<'_, Result<Vec<DiscoveredReceiver>, OperationError>> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            self.calls.discoveries.fetch_add(1, Ordering::SeqCst);
            self.calls
                .discovery_timeouts
                .lock()
                .expect("discovery timeout lock")
                .push(timeout);
            Ok(vec![self.receiver.clone()])
        })
    }
}

struct FakeFactory {
    calls: Arc<Calls>,
}

impl SessionFactory for FakeFactory {
    fn connect(
        &self,
        identity: ReceiverIdentity,
    ) -> BoxFuture<'_, Result<Box<dyn ReceiverSession>, OperationError>> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            self.calls.connections.fetch_add(1, Ordering::SeqCst);
            self.calls
                .connected_identities
                .lock()
                .expect("connected identity lock")
                .push(identity);
            Ok(Box::new(FakeSession {
                calls: Arc::clone(&self.calls),
            }) as Box<dyn ReceiverSession>)
        })
    }
}

struct FakeSession {
    calls: Arc<Calls>,
}

impl ReceiverSession for FakeSession {
    fn query_field(
        &mut self,
        field: MainZoneField,
    ) -> BoxFuture<'_, Result<MainZoneValue, OperationError>> {
        self.calls
            .queried_fields
            .lock()
            .expect("queried fields lock")
            .push(field);
        let calls = Arc::clone(&self.calls);
        Box::pin(async move {
            tokio::task::yield_now().await;
            let value = match field {
                MainZoneField::Power => MainZoneValue::Power(PowerState::On),
                MainZoneField::Input => MainZoneValue::Input(Input::new("CD").unwrap()),
                MainZoneField::Volume => MainZoneValue::Volume(Volume::from_parts("50", -300)),
                MainZoneField::Mute => MainZoneValue::Mute(MuteState::Off),
                MainZoneField::SurroundMode => {
                    MainZoneValue::SurroundMode(SurroundMode::new("STEREO").unwrap())
                }
            };
            if field == MainZoneField::SurroundMode {
                calls.core_refreshes.fetch_add(1, Ordering::SeqCst);
                calls.refresh_finished.notify_one();
            }
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
        Box::pin(async move {
            tokio::task::yield_now().await;
            self.calls
                .audio_context_queries
                .fetch_add(1, Ordering::SeqCst);
            self.calls.refresh_finished.notify_one();
            AudioContextSnapshot::default()
        })
    }

    fn next_event(&mut self) -> BoxFuture<'_, Result<SessionEvent, OperationError>> {
        Box::pin(std::future::pending())
    }

    fn close(&mut self) -> BoxFuture<'_, Result<(), OperationError>> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            self.calls.closes.fetch_add(1, Ordering::SeqCst);
            self.calls.close_finished.notify_one();
            Ok(())
        })
    }
}

async fn task_message(task: iced::Task<Message>) -> Message {
    let mut actions = iced_runtime::task::into_stream(task).expect("task should produce a message");
    while let Some(action) = actions.next().await {
        if let iced_runtime::Action::Output(message) = action {
            return message;
        }
    }
    panic!("task finished without producing a message");
}

async fn wait_for(counter: &AtomicUsize, notification: &Notify) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while counter.load(Ordering::SeqCst) == 0 {
            notification.notified().await;
        }
    })
    .await
    .expect("async GUI operation should finish");
}

fn services(calls: &Arc<Calls>, receiver: DiscoveredReceiver) -> GuiServices {
    GuiServices {
        factory: Arc::new(FakeFactory {
            calls: Arc::clone(calls),
        }),
        configuration: Arc::new(FakeConfiguration {
            calls: Arc::clone(calls),
        }),
        discovery: Arc::new(FakeDiscovery {
            calls: Arc::clone(calls),
            receiver,
        }),
    }
}

#[tokio::test]
async fn gui_drives_async_configuration_discovery_and_session_apis() {
    let calls = Arc::new(Calls::default());
    let discovered = DiscoveredReceiver {
        address: ReceiverEndpoint {
            host: "192.0.2.10".into(),
            port: 23,
        },
        location: None,
        server: Some("Denon test receiver".into()),
        model: Some("AVR-X3800H".into()),
        search_target: None,
        unique_service_name: None,
    };
    let services = services(&calls, discovered.clone());

    let (mut gui, load_task) = boot_with_services(services);
    let no_follow_up = update(&mut gui, task_message(load_task).await);
    assert_eq!(no_follow_up.units(), 0);
    assert_eq!(calls.configuration_loads.load(Ordering::SeqCst), 1);

    let discovery_task = update(&mut gui, Message::Discover);
    let save_task = update(&mut gui, task_message(discovery_task).await);
    let connect_task = update(&mut gui, task_message(save_task).await);
    assert!(matches!(
        task_message(connect_task).await,
        Message::CommandFinished(Ok(()))
    ));

    wait_for(&calls.core_refreshes, &calls.refresh_finished).await;

    assert_eq!(calls.discoveries.load(Ordering::SeqCst), 1);
    assert_eq!(calls.configuration_saves.load(Ordering::SeqCst), 1);
    assert_eq!(calls.connections.load(Ordering::SeqCst), 1);
    assert_eq!(
        *calls
            .discovery_timeouts
            .lock()
            .expect("discovery timeout lock"),
        vec![Duration::from_secs(3)]
    );
    assert_eq!(
        *calls.queried_fields.lock().expect("queried fields lock"),
        MainZoneField::ALL
    );
    assert_eq!(calls.audio_context_queries.load(Ordering::SeqCst), 0);
    assert_eq!(
        calls
            .connected_identities
            .lock()
            .expect("connected identity lock")[0]
            .host,
        discovered.address.host
    );

    let saved_configuration = {
        let saved = calls
            .saved_configurations
            .lock()
            .expect("saved configuration lock");
        assert_eq!(saved.len(), 1);
        saved[0].clone()
    };
    assert_eq!(saved_configuration, gui.configured);
    assert_eq!(
        gui.selection.as_ref().map(|selection| selection.identity()),
        saved_configuration
            .current()
            .map(|(_, identity)| identity.clone())
    );

    let shutdown_task = update(&mut gui, Message::Shutdown);
    assert!(matches!(
        task_message(shutdown_task).await,
        Message::CommandFinished(Ok(()))
    ));
    wait_for(&calls.closes, &calls.close_finished).await;
}

#[tokio::test]
async fn manual_setup_persists_before_connecting() {
    let calls = Arc::new(Calls::default());
    let discovered = DiscoveredReceiver {
        address: ReceiverEndpoint {
            host: "192.0.2.11".into(),
            port: 23,
        },
        location: None,
        server: None,
        model: None,
        search_target: None,
        unique_service_name: None,
    };
    let (mut gui, load_task) = boot_with_services(services(&calls, discovered));
    let _ = update(&mut gui, task_message(load_task).await);
    let _ = update(
        &mut gui,
        Message::AddressChanged("  receiver.example  ".into()),
    );
    let _ = update(&mut gui, Message::NameChanged("Living room".into()));

    let saved_task = update(&mut gui, Message::ManualSetup);
    let saved_message = task_message(saved_task).await;
    let (saved_config, saved_selection) = match saved_message {
        Message::ManualSaved(Ok(value)) => value,
        other => panic!("unexpected manual save result: {other:?}"),
    };
    assert_eq!(calls.configuration_saves.load(Ordering::SeqCst), 1);
    assert_eq!(saved_config.current.as_deref(), Some("Living room"));
    assert_eq!(saved_selection.identity().host, "receiver.example");

    let connect_task = update(
        &mut gui,
        Message::ManualSaved(Ok((saved_config.clone(), saved_selection))),
    );
    assert!(matches!(
        task_message(connect_task).await,
        Message::CommandFinished(Ok(()))
    ));
    wait_for(&calls.core_refreshes, &calls.refresh_finished).await;
    assert_eq!(calls.audio_context_queries.load(Ordering::SeqCst), 0);
    assert_eq!(
        calls.connected_identities.lock().expect("identity lock")[0].host,
        "receiver.example"
    );
    assert_eq!(gui.configured, saved_config);

    let shutdown_task = update(&mut gui, Message::Shutdown);
    assert!(matches!(
        task_message(shutdown_task).await,
        Message::CommandFinished(Ok(()))
    ));
    wait_for(&calls.closes, &calls.close_finished).await;
}

#[tokio::test]
async fn saved_receiver_startup_connects_and_refreshes_only_core_status() {
    let calls = Arc::new(Calls::default());
    let bridge = ControllerBridge::new(FakeFactory {
        calls: Arc::clone(&calls),
    });
    let mut gui = Gui::new(bridge);
    let identity = ReceiverIdentity {
        host: "192.168.0.8".into(),
        model: Some("Denon AVC-X3800H".into()),
        friendly_name: Some("Denon AVC-X3800H".into()),
    };
    let config = ConfiguredReceivers {
        current: Some("Denon AVC-X3800H".into()),
        receivers: [("Denon AVC-X3800H".into(), identity.clone())].into(),
    };

    let startup_task = update(&mut gui, Message::ConfigLoaded(Ok(config)));
    assert!(matches!(
        task_message(startup_task).await,
        Message::CommandFinished(Ok(()))
    ));
    wait_for(&calls.core_refreshes, &calls.refresh_finished).await;

    assert_eq!(calls.connections.load(Ordering::SeqCst), 1);
    assert_eq!(
        calls
            .connected_identities
            .lock()
            .expect("identity lock")
            .as_slice(),
        &[identity]
    );
    assert_eq!(
        *calls.queried_fields.lock().expect("queried fields lock"),
        MainZoneField::ALL
    );
    assert_eq!(calls.audio_context_queries.load(Ordering::SeqCst), 0);

    let shutdown_task = update(&mut gui, Message::Shutdown);
    assert!(matches!(
        task_message(shutdown_task).await,
        Message::CommandFinished(Ok(()))
    ));
    wait_for(&calls.closes, &calls.close_finished).await;
}
