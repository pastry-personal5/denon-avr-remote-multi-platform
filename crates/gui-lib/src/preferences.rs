//! Navigation, the message panel, and the session-only accessibility
//! preferences.

use super::*;

impl Gui {
    pub(super) fn navigate(&mut self, route: Route) -> Task<Message> {
        self.route = route;
        Task::none()
    }

    pub(super) fn toggle_messages(&mut self) -> Task<Message> {
        self.messages_collapsed = !self.messages_collapsed;
        Task::none()
    }

    pub(super) fn clear_messages(&mut self) -> Task<Message> {
        self.messages.clear();
        self.announcement = "Message history cleared.".into();
        Task::none()
    }

    pub(super) fn set_text_scale(&mut self, scale: u8) -> Task<Message> {
        self.text_scale = scale.clamp(100, 200);
        self.announce(format!(
            "Text scale set to {}% for this session.",
            self.text_scale
        ));
        Task::none()
    }

    pub(super) fn set_motion(&mut self, preference: MotionPreference) -> Task<Message> {
        self.motion_preference = preference;
        self.announce(match preference {
            MotionPreference::Normal => "Normal motion enabled for this session.",
            MotionPreference::Reduced => "Reduced motion enabled for this session.",
        });
        Task::none()
    }

    pub(super) fn set_contrast(&mut self, preference: ContrastPreference) -> Task<Message> {
        self.contrast_preference = preference;
        self.announce(match preference {
            ContrastPreference::Normal => "Normal contrast enabled for this session.",
            ContrastPreference::High => "High contrast enabled for this session.",
        });
        Task::none()
    }
}
