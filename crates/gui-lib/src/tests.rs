//! Unit tests for the `Gui` state: its reducer, its bridge plumbing, and its views.

#[cfg(target_os = "macos")]
use super::keyboard::is_close_window_shortcut;
use super::keyboard::{tab_direction, TabDirection};
use super::session::shut_down_within;
use super::setup::receiver_entry_for_discovered;
use super::*;
use denon_avr_application::ports::{
    AsyncConfigRepository, AsyncReceiverDiscovery, BoxFuture, OperationError, OperationErrorKind,
    ReceiverConnector,
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
    gui_with_shutdown(Arc::new(|| Box::pin(async {})))
}

fn gui_with_shutdown(shutdown: ShutdownHook) -> Gui {
    let service = Arc::new(ControlService::new(
        Arc::new(Unreachable),
        Arc::new(Empty(Mutex::new(ConfiguredReceivers::default()))),
        Arc::new(Nobody),
        ServiceConfig::default(),
    ));
    Gui::new(GuiServices {
        control: service.operator(),
        shutdown,
    })
}

/// Every message a task produces, run to its end.
async fn outputs(task: Task<Message>) -> Vec<Message> {
    use iced::futures::StreamExt;
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

/// A shutdown hook that counts how often it ran.
fn counting_hook() -> (ShutdownHook, Arc<std::sync::atomic::AtomicUsize>) {
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hook: ShutdownHook = {
        let count = Arc::clone(&count);
        Arc::new(move || {
            let count = Arc::clone(&count);
            Box::pin(async move {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            })
        })
    };
    (hook, count)
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
async fn closing_the_window_closes_the_receiver_connection_first_and_once() {
    let (hook, count) = counting_hook();
    let mut gui = gui_with_shutdown(hook);
    let window = iced::window::Id::unique();

    let task = gui.update(Message::CloseRequested(window));
    assert!(gui.closing);
    assert_eq!(gui.announcement, "Closing the receiver connection…");
    // The window is only told to close once the connection has closed.
    let produced = outputs(task).await;
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(
        matches!(produced.as_slice(), [Message::ShutdownFinished(Some(id))] if *id == window),
        "{produced:?}"
    );
}

#[tokio::test]
async fn a_second_close_request_does_not_wait_for_the_connection_again() {
    let (hook, count) = counting_hook();
    let mut gui = gui_with_shutdown(hook);
    let window = iced::window::Id::unique();
    let _first = gui.update(Message::CloseRequested(window));

    let second = gui.update(Message::CloseRequested(window));
    let produced = outputs(second).await;
    assert!(
        produced.is_empty(),
        "it closes the window and waits for nothing"
    );
    assert_eq!(
        count.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the first request's task has not run here, and the second adds none"
    );
}

#[tokio::test]
async fn closing_gives_up_waiting_for_a_connection_that_never_closes() {
    let hook: ShutdownHook = Arc::new(|| Box::pin(std::future::pending()));
    let started = std::time::Instant::now();
    shut_down_within(Duration::from_millis(50), hook).await;
    let waited = started.elapsed();
    assert!(waited >= Duration::from_millis(50), "{waited:?}");
    assert!(waited < Duration::from_secs(2), "{waited:?}");
}

#[tokio::test]
async fn a_closing_window_ignores_what_the_closing_connection_reports() {
    let mut gui = selected_gui();
    let _ = gui.update(state_message(&gui, state_at(Some(1), powered_on())));
    let _ = gui.update(Message::CloseRequested(iced::window::Id::unique()));

    // Closing the connection ends the subscription, and subscribing again
    // fails; neither is news for a window that is going away.
    let _ = gui.update(event(&gui, 1, PortEvent::SessionEnded));
    let _ = gui.update(event(
        &gui,
        1,
        PortEvent::ConnectFailed("service shut down".into()),
    ));

    assert_eq!(gui.lifecycle, Lifecycle::Connected { generation: 1 });
    assert_eq!(gui.snapshot.power.value(), Some(&PowerState::On));
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
        gui.sound_mode.category_filter,
        Some(SoundModeCategory::Pure)
    );
    assert_eq!(gui.sound_mode.request_id, None);

    let _ = gui.update(Message::SelectSoundModeCategory(SoundModeCategory::Movie));
    assert_eq!(
        gui.sound_mode.category_filter,
        Some(SoundModeCategory::Movie)
    );
    assert!(gui.sound_mode.request_id.is_some());
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
    gui.launch.status_wait_ticks = 99;
    gui.volume.command_pending = true;

    // The channel coalesced the disconnect away.
    let _ = gui.update(state_message(&gui, state_at(Some(3), powered_on())));

    assert_eq!(gui.lifecycle, Lifecycle::Connected { generation: 3 });
    assert_eq!(gui.generation, 3);
    assert!(
        !gui.volume.command_pending,
        "a command from the old connection ended"
    );
    assert_eq!(gui.launch.status_wait_ticks, 0);
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
    assert!(gui.supplemental.http_read_in_flight);
    assert!(!gui.supplemental.http_read_again);

    let _ = gui.request_http_information();
    assert!(
        gui.supplemental.http_read_again,
        "a second request waits its turn"
    );

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
    assert!(
        gui.supplemental.http_read_in_flight,
        "the waiting request started"
    );
    assert!(!gui.supplemental.http_read_again);
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
    assert!(gui.supplemental.catalog_read_in_flight);
    assert_eq!(gui.supplemental.catalog_auto_generation, Some(1));

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
    assert!(!gui.supplemental.catalog_read_in_flight);
    let _ = gui.update(state_message(&gui, state_at(Some(1), powered_on())));
    assert!(
        !gui.supplemental.catalog_read_in_flight,
        "not asked a second time"
    );
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
    gui.volume.command_pending = true;

    let _ = gui.update(event(&gui, 1, PortEvent::Failed("setting volume".into())));

    assert!(!gui.volume.command_pending);
    assert!(gui.announcement.contains("setting volume"));
}

#[tokio::test]
async fn unknown_volume_steps_initialize_to_minimum_once_then_adjust() {
    let mut gui = selected_gui();
    assert_eq!(gui.volume_slider, MIN_VOLUME_DB);

    let _ = gui.update(Message::AdjustVolume(0.5));

    assert_eq!(gui.volume_slider, MIN_VOLUME_DB);
    assert!(gui.volume.command_pending);
    assert!(gui.volume.baseline_initialized);

    let request_id = gui.volume.command_request_id.expect("volume was submitted");
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
    assert!(gui.volume.command_pending);
}

#[tokio::test]
async fn volume_step_respects_the_receiver_ceiling_and_blocks_duplicates() {
    let mut gui = selected_gui();
    gui.snapshot.set_value(
        MainZoneValue::Volume(Volume::from_parts("95", 150)),
        StateAuthority::Authoritative,
    );
    gui.volume.baseline_initialized = true;
    gui.volume_slider = 15.0;

    let _ = gui.update(Message::AdjustVolume(10.0));

    assert_eq!(gui.volume_slider, MAX_VOLUME_DB);
    assert_eq!(gui.volume.value, Some(MAX_VOLUME_DB));
    assert!(gui.volume.command_pending);

    let _ = gui.update(Message::AdjustVolume(-0.5));
    assert_eq!(gui.volume_slider, MAX_VOLUME_DB);

    let current_request = gui.volume.value_request_id;
    let _ = gui.update(Message::HideVolumeValue(current_request.saturating_sub(1)));
    assert_eq!(gui.volume.value, Some(MAX_VOLUME_DB));
    let _ = gui.update(Message::HideVolumeValue(current_request));
    assert_eq!(gui.volume.value, None);
}

#[tokio::test]
async fn a_volume_confirmation_reenables_the_slider_whatever_the_request_counter_says() {
    let mut gui = selected_gui();
    gui.volume.command_pending = true;
    gui.volume.command_request_id = Some(4);
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

    assert!(!gui.volume.command_pending);
    assert_eq!(gui.volume.command_request_id, None);
    assert_eq!(gui.announcement, "Command confirmed by the receiver.");
}

#[tokio::test]
async fn a_sound_mode_confirmation_reenables_mode_controls() {
    let mut gui = selected_gui();
    gui.sound_mode.request_id = Some(4);
    gui.request_id = 5;

    let _ = gui.update(event(&gui, 4, PortEvent::Control(ControlReport::Conflict)));

    assert_eq!(gui.sound_mode.request_id, None);
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
    gui.sound_mode.request_id = Some(4);

    let _ = gui.update(Message::SoundModeControlTimedOut(3));
    assert_eq!(gui.sound_mode.request_id, Some(4));

    let _ = gui.update(Message::SoundModeControlTimedOut(4));
    assert_eq!(gui.sound_mode.request_id, None);
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
    // The top of the slider is the receiver's maximum, +18.0 dB.
    assert_eq!(
        volume_level_for_slider(MAX_VOLUME_DB)
            .unwrap()
            .to_native_code(),
        980
    );
    for above in [18.5, 19.0] {
        assert_eq!(
            volume_level_for_slider(above).unwrap_err(),
            "Volume must be between -80.0 and +18.0 dB."
        );
    }
}

#[test]
fn volume_slider_uses_the_receiver_decibel_scale() {
    let mut snapshot = MainZoneSnapshot::default();
    snapshot.set_value(
        denon_avr_domain::MainZoneValue::Volume(denon_avr_domain::Volume::from_parts("245", -555)),
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
