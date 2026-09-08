//! Code-native visual tokens for the receiver console.

use iced::widget::{button, container};
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

pub const RAIL: f32 = 240.0;
pub const MIN_WIDTH: f32 = 1400.0;
pub const MIN_HEIGHT: f32 = 880.0;
pub const MAX_SESSION_MESSAGES: usize = 100;

pub fn theme() -> Theme {
    Theme::Dark
}

pub fn panel(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(SURFACE)),
        border: iced::Border {
            radius: 12.0.into(),
            width: 1.0,
            color: BORDER,
        },
        ..Default::default()
    }
}

pub fn rail(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(SURFACE)),
        border: iced::Border {
            width: 0.0,
            ..Default::default()
        },
        ..Default::default()
    }
}

pub fn primary(_: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => ACTIVE,
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

pub fn secondary(_: &Theme, status: button::Status) -> button::Style {
    let background = if matches!(status, button::Status::Hovered) {
        ACTIVE
    } else {
        SURFACE
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: TEXT,
        border: iced::Border {
            radius: 8.0.into(),
            width: 1.0,
            color: BORDER,
        },
        ..Default::default()
    }
}
