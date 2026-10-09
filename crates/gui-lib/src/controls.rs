//! Receiver controls: turning a user's action into a guarded operation, and
//! the volume slider's local state while an operation is pending.

use super::*;

impl Gui {
    pub(super) fn control(&mut self, control: MainZoneControl) -> Task<Message> {
        let intent = match projection::intent_for(&control) {
            Ok(intent) => intent,
            Err(error) => {
                self.announce(error);
                return Task::none();
            }
        };
        let id = self.next_request();
        self.announce("Command pending; waiting for receiver confirmation…");
        self.submit(id, intent)
    }

    pub(super) fn control_zone2(&mut self, power: PowerState) -> Task<Message> {
        let id = self.next_request();
        self.submit(id, projection::zone2_power_intent(power))
    }

    /// Send an operation to the receiver. A control that depends on what the
    /// user saw, which is every one but a power control, carries the value the
    /// target field showed, so a change since then is reported and not overwritten.
    fn submit(&mut self, id: u64, intent: ReceiverIntent) -> Task<Message> {
        let guard = self.guard_for(&intent);
        self.command(BridgeCommand::Control { id, intent, guard })
    }

    /// Power controls state an absolute target, so an unrelated change must not
    /// stop them.
    pub(super) fn guard_for(&self, intent: &ReceiverIntent) -> Option<FieldBaseline> {
        match intent {
            ReceiverIntent::SystemPower(_)
            | ReceiverIntent::MainZonePower(_)
            | ReceiverIntent::Zone2Power(_) => None,
            _ => self
                .state
                .as_ref()
                .map(|state| FieldBaseline::capture(state, intent.field())),
        }
    }

    pub(super) fn control_sound_mode(&mut self, control: MainZoneControl) -> Task<Message> {
        const SOUND_MODE_CONTROL_TIMEOUT: Duration = Duration::from_secs(3);
        let intent = match projection::intent_for(&control) {
            Ok(intent) => intent,
            Err(error) => {
                self.announce(error);
                return Task::none();
            }
        };
        let id = self.next_request();
        self.sound_mode.request_id = Some(id);
        self.announce("Command pending; waiting for receiver confirmation…");
        let command = self.submit(id, intent);
        let timeout = Task::perform(
            async move {
                tokio::time::sleep(SOUND_MODE_CONTROL_TIMEOUT).await;
                id
            },
            Message::SoundModeControlTimedOut,
        );
        Task::batch([command, timeout])
    }

    pub(super) fn adjust_volume(&mut self, delta: f32) -> Task<Message> {
        if self.selection.is_none() || self.volume.command_pending {
            return Task::none();
        }
        // Until the AVR has supplied a canonical level, the first volume
        // button press establishes a safe minimum baseline. Apply later
        // button presses relative to that baseline, even if the receiver has
        // not yet returned a usable volume observation.
        if !self.volume.baseline_initialized {
            self.volume_slider = MIN_VOLUME_DB;
            self.volume.baseline_initialized = true;
            let minimum = volume_level_for_slider(MIN_VOLUME_DB)
                .expect("minimum volume is inside the validated receiver range");
            let value_task = self.show_volume_value();
            return Task::batch([self.submit_volume(minimum), value_task]);
        }
        let requested = self.volume_slider + delta;
        let target = requested.clamp(MIN_VOLUME_DB, MAX_VOLUME_DB);
        let changed = target != self.volume_slider;
        self.volume_slider = target;
        let value_task = changed.then(|| self.show_volume_value());
        let task = if changed {
            match volume_level_for_slider(target) {
                Ok(level) => self.submit_volume(level),
                Err(error) => {
                    self.announce(error);
                    Task::none()
                }
            }
        } else {
            Task::none()
        };
        match value_task {
            Some(value_task) => Task::batch([task, value_task]),
            None => task,
        }
    }

    pub(super) fn volume_is_interactive(&self) -> bool {
        self.selection.is_some()
            && self.snapshot.volume.value().is_some()
            && !self.volume.command_pending
    }

    pub(super) fn submit_volume(&mut self, level: denon_avr_domain::VolumeLevel) -> Task<Message> {
        self.volume.command_pending = true;
        let id = self.next_request();
        self.volume.command_request_id = Some(id);
        self.announce("Command pending; waiting for receiver confirmation…");
        self.submit(id, ReceiverIntent::Volume(projection::master_volume(level)))
    }

    pub(super) fn show_volume_value(&mut self) -> Task<Message> {
        self.volume.value_request_id = self.volume.value_request_id.saturating_add(1);
        self.volume.value = Some(self.volume_slider);
        let request_id = self.volume.value_request_id;
        Task::perform(
            async move {
                tokio::time::sleep(Duration::from_secs(3)).await;
                request_id
            },
            Message::HideVolumeValue,
        )
    }

    pub(super) fn toggle_main_zone_power(&mut self) -> Task<Message> {
        main_zone_power_control(self.snapshot.power.value())
            .map_or_else(Task::none, |control| self.control(control))
    }

    pub(super) fn toggle_zone2_power(&mut self) -> Task<Message> {
        let power = match self.zone2.power.value() {
            Some(PowerState::On) => PowerState::Standby,
            Some(PowerState::Standby) => PowerState::On,
            None => return Task::none(),
        };
        self.control_zone2(power)
    }

    pub(super) fn open_main_zone_power_popup(&mut self) -> Task<Message> {
        self.power_popup = self
            .snapshot
            .power
            .value()
            .is_some()
            .then_some(PowerPopup::MainZone);
        Task::none()
    }

    pub(super) fn open_zone2_power_popup(&mut self) -> Task<Message> {
        self.power_popup = self
            .zone2
            .power
            .value()
            .is_some()
            .then_some(PowerPopup::Zone2);
        Task::none()
    }

    pub(super) fn close_power_popup(&mut self) -> Task<Message> {
        self.power_popup = None;
        Task::none()
    }

    pub(super) fn set_main_zone_power(&mut self, power: PowerState) -> Task<Message> {
        self.power_popup = None;
        self.control(MainZoneControl::Power(power))
    }

    pub(super) fn set_zone2_power(&mut self, power: PowerState) -> Task<Message> {
        self.power_popup = None;
        self.control_zone2(power)
    }

    pub(super) fn unmute(&mut self) -> Task<Message> {
        self.control(MainZoneControl::Mute(denon_avr_domain::MuteState::Off))
    }

    pub(super) fn volume_changed(&mut self, value: f32) -> Task<Message> {
        if self.volume_is_interactive() {
            self.volume_slider = value.clamp(MIN_VOLUME_DB, MAX_VOLUME_DB);
            self.show_volume_value()
        } else {
            Task::none()
        }
    }

    pub(super) fn commit_volume(&mut self) -> Task<Message> {
        if !self.volume_is_interactive() {
            return Task::none();
        }
        match volume_level_for_slider(self.volume_slider) {
            Ok(level) => self.submit_volume(level),
            Err(error) => {
                self.announce(error);
                Task::none()
            }
        }
    }

    pub(super) fn hide_volume_value(&mut self, request_id: u64) -> Task<Message> {
        if request_id == self.volume.value_request_id {
            self.volume.value = None;
        }
        Task::none()
    }

    pub(super) fn open_source_picker(&mut self) -> Task<Message> {
        self.source_picker_open = true;
        if self.selected_capabilities().source_catalog_read
            && matches!(
                self.source_catalog.freshness,
                denon_avr_domain::Freshness::Unknown | denon_avr_domain::Freshness::Invalidated
            )
        {
            self.announce("Refreshing source list…");
            self.request_source_catalog()
        } else {
            Task::none()
        }
    }

    pub(super) fn close_source_picker(&mut self) -> Task<Message> {
        self.source_picker_open = false;
        Task::none()
    }

    pub(super) fn select_input(&mut self, value: String) -> Task<Message> {
        self.source_picker_open = false;
        match self.selected_capabilities().input(&value) {
            Ok(input) => self.control(MainZoneControl::Input(input)),
            Err(error) => {
                self.announce(error);
                Task::none()
            }
        }
    }

    pub(super) fn refresh_source_catalog(&mut self) -> Task<Message> {
        self.announce("Refreshing source list…");
        self.request_source_catalog()
    }
}
