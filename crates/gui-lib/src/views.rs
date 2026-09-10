//! Route-specific views for the desktop presentation.

use super::*;
use iced::widget::column;

impl Gui {
    pub(super) fn dashboard(&self) -> iced::widget::Column<'_, Message> {
        let capabilities = self.selected_capabilities();
        let writable = capabilities.writable;
        let power_action = (writable
            && main_zone_power_control(self.snapshot.power.value()).is_some())
        .then_some(Message::ToggleMainZonePower);
        let header = dashboard_header(self.snapshot.power.value(), power_action);
        if self.snapshot.power.value() == Some(&PowerState::Standby) {
            return column![container(stack![
                container(text("POWER OFF").size(36).color(design::MUTED))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .center(Length::Fill),
                container(header).width(Length::Fill)
            ])
            .width(Length::Fill)
            .height(Length::Fixed(600.0))];
        }
        if self.snapshot.power.value() != Some(&PowerState::On) {
            let (title, detail, action) = power_recovery(&self.lifecycle, &self.snapshot);
            let action: Element<'_, Message> = action.map_or_else(
                || space().into(),
                |(label, message)| components::action(label, message).into(),
            );
            return column![
                header,
                container(
                    column![
                        text(title).size(28).color(design::MUTED),
                        text(detail).size(15).color(design::MUTED),
                        action,
                    ]
                    .spacing(14)
                    .align_x(iced::Alignment::Center)
                )
                .width(Length::Fill)
                .height(Length::Fixed(520.0))
                .center(Length::Fill)
            ]
            .spacing(18);
        }

        let input = self
            .snapshot
            .input
            .value()
            .map(ToString::to_string)
            .unwrap_or_else(|| "Unavailable".into());
        let quick_select_supported = capabilities.quick_select_recall;
        let quick_select_names_supported = capabilities.quick_select_names;
        let group_controls = if writable {
            ListeningModeGroup::ALL
                .into_iter()
                .fold(row![].spacing(12), |row, group| {
                    row.push(
                        components::quiet_action(
                            format!("{}  {}", mode_icon(group), group.as_str()),
                            Message::SelectListeningModeGroup(group),
                        )
                        .width(Length::Fill),
                    )
                })
                .width(Length::Fill)
        } else {
            row![text("Mode groups unavailable for this receiver.")]
        };
        let mute_controls: Element<'_, Message> = if writable {
            let muted = self.snapshot.mute.value() == Some(&denon_avr_domain::MuteState::On);
            components::toggle_action(
                if muted { "🔇" } else { "🔊" },
                "Mute",
                muted,
                if muted {
                    Message::Unmute
                } else {
                    Message::Mute
                },
            )
            .width(Length::Fill)
            .into()
        } else {
            text("Controls unavailable: receiver model is not validated for writes.")
                .color(design::MUTED)
                .into()
        };
        let information = &self.snapshot.http_information;
        // Keep the established volume control visible while its command is in
        // flight. `volume_is_interactive` still rejects input until the
        // receiver confirms the command, but replacing the slider with a
        // transient status label makes an ordinary adjustment look like the
        // control disappeared.
        let volume_controls: Element<'_, Message> = if self.snapshot.volume.value().is_some() {
            container(
                row![
                    column![
                        container(volume_slider(self.volume_slider, self.volume_value,))
                            .width(Length::Fill),
                        row![
                            text("-80.0 dB").size(11).color(design::MUTED),
                            space().width(Length::Fill),
                            text("+18.5 dB").size(11).color(design::MUTED),
                        ]
                        .width(Length::Fill),
                    ]
                    .spacing(2)
                    .width(Length::Fill),
                    row![
                        components::quiet_action("≪", Message::AdjustVolume(-10.0)),
                        components::quiet_action("−", Message::AdjustVolume(-0.5)),
                        components::quiet_action("+", Message::AdjustVolume(0.5)),
                        components::quiet_action("≫", Message::AdjustVolume(10.0)),
                    ]
                    .spacing(4),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center)
                .padding([4, 0]),
            )
            .width(Length::Fill)
            .height(Length::Fixed(90.0))
            .into()
        } else {
            text("Volume controls are unavailable until the receiver reports its current volume.")
                .color(design::MUTED)
                .into()
        };
        column![
            header,
            dashboard_context_line(&input, self.source_catalog.clone()),
            row![
                container(
                    column![
                        container(text("INPUT").size(14).color(iced::Color::WHITE))
                            .width(Length::Fill)
                            .padding(iced::Padding::ZERO.top(10))
                            .center_x(Length::Fill),
                        container(typed_input_channel_grid(&information.input_slots))
                            .width(Length::Fill)
                            .height(Length::Fill)
                            .padding(iced::Padding::ZERO.bottom(12))
                            .center(Length::Fill)
                    ]
                    .height(Length::Fill)
                )
                .width(Length::Fill)
                .height(Length::Fixed(190.0))
                .style(design::panel),
                container(
                    column![
                        container(text("OUTPUT").size(14).color(iced::Color::WHITE))
                            .width(Length::Fill)
                            .padding(iced::Padding::ZERO.top(10))
                            .center_x(Length::Fill),
                        container(typed_output_channel_grid(&information.output_slots))
                            .width(Length::Fill)
                            .height(Length::Fill)
                            .padding(iced::Padding::ZERO.bottom(12))
                            .center(Length::Fill)
                    ]
                    .height(Length::Fill)
                )
                .width(Length::Fill)
                .height(Length::Fixed(190.0))
                .style(design::panel),
                container(information_card(
                    "AUDYSSEY",
                    &[
                        ("MultEQ", &information.audyssey.multeq),
                        ("Dynamic EQ", &information.audyssey.dynamic_eq),
                        ("Dynamic Volume", &information.audyssey.dynamic_volume),
                    ]
                ))
                .width(Length::Fill)
                .height(Length::Fixed(190.0))
                .style(design::panel)
            ]
            .spacing(16),
            row![
                container(information_card(
                    "VIDEO",
                    &[
                        ("Monitor", &information.video.monitor),
                        ("HDMI in", &information.video.hdmi_input),
                        ("HDMI out", &information.video.hdmi_output)
                    ]
                ))
                .width(Length::Fill)
                .height(Length::Fixed(125.0))
                .style(design::panel),
                container(information_card(
                    "AUDIO",
                    &[
                        ("Input", &information.audio.input_mode),
                        ("Output", &information.audio.output),
                        ("Signal", &information.audio.signal),
                        ("Sound", &information.audio.sound),
                        ("Rate", &information.audio.sample_rate),
                    ]
                ))
                .width(Length::Fill)
                .height(Length::Fixed(125.0))
                .style(design::panel),
            ]
            .spacing(16),
            volume_controls,
            row![mute_controls, group_controls]
                .width(Length::Fill)
                .spacing(14)
                .align_y(iced::Alignment::Center),
            quick_select_bar(
                &self.quick_select,
                quick_select_supported,
                quick_select_names_supported,
                self.source_catalog.clone(),
            ),
        ]
        .spacing(10)
    }
}
