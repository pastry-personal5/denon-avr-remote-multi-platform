//! Code-native visual tokens for the receiver console.

use iced::widget::{button, container, text_input};
use iced::{Background, Color, Theme};

pub const CANVAS: Color = Color::from_rgb(0.067, 0.075, 0.082);
pub const SURFACE: Color = Color::from_rgb(0.094, 0.110, 0.125);
pub const ACTIVE: Color = Color::from_rgb(0.133, 0.153, 0.176);
pub const BORDER: Color = Color::from_rgb(0.204, 0.231, 0.263);
pub const TEXT: Color = Color::from_rgb(0.957, 0.945, 0.918);
pub const MUTED: Color = Color::from_rgb(0.710, 0.733, 0.765);
pub const ACCENT: Color = Color::from_rgb(0.851, 0.604, 0.227);
pub const SUCCESS: Color = Color::from_rgb(0.345, 0.722, 0.553);
pub const WARNING: Color = ACCENT;
pub const ERROR: Color = Color::from_rgb(0.882, 0.431, 0.404);
pub const HIGH_CONTRAST_BORDER: Color = Color::from_rgb(0.92, 0.94, 0.97);
pub const MESSAGE_ICON: Color = Color::from_rgb(0.30, 0.32, 0.35);

pub const RAIL: f32 = 180.0;
pub const MIN_WIDTH: f32 = 1100.0;
pub const MIN_HEIGHT: f32 = 760.0;
pub const MAX_SESSION_MESSAGES: usize = 100;

pub fn theme(high_contrast: bool) -> Theme {
    Theme::custom(
        if high_contrast {
            "Receiver console high contrast"
        } else {
            "Receiver console"
        },
        iced::theme::Palette {
            background: if high_contrast { Color::BLACK } else { CANVAS },
            text: if high_contrast { Color::WHITE } else { TEXT },
            primary: if high_contrast {
                Color::from_rgb(1.0, 0.75, 0.25)
            } else {
                ACCENT
            },
            success: SUCCESS,
            warning: WARNING,
            danger: ERROR,
        },
    )
}

fn high_contrast(theme: &Theme) -> bool {
    theme.palette().background == Color::BLACK
}

pub fn panel(theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(if high_contrast(theme) {
            Color::BLACK
        } else {
            SURFACE
        })),
        border: iced::Border {
            radius: 12.0.into(),
            width: 1.0,
            color: if high_contrast(theme) {
                HIGH_CONTRAST_BORDER
            } else {
                BORDER
            },
        },
        ..Default::default()
    }
}

pub fn rail(theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(if high_contrast(theme) {
            Color::BLACK
        } else {
            SURFACE
        })),
        border: iced::Border {
            width: 0.0,
            ..Default::default()
        },
        ..Default::default()
    }
}

pub fn primary(theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => ACTIVE,
        _ if high_contrast(theme) => theme.palette().primary,
        _ => ACCENT,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: Color::from_rgb(0.08, 0.07, 0.05),
        border: iced::Border {
            radius: 8.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

pub fn secondary(theme: &Theme, status: button::Status) -> button::Style {
    let background = if matches!(status, button::Status::Hovered) {
        ACTIVE
    } else {
        if high_contrast(theme) {
            Color::BLACK
        } else {
            SURFACE
        }
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: TEXT,
        border: iced::Border {
            radius: 8.0.into(),
            width: 1.0,
            color: if high_contrast(theme) {
                HIGH_CONTRAST_BORDER
            } else {
                BORDER
            },
        },
        ..Default::default()
    }
}

pub fn message_icon(theme: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: None,
        text_color: if matches!(status, button::Status::Hovered) {
            MUTED
        } else if high_contrast(theme) {
            HIGH_CONTRAST_BORDER
        } else {
            MESSAGE_ICON
        },
        border: iced::Border::default(),
        ..Default::default()
    }
}

/// Style for editable fields. Keeping it here prevents stock light fields
/// from appearing on setup routes when the application theme changes.
pub fn text_field(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: Background::Color(if high_contrast(theme) {
            Color::BLACK
        } else {
            SURFACE
        }),
        border: iced::Border {
            radius: 8.0.into(),
            width: if focused { 2.0 } else { 1.0 },
            color: if focused {
                theme.palette().primary
            } else if high_contrast(theme) {
                HIGH_CONTRAST_BORDER
            } else {
                BORDER
            },
        },
        icon: MUTED,
        placeholder: MUTED,
        value: TEXT,
        selection: theme.palette().primary,
    }
}

/// WCAG relative contrast ratio for opaque sRGB tokens. Keeping this tiny
/// implementation local makes token regressions testable without claiming a
/// platform semantic accessibility audit.
pub fn contrast_ratio(foreground: Color, background: Color) -> f32 {
    fn linear(channel: f32) -> f32 {
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    }
    let luminance = |color: Color| {
        0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
    };
    let first = luminance(foreground);
    let second = luminance(background);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_text_and_focus_tokens_meet_their_contrast_floor() {
        assert!(contrast_ratio(TEXT, CANVAS) >= 7.0);
        assert!(contrast_ratio(MUTED, CANVAS) >= 4.5);
        assert!(contrast_ratio(ACCENT, CANVAS) >= 4.5);
        assert!(contrast_ratio(HIGH_CONTRAST_BORDER, Color::BLACK) >= 7.0);
    }
}
