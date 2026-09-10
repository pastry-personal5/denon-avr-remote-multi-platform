//! Route-specific views for the desktop presentation.

use super::*;
use iced::widget::{button, column, radio, scrollable};

impl Gui {
    pub(super) fn dashboard(&self) -> iced::widget::Column<'_, Message> {
        let capabilities = self.selected_capabilities();
        let writable = capabilities.writable;
        let waiting = self.status_waiting();
        let frame = self.status_wait_ticks;
        let input = self
            .snapshot
            .input
            .value()
            .map(ToString::to_string)
            .unwrap_or_else(|| {
                if waiting {
                    waiting_dots(frame).into()
                } else {
                    String::new()
                }
            });
        let power_action = (writable
            && main_zone_power_control(self.snapshot.power.value()).is_some())
        .then_some(Message::ToggleMainZonePower);
        let main_zone_popup_action = (writable && self.snapshot.power.value().is_some())
            .then_some(Message::OpenMainZonePowerPopup);
        let zone2_popup_action = (capabilities.zone2_power && self.zone2.power.value().is_some())
            .then_some(Message::OpenZone2PowerPopup);
        let header = dashboard_header(
            self.snapshot.power.value(),
            power_action,
            main_zone_popup_action,
            self.zone2.power.value(),
            zone2_popup_action,
            &input,
            self.source_catalog.clone(),
            waiting,
            frame,
        );
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
            let (title, detail, action) = if waiting {
                (
                    format!("WAITING {}", waiting_dots(frame)),
                    String::new(),
                    None,
                )
            } else {
                let (title, detail, action) = power_recovery(&self.lifecycle, &self.snapshot);
                (title.to_owned(), detail, action)
            };
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

        let quick_select_supported = capabilities.quick_select_recall;
        let quick_select_names_supported = capabilities.quick_select_names;
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
            // Keep Mute compact so the slider remains the primary control in
            // this row. The prior fill width made it read as a second slider.
            .width(Length::Fixed(96.0))
            .into()
        } else {
            text("Controls unavailable: receiver model is not validated for writes.")
                .color(design::MUTED)
                .width(Length::Fixed(96.0))
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
                    container(
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
                    )
                    .width(Length::FillPortion(2))
                    .height(Length::Fill)
                    .center_y(Length::Fill),
                    column![
                        space().height(Length::Fixed(VOLUME_CONTROL_TOP_OFFSET)),
                        mute_controls,
                    ],
                    column![
                        space().height(Length::Fixed(VOLUME_CONTROL_TOP_OFFSET)),
                        row![
                            components::quiet_action("≪", Message::AdjustVolume(-10.0)),
                            components::quiet_action("−", Message::AdjustVolume(-0.5)),
                            components::quiet_action("+", Message::AdjustVolume(0.5)),
                            components::quiet_action("≫", Message::AdjustVolume(10.0)),
                        ]
                        .spacing(4),
                    ],
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center)
                .padding([4, 0]),
            )
            .width(Length::Fill)
            .height(Length::Fixed(VOLUME_ROW_HEIGHT))
            .into()
        } else {
            text("Volume controls are unavailable until the receiver reports its current volume.")
                .color(design::MUTED)
                .into()
        };
        column![
            header,
            // The source picker shares the power-control row. This lead makes
            // that compact row read 16 px above the information dashboard.
            space().height(Length::Fixed(16.0)),
            row![
                column![
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
                        .height(Length::Fixed(DASHBOARD_INFORMATION_TOP_ROW_HEIGHT))
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
                        .height(Length::Fixed(DASHBOARD_INFORMATION_TOP_ROW_HEIGHT))
                        .style(design::panel),
                    ]
                    .spacing(16),
                    row![
                        container(information_card(
                            "AUDIO",
                            &[
                                ("Input", &information.audio.input_mode),
                                ("Output", &information.audio.output),
                                ("Signal", &information.audio.signal),
                                ("Sound", &information.audio.sound),
                                ("Rate", &information.audio.sample_rate),
                            ],
                            waiting,
                            frame,
                        ))
                        .width(Length::Fill)
                        .height(Length::Fixed(DASHBOARD_INFORMATION_BOTTOM_ROW_HEIGHT))
                        .style(design::panel),
                        container(information_card(
                            "VIDEO",
                            &[
                                ("Monitor", &information.video.monitor),
                                ("HDMI in", &information.video.hdmi_input),
                                ("HDMI out", &information.video.hdmi_output)
                            ],
                            waiting,
                            frame,
                        ))
                        .width(Length::Fill)
                        .height(Length::Fixed(DASHBOARD_INFORMATION_BOTTOM_ROW_HEIGHT))
                        .style(design::panel),
                        container(information_card(
                            "AUDYSSEY",
                            &[
                                ("MultEQ", &information.audyssey.multeq),
                                ("Dynamic EQ", &information.audyssey.dynamic_eq),
                                ("Dynamic Volume", &information.audyssey.dynamic_volume),
                            ],
                            waiting,
                            frame,
                        ))
                        .width(Length::Fill)
                        .height(Length::Fixed(DASHBOARD_INFORMATION_BOTTOM_ROW_HEIGHT))
                        .style(design::panel)
                    ]
                    .spacing(DASHBOARD_INFORMATION_ROW_GAP),
                ]
                .spacing(DASHBOARD_INFORMATION_ROW_GAP)
                .width(Length::FillPortion(2)),
                sound_mode_panel(
                    capabilities,
                    self.snapshot
                        .surround_mode
                        .value()
                        .map(|mode| mode.as_str()),
                    self.snapshot
                        .sound_mode_category
                        .or(self.sound_mode_category_preference),
                    &self.configured,
                    waiting,
                    frame,
                ),
            ]
            .spacing(DASHBOARD_INFORMATION_ROW_GAP),
            volume_controls,
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

fn sound_mode_panel<'a>(
    capabilities: ModelCapabilities,
    active: Option<&'a str>,
    selected_category: Option<SoundModeCategory>,
    favorites: &'a ConfiguredReceivers,
    waiting: bool,
    frame: u16,
) -> Element<'a, Message> {
    use denon_avr_domain::SoundModeCategory;
    let categories = [
        ("MOVIE", SoundModeCategory::Movie),
        ("MUSIC", SoundModeCategory::Music),
        ("GAME", SoundModeCategory::Game),
        ("PURE", SoundModeCategory::Pure),
    ];
    let rows = categories
        .into_iter()
        .flat_map(|(category, kind)| {
            capabilities
                .sound_modes(kind)
                .iter()
                .enumerate()
                .map(move |(index, mode)| (category, kind, *mode, index == 0))
        })
        .collect::<Vec<_>>();
    // A mode can appear in more than one organizational category. Select the
    // first matching row so the receiver-reported `MS…` state lights exactly
    // one radio button.
    let selected = selected_sound_mode_index(&rows, active, selected_category);
    let current_category =
        selected.and_then(|index| rows.get(index).map(|(_, category, _, _)| *category));
    let table = rows.into_iter().enumerate().fold(
        column![].spacing(2),
        |table, (index, (category, kind, mode, group_start))| {
            let selected_row = selected == Some(index);
            let row_style = if selected_row {
                design::sound_mode_row_selected
            } else if index.is_multiple_of(2) {
                design::sound_mode_row_a
            } else {
                design::sound_mode_row_b
            };
            let favorite = favorites.is_sound_mode_favorite(kind, mode);
            let category_control: Element<'_, Message> = if group_start {
                button(
                    text(category)
                        .size(9)
                        .width(Length::Fill)
                        .align_x(iced::alignment::Horizontal::Center),
                )
                .padding([3, 5])
                .style(if current_category == Some(kind) {
                    design::primary
                } else {
                    design::secondary
                })
                .on_press(Message::SelectSoundModeCategory(kind))
                .width(Length::Fixed(52.0))
                .into()
            } else {
                container(text("")).width(Length::Fixed(52.0)).into()
            };
            table.push(
                container(
                    row![
                        category_control,
                        text(mode).size(9).width(Length::Fill),
                        button(text(if favorite { "♥" } else { "♡" }).size(13))
                            .padding([0, 3])
                            .style(if favorite {
                                design::sound_mode_favorite_heart
                            } else {
                                design::sound_mode_inactive_heart
                            })
                            .on_press(Message::ToggleSoundModeFavorite(kind, mode.into())),
                        container(
                            radio("", index, selected, |_| Message::SelectSurroundMode(
                                kind,
                                mode.into()
                            ))
                            .size(12)
                            .spacing(0)
                            .style(design::sound_mode_radio),
                        )
                        .width(Length::Fixed(40.0))
                        .padding(iced::Padding::ZERO.right(12)),
                    ]
                    .align_y(iced::Alignment::Center)
                    .spacing(6),
                )
                .padding([3, 6])
                .width(Length::Fill)
                .style(row_style),
            )
        },
    );
    container(
        column![
            row![
                text("SOUND MODE").size(14).color(iced::Color::WHITE),
                text(active.map_or_else(
                    || if waiting {
                        format!("WAITING {}", waiting_dots(frame))
                    } else {
                        String::new()
                    },
                    str::to_owned,
                ))
                .size(9)
                .color(design::MUTED),
            ]
            .width(Length::Fill)
            .align_y(iced::Alignment::Center)
            .spacing(8),
            scrollable(table).height(Length::Fill).spacing(10),
        ]
        .spacing(8)
        .padding(12),
    )
    .width(Length::Fill)
    .height(Length::Fixed(SOUND_MODE_PANEL_HEIGHT))
    .style(design::panel)
    .into()
}

fn selected_sound_mode_index(
    rows: &[(&str, SoundModeCategory, &str, bool)],
    active: Option<&str>,
    selected_category: Option<SoundModeCategory>,
) -> Option<usize> {
    active
        .zip(selected_category)
        .and_then(|(active, selected_category)| {
            rows.iter().position(|(_, category, mode, _)| {
                *mode == active && selected_category == *category
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detailed_mode_radio_respects_the_clicked_category_when_mode_is_shared() {
        let rows = [
            ("MOVIE", SoundModeCategory::Movie, "DOLBY SURROUND", true),
            ("MUSIC", SoundModeCategory::Music, "DOLBY SURROUND", true),
        ];
        assert_eq!(
            selected_sound_mode_index(
                &rows,
                Some("DOLBY SURROUND"),
                Some(SoundModeCategory::Music),
            ),
            Some(1)
        );
        // Receiver status carries the detailed mode but no category. A radio
        // is active only after the controller reports the confirmed pair.
        assert_eq!(
            selected_sound_mode_index(&rows, Some("DOLBY SURROUND"), None),
            None
        );
    }
}
