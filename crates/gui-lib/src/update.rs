//! The message reducer: `Gui::update` dispatches each message to the handler
//! that owns it.

use super::*;

impl Gui {
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ConfigLoaded(Ok(config)) => self.config_loaded(config),
            Message::ConfigLoaded(Err(error)) => self.config_load_failed(error),
            Message::CommandFinished(Err(error)) => self.command_failed(error),
            Message::CommandFinished(Ok(())) => Task::none(),
            Message::Navigate(route) => self.navigate(route),
            Message::ToggleMessages => self.toggle_messages(),
            Message::ClearMessages => self.clear_messages(),
            Message::SetTextScale(scale) => self.set_text_scale(scale),
            Message::SetMotion(preference) => self.set_motion(preference),
            Message::SetContrast(preference) => self.set_contrast(preference),
            Message::CaptureVisual => self.capture_visual(),
            Message::ScreenshotCaptured(screenshot) => self.screenshot_captured(screenshot),
            Message::ScreenshotWritten(Ok(path)) => self.screenshot_written(path),
            Message::ScreenshotWritten(Err(error)) => self.screenshot_write_failed(error),
            Message::Keyboard(event) => self.keyboard(event),
            Message::AddressChanged(value) => self.address_changed(value),
            Message::NameChanged(value) => self.name_changed(value),
            Message::Select(selection) => self.select(selection),
            Message::Bridge(event) => self.on_bridge(*event),
            Message::Refresh => self.refresh(),
            Message::HttpTick => self.http_tick(),
            Message::ToggleMainZonePower => self.toggle_main_zone_power(),
            Message::ToggleZone2Power => self.toggle_zone2_power(),
            Message::OpenMainZonePowerPopup => self.open_main_zone_power_popup(),
            Message::OpenZone2PowerPopup => self.open_zone2_power_popup(),
            Message::ClosePowerPopup => self.close_power_popup(),
            Message::SetMainZonePower(power) => self.set_main_zone_power(power),
            Message::SetZone2Power(power) => self.set_zone2_power(power),
            Message::Mute => self.control(MainZoneControl::Mute(denon_avr_domain::MuteState::On)),
            Message::Unmute => self.unmute(),
            Message::VolumeChanged(value) => self.volume_changed(value),
            Message::CommitVolume => self.commit_volume(),
            Message::AdjustVolume(delta) => self.adjust_volume(delta),
            Message::HideVolumeValue(request_id) => self.hide_volume_value(request_id),
            Message::SoundModeControlTimedOut(request_id) => {
                self.sound_mode_control_timed_out(request_id)
            }
            Message::LaunchTick => self.launch_tick(),
            Message::SelectSoundModeCategory(category) => self.select_sound_mode_category(category),
            Message::SelectSurroundMode(category, value) => {
                self.select_surround_mode(category, value)
            }
            Message::ToggleSoundModeFavorite(mode) => self.toggle_sound_mode_favorite(mode),
            Message::SoundModeFavoritesSaved(Ok(config)) => self.sound_mode_favorites_saved(config),
            Message::SoundModeFavoritesSaved(Err(error)) => {
                self.sound_mode_favorites_save_failed(error)
            }
            Message::OpenSourcePicker => self.open_source_picker(),
            Message::CloseSourcePicker => self.close_source_picker(),
            Message::SelectInput(value) => self.select_input(value),
            Message::RefreshSourceCatalog => self.refresh_source_catalog(),
            Message::Discover => self.discover(),
            Message::DiscoveryFinished(Ok(receivers)) => self.discovery_finished(receivers),
            Message::DiscoveryFinished(Err(error)) => self.discovery_failed(error),
            Message::SaveDiscovered(receiver) => self.save_discovered(receiver),
            Message::DiscoveredSaved(Ok((config, selection))) => {
                self.discovered_saved(config, selection)
            }
            Message::DiscoveredSaved(Err(error)) => self.discovered_save_failed(error),
            Message::ManualSetup => self.manual_setup(),
            Message::ManualSaved(Ok((config, selection))) => self.manual_saved(config, selection),
            Message::ManualSaved(Err(error)) => self.manual_save_failed(error),
            Message::Shutdown => self.shut_down(None),
            Message::CloseRequested(window) => self.close_requested(window),
            Message::ShutdownFinished(Some(window)) => iced::window::close(window),
            Message::ShutdownFinished(None) => Task::none(),
        }
    }
}
