//! Deterministic capture scenarios and the window tasks a visual capture
//! needs: take a screenshot, and close the window when it is written.

use super::*;

impl Gui {
    pub(super) fn capture_destination(&self) -> Option<PathBuf> {
        let route = match self.route {
            Route::Dashboard => "dashboard",
            Route::Receivers => "receivers",
            Route::Settings => "settings",
            Route::Advanced => "advanced",
            Route::Diagnostics => "diagnostics",
        };
        let scenario = self.capture.scenario.as_deref().unwrap_or(route);
        self.capture.directory.as_ref().map(|directory| {
            directory
                .join(std::env::consts::OS)
                .join(format!("{scenario}-{}pct.png", self.text_scale))
        })
    }
    pub(super) fn configure_capture_scenario(&mut self, scenario: &str) -> Result<(), String> {
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
        self.capture.scenario = Some(scenario.into());
        Ok(())
    }

    pub(super) fn capture_visual(&mut self) -> Task<Message> {
        if self.capture_destination().is_none() {
            self.announce("Visual capture is disabled. Set DENON_AVR_CAPTURE_DIR to an explicit output directory.");
            return Task::none();
        }
        capture_window()
    }

    pub(super) fn screenshot_captured(
        &mut self,
        screenshot: iced::window::Screenshot,
    ) -> Task<Message> {
        let Some(destination) = self.capture_destination() else {
            self.announce("Discarded visual capture because DENON_AVR_CAPTURE_DIR is not set.");
            return Task::none();
        };
        Task::perform(
            async move { capture::write_png(screenshot, &destination) },
            Message::ScreenshotWritten,
        )
    }

    pub(super) fn screenshot_written(&mut self, path: PathBuf) -> Task<Message> {
        self.announce(format!("Wrote native PNG capture to {}.", path.display()));
        if self.capture.scenario.is_some() {
            close_latest_window()
        } else {
            Task::none()
        }
    }

    pub(super) fn screenshot_write_failed(&mut self, error: String) -> Task<Message> {
        self.announce(format!("Could not write native PNG capture: {error}"));
        Task::none()
    }
}

pub(super) fn capture_window() -> Task<Message> {
    iced::window::latest().then(|id| match id {
        Some(id) => iced::window::screenshot(id).map(Message::ScreenshotCaptured),
        None => Task::done(Message::ScreenshotWritten(Err(
            "could not capture: no application window exists".into(),
        ))),
    })
}

/// Ask for the newest window to close, which closes the receiver connection
/// first.
pub(super) fn request_close_latest_window() -> Task<Message> {
    iced::window::latest().then(|id| match id {
        Some(id) => Task::done(Message::CloseRequested(id)),
        None => Task::none(),
    })
}

pub(super) fn close_latest_window() -> Task<Message> {
    iced::window::latest().then(|id| match id {
        Some(id) => iced::window::close(id),
        None => Task::none(),
    })
}
