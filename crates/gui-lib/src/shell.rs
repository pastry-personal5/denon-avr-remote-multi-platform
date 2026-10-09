//! The application frame: navigation rail, route toolbar, routed body with
//! its overlays, and the global message panel.

use super::*;
use iced::widget::column;

impl Gui {
    pub fn view(&self) -> Element<'_, Message> {
        if !self.launch_ready {
            return launch_waiting_animation(self.launch.frame);
        }
        let rail = container(
            column![
                text("MAIN ZONE")
                    .width(Length::Fill)
                    .align_x(iced::Alignment::Center)
                    .size(12)
                    .color(design::MUTED),
                components::nav(
                    "Dashboard",
                    Route::Dashboard,
                    self.route == Route::Dashboard
                ),
                components::nav(
                    "Receivers",
                    Route::Receivers,
                    self.route == Route::Receivers
                ),
                components::nav("Settings", Route::Settings, self.route == Route::Settings),
                components::nav("Advanced", Route::Advanced, self.route == Route::Advanced),
                components::nav(
                    "Diagnostics",
                    Route::Diagnostics,
                    self.route == Route::Diagnostics
                ),
            ]
            .spacing(14)
            .padding(20),
        )
        .width(Length::Fixed(design::RAIL))
        .height(Length::Fill)
        .style(design::rail);
        let toolbar: Element<'_, Message> = if self.route == Route::Dashboard {
            space().height(Length::Shrink).into()
        } else {
            row![column![
                text(route_title(self.route)).size(28),
                text("Main Zone receiver context")
                    .size(13)
                    .color(design::MUTED)
            ]
            .spacing(4)
            .width(Length::Fill),]
            .align_y(iced::Alignment::Center)
            .spacing(16)
            .into()
        };
        let base_body: Element<'_, Message> = match self.route {
            Route::Dashboard => self.dashboard(),
            Route::Receivers => self.receivers().into(),
            Route::Settings => self.settings().into(),
            Route::Advanced => self.advanced().into(),
            Route::Diagnostics => self.diagnostics().into(),
        };
        let dashboard_popup = (self.route == Route::Dashboard)
            .then_some(self.power_popup)
            .flatten();
        let source_picker_open = self.route == Route::Dashboard && self.source_picker_open;
        let body: Element<'_, Message> = match (source_picker_open, dashboard_popup) {
            // Keep overlays separate from the dashboard column so opening one
            // never reflows cards or controls below the source line.
            (true, None) => stack![
                base_body,
                source_picker_overlay(self.selected_capabilities(), self.source_catalog.clone()),
            ]
            .into(),
            (false, Some(popup)) => stack![
                base_body,
                power_popup_overlay(popup, self.snapshot.power.value(), self.zone2.power.value(),),
            ]
            .into(),
            (true, Some(popup)) => stack![
                base_body,
                source_picker_overlay(self.selected_capabilities(), self.source_catalog.clone()),
                power_popup_overlay(popup, self.snapshot.power.value(), self.zone2.power.value(),),
            ]
            .into(),
            (false, None) => base_body,
        };
        let messages: Element<'_, Message> = if self.messages_collapsed {
            container(
                components::message_icon_action("⌄", Message::ToggleMessages)
                    .width(Length::Shrink)
                    .padding([2, 6]),
            )
            .width(Length::Fill)
            .style(design::panel)
            .into()
        } else {
            let items = self
                .messages
                .iter()
                .rev()
                .take(3)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .fold(column![].spacing(5), |column, message| {
                    column.push(text(message).size(12))
                });
            container(
                row![
                    scrollable(items)
                        .width(Length::Fill)
                        .height(Length::Fixed(54.0))
                        .anchor_bottom(),
                    components::message_icon_action("⌫", Message::ClearMessages),
                    components::message_icon_action("⌃", Message::ToggleMessages)
                ]
                .spacing(6)
                .align_y(iced::Alignment::Center),
            )
            .width(Length::Fill)
            .padding(6)
            .style(design::panel)
            .into()
        };
        container(
            row![
                rail,
                container(
                    column![toolbar, scrollable(body).height(Length::Fill), messages]
                        .spacing(18)
                        .padding(28)
                        .width(Length::Fill)
                )
                .width(Length::Fill)
                .height(Length::Fill)
            ]
            .height(Length::Fill)
            .width(Length::Fill),
        )
        .style(|_| iced::widget::container::Style {
            background: Some(iced::Background::Color(design::CANVAS)),
            text_color: Some(design::TEXT),
            ..Default::default()
        })
        .into()
    }
}

fn route_title(route: Route) -> &'static str {
    match route {
        Route::Dashboard => "Dashboard",
        Route::Receivers => "Receiver setup",
        Route::Settings => "Settings",
        Route::Advanced => "Advanced",
        Route::Diagnostics => "Diagnostics",
    }
}
