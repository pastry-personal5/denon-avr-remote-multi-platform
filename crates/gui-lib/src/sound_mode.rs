//! Sound mode controls: choosing a category or a mode, the request timeout,
//! and saving the sound mode favorites.

use super::setup::save_configuration;
use super::*;

/// Save the sound mode favorites in `config`, leaving the rest of the stored
/// configuration as it is now on disk.
async fn save_favorites(
    control: SharedOperatorControl,
    config: ConfiguredReceivers,
) -> Result<ConfiguredReceivers, String> {
    let mut stored = control
        .configuration()
        .await
        .map_err(|error| error.to_string())?;
    stored.sound_mode_favorites = config.sound_mode_favorites.clone();
    save_configuration(control, stored).await?;
    Ok(config)
}

impl Gui {
    pub(super) fn sound_mode_control_timed_out(&mut self, request_id: u64) -> Task<Message> {
        if self.sound_mode.request_id == Some(request_id) {
            self.sound_mode.request_id = None;
            self.announce("Sound Mode request timed out; controls re-enabled.");
        }
        Task::none()
    }

    pub(super) fn select_sound_mode_category(
        &mut self,
        category: SoundModeCategory,
    ) -> Task<Message> {
        if self.sound_mode.request_id.is_some() {
            return Task::none();
        }
        self.sound_mode.category_filter = Some(category);
        if category == SoundModeCategory::Pure {
            Task::none()
        } else {
            self.control_sound_mode(MainZoneControl::RecallSoundModeCategory(category))
        }
    }

    pub(super) fn select_surround_mode(
        &mut self,
        category: SoundModeCategory,
        value: String,
    ) -> Task<Message> {
        if self.sound_mode.request_id.is_some() {
            return Task::none();
        }
        match denon_avr_domain::SurroundMode::new(value) {
            Ok(mode) => {
                self.control_sound_mode(MainZoneControl::SelectSoundMode { category, mode })
            }
            Err(error) => {
                self.announce(error);
                Task::none()
            }
        }
    }

    pub(super) fn toggle_sound_mode_favorite(&mut self, mode: String) -> Task<Message> {
        let mut config = self.configured.clone();
        let favorite = match config.toggle_current_sound_mode_favorite(&mode) {
            Ok(favorite) => favorite,
            Err(error) => {
                self.announce(error);
                return Task::none();
            }
        };
        let control = self.control.clone();
        self.announce(format!(
            "{} {} favorite…",
            if favorite { "Saving" } else { "Removing" },
            mode
        ));
        // Publish the new preference immediately so subsequent rapid
        // clicks build on it instead of cloning a stale committed
        // configuration. Persist writes are serialized below.
        self.configured = config.clone();
        if self.sound_mode.save_in_flight {
            self.sound_mode.pending_config = Some(config);
            return Task::none();
        }
        self.sound_mode.save_in_flight = true;
        Task::perform(
            save_favorites(control, config),
            Message::SoundModeFavoritesSaved,
        )
    }

    pub(super) fn sound_mode_favorites_saved(
        &mut self,
        config: ConfiguredReceivers,
    ) -> Task<Message> {
        self.configured = config;
        self.announce("Sound mode favorites saved.");
        if let Some(next) = self.sound_mode.pending_config.take() {
            self.configured = next.clone();
            let control = self.control.clone();
            return Task::perform(
                save_favorites(control, next),
                Message::SoundModeFavoritesSaved,
            );
        }
        self.sound_mode.save_in_flight = false;
        Task::none()
    }

    pub(super) fn sound_mode_favorites_save_failed(&mut self, error: String) -> Task<Message> {
        self.sound_mode.save_in_flight = false;
        self.sound_mode.pending_config = None;
        self.announce(format!("Could not save sound mode favorites: {error}"));
        Task::none()
    }
}
