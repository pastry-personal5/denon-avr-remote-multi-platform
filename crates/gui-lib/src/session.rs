//! The receiver session as the window sees it: connecting, closing, and
//! taking in what the bridge reports.

use super::*;

/// How long closing the window waits for the receiver connection to close. An
/// operation in flight is waited for, and a receiver that never answers must not
/// keep the window open.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

/// Run the shutdown hook, giving up after `grace`.
pub(super) async fn shut_down_within(grace: Duration, hook: ShutdownHook) {
    if tokio::time::timeout(grace, hook()).await.is_err() {
        tracing::warn!(?grace, "closing the receiver connection took too long");
    }
}

impl Gui {
    /// Choose `selection` and ask the bridge to connect to it.
    pub(super) fn connect_to(&mut self, selection: &Selection) -> Task<Message> {
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
        self.supplemental.http_read_in_flight = false;
        self.supplemental.http_read_again = false;
        self.supplemental.catalog_read_in_flight = false;
        self.supplemental.names_read_in_flight = false;
        self.supplemental.catalog_auto_generation = None;
        self.command(BridgeCommand::Select(id, receiver))
    }

    /// Close the receiver connection, then tell the window to close when one is
    /// given.
    pub(super) fn shut_down(&self, window: Option<iced::window::Id>) -> Task<Message> {
        let hook = std::sync::Arc::clone(&self.shutdown);
        Task::perform(
            async move {
                shut_down_within(SHUTDOWN_GRACE, hook).await;
                window
            },
            Message::ShutdownFinished,
        )
    }

    fn reset_pending_controls(&mut self) {
        self.volume.clear_command();
        self.sound_mode.clear_request();
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
        self.launch.status_wait_ticks = 0;
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
                    self.launch.status_wait_ticks = 0;
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
            if changed || !self.volume.baseline_initialized {
                self.volume_slider = volume;
                self.volume.baseline_initialized = true;
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
            && self.supplemental.catalog_auto_generation != Some(self.generation)
        {
            // Once per connection: a receiver that cannot answer must not be
            // asked again at every state.
            self.supplemental.catalog_auto_generation = Some(self.generation);
            self.announce("Refreshing source list…");
            tasks.push(self.request_source_catalog());
        }
        Task::batch(tasks)
    }

    pub(super) fn on_bridge(&mut self, event: BridgeEvent) -> Task<Message> {
        // A receiver the user has since left no longer speaks for this window,
        // and neither does one whose connection the window is closing.
        if event.subscription != self.subscription || self.closing {
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
                self.supplemental.catalog_read_in_flight = false;
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
                self.supplemental.names_read_in_flight = false;
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
                self.supplemental.http_read_in_flight = false;
                let again = std::mem::take(&mut self.supplemental.http_read_again);
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
        if self.volume.command_request_id == Some(request_id) {
            self.volume.clear_command();
        }
        if self.sound_mode.request_id == Some(request_id) {
            self.sound_mode.request_id = None;
        }
        if matches!(report, ControlReport::Failed(_)) {
            self.sound_mode.category_filter = None;
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

    pub(super) fn config_loaded(&mut self, config: ConfiguredReceivers) -> Task<Message> {
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

    pub(super) fn config_load_failed(&mut self, error: String) -> Task<Message> {
        // A configuration error must still leave the app usable so
        // the user can add a receiver manually.
        self.launch_ready = true;
        self.announce(format!("Configuration unavailable: {error}"));
        Task::none()
    }

    pub(super) fn command_failed(&mut self, error: String) -> Task<Message> {
        self.volume.clear_command();
        self.sound_mode.clear_request();
        self.announce(format!("Operation failed: {error}"));
        Task::none()
    }

    pub(super) fn select(&mut self, selection: Selection) -> Task<Message> {
        self.selection = Some(selection.clone());
        self.route = Route::Dashboard;
        self.snapshot.invalidate();
        self.volume_slider = MIN_VOLUME_DB;
        self.volume.baseline_initialized = false;
        self.zone2.invalidate();
        self.launch.status_wait_ticks = 0;
        self.invalidate_supplemental();
        self.volume.clear_command();
        self.sound_mode.clear_request();
        self.status_confirmed_generation = None;
        self.announce(format!(
            "{} selected; connecting for confirmed status…",
            selection_label(&selection)
        ));
        self.connect_to(&selection)
    }

    pub(super) fn refresh(&mut self) -> Task<Message> {
        let id = self.next_request();
        if self.lifecycle == Lifecycle::Disconnected {
            self.lifecycle = Lifecycle::Connecting;
        }
        self.command(BridgeCommand::Refresh(id))
    }

    pub(super) fn http_tick(&mut self) -> Task<Message> {
        if self.state.is_some() && matches!(self.lifecycle, Lifecycle::Connected { .. }) {
            self.request_http_information()
        } else {
            Task::none()
        }
    }

    pub(super) fn launch_tick(&mut self) -> Task<Message> {
        if !self.launch_ready || self.status_waiting() || self.sound_mode.request_id.is_some() {
            self.launch.frame = self.launch.frame.wrapping_add(1);
        }
        self.launch.status_wait_ticks = self.launch.status_wait_ticks.saturating_add(1);
        Task::none()
    }

    pub(super) fn close_requested(&mut self, window: iced::window::Id) -> Task<Message> {
        if self.closing {
            // The receiver connection is already closing; a second
            // request means the user does not want to wait.
            return iced::window::close(window);
        }
        self.closing = true;
        self.announce("Closing the receiver connection…");
        self.shut_down(Some(window))
    }
}
