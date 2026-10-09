//! Keyboard shortcuts the window handles itself: focus traversal with Tab and,
//! on macOS, Command-W.

#[cfg(target_os = "macos")]
use super::capture_scenario::request_close_latest_window;
use super::{Gui, Message, Task};

impl Gui {
    pub(super) fn keyboard(&self, event: iced::keyboard::Event) -> Task<Message> {
        #[cfg(target_os = "macos")]
        if is_close_window_shortcut(&event) {
            return request_close_latest_window();
        }
        match tab_direction(&event) {
            Some(TabDirection::Forward) => iced::widget::operation::focus_next(),
            Some(TabDirection::Backward) => iced::widget::operation::focus_previous(),
            None => Task::none(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TabDirection {
    Forward,
    Backward,
}

pub(super) fn tab_direction(event: &iced::keyboard::Event) -> Option<TabDirection> {
    match event {
        iced::keyboard::Event::KeyPressed { key, modifiers, .. }
            if *key == iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab) =>
        {
            Some(if modifiers.shift() {
                TabDirection::Backward
            } else {
                TabDirection::Forward
            })
        }
        _ => None,
    }
}

/// Command-W is the standard macOS command for closing the active window.
/// Use the physical key as a Latin fallback so it also works with non-Latin
/// keyboard layouts.
#[cfg(target_os = "macos")]
pub(super) fn is_close_window_shortcut(event: &iced::keyboard::Event) -> bool {
    matches!(
        event,
        iced::keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            ..
        } if modifiers.command()
            && matches!(key.to_latin(*physical_key), Some('w' | 'W'))
    )
}
