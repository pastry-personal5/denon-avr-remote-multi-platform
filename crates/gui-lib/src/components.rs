//! Reusable presentation primitives. They intentionally accept plain text and
//! domain-derived values so receiver policy stays in the application layer.

use crate::{design, Message, Route};
use iced::widget::{button, column, container, row, text};
use iced::{Element, Length, Theme};

pub fn panel<'a>(title: &'a str, body: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(
        column![text(title).size(20), body.into()]
            .spacing(12)
            .padding(20),
    )
    .width(Length::Fill)
    .style(design::panel)
    .into()
}

pub fn action<'a>(label: impl Into<String>, message: Message) -> iced::widget::Button<'a, Message> {
    button(text(label.into()))
        .padding([10, 16])
        .style(design::primary)
        .on_press(message)
}

pub fn quiet_action<'a>(
    label: impl Into<String>,
    message: Message,
) -> iced::widget::Button<'a, Message> {
    button(text(label.into()))
        .padding([9, 14])
        .style(design::secondary)
        .on_press(message)
}

pub fn quiet_icon_action<'a>(icon: &'a str, message: Message) -> iced::widget::Button<'a, Message> {
    button(text(icon).size(18))
        .padding([7, 10])
        .style(design::secondary)
        .on_press(message)
}

pub fn nav<'a>(label: &'a str, route: Route, active: bool) -> iced::widget::Button<'a, Message> {
    let button = button(text(label)).width(Length::Fill).padding([11, 14]);
    if active {
        button
            .style(design::primary)
            .on_press(Message::Navigate(route))
    } else {
        button
            .style(design::secondary)
            .on_press(Message::Navigate(route))
    }
}

pub fn state_row<'a>(label: &'a str, value: String) -> Element<'a, Message> {
    row![
        text(label).width(Length::Fixed(170.0)).color(design::MUTED),
        text(value).size(16)
    ]
    .spacing(12)
    .into()
}

pub fn status_color(status: &str) -> iced::Color {
    match status {
        "confirmed" | "connected" => design::SUCCESS,
        "rejected" | "error" => design::ERROR,
        _ => design::WARNING,
    }
}

pub fn _theme(_: &Theme) {}
