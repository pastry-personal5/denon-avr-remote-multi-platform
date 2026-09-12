//! Route-specific views for the desktop presentation.

use super::*;
use iced::widget::{button, column, radio, scrollable};

impl Gui {
    pub(super) fn dashboard(&self) -> Element<'_, Message> {
        let capabilities = self.selected_capabilities();
        let writable = capabilities.writable;
        let sound_mode_pending = self.sound_mode_request_id.is_some();
        let pending = dashboard_pending(self.status_waiting(), sound_mode_pending);
        let input = self
            .snapshot
            .input
            .value()
            .map(ToString::to_string)
            .unwrap_or_default();
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
            .height(Length::Fixed(600.0))]
            .into();
        }
        if self.snapshot.power.value() != Some(&PowerState::On) && !pending {
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
            .spacing(18)
            .into();
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
        // flight. The slider remains inert until the receiver confirms the
        // command, but replacing it with a
        // transient status label makes an ordinary adjustment look like the
        // control disappeared.
        let volume_controls: Element<'_, Message> = container(
            row![
                container(
                    column![
                        container(volume_slider(
                            self.volume_slider,
                            self.volume_value,
                            self.volume_is_interactive(),
                        ))
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
        .into();
        let dashboard = column![
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
                    self.sound_mode_category_filter,
                    &self.configured,
                    self.sound_mode_request_id.is_none(),
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
        .spacing(10);
        if pending {
            stack![dashboard, sound_mode_wait_overlay(self.launch_frame),].into()
        } else {
            dashboard.into()
        }
    }
}

fn sound_mode_wait_overlay(frame: u8) -> Element<'static, Message> {
    // The same text-free spinner used during launch, deliberately without a
    // panel background so the current dashboard remains visible below it.
    launch_waiting_animation(frame)
}

fn dashboard_pending(status_waiting: bool, sound_mode_pending: bool) -> bool {
    status_waiting || sound_mode_pending
}

fn sound_mode_panel<'a>(
    capabilities: ModelCapabilities,
    active: Option<&'a str>,
    category_filter: Option<SoundModeCategory>,
    favorites: &'a ConfiguredReceivers,
    controls_enabled: bool,
) -> Element<'a, Message> {
    let categories = sound_mode_categories();
    let rows = sound_mode_rows(capabilities, category_filter);
    let selected = selected_sound_mode_index(&rows, active);
    let category_buttons = categories.into_iter().fold(
        row![].spacing(4).width(Length::Fill),
        |buttons, (label, category)| {
            let mut control = button(
                text(label)
                    .size(9)
                    .width(Length::Fill)
                    .align_x(iced::alignment::Horizontal::Center),
            )
            .padding([4, 5])
            .style(if category_filter == Some(category) {
                design::primary
            } else {
                design::secondary
            })
            .width(Length::FillPortion(1));
            if sound_mode_category_is_interactive(capabilities, category, controls_enabled) {
                control = control.on_press(Message::SelectSoundModeCategory(category));
            }
            buttons.push(control)
        },
    );
    let table =
        rows.into_iter()
            .enumerate()
            .fold(column![].spacing(2), |table, (index, (kind, mode))| {
                let selected_row = selected == Some(index);
                let row_style = if selected_row {
                    design::sound_mode_row_selected
                } else if index.is_multiple_of(2) {
                    design::sound_mode_row_a
                } else {
                    design::sound_mode_row_b
                };
                let favorite = favorites.is_sound_mode_favorite(mode);
                let select_control: Element<'_, Message> = if controls_enabled {
                    radio("", index, selected, |_| {
                        Message::SelectSurroundMode(kind, mode.into())
                    })
                    .size(12)
                    .spacing(0)
                    .style(design::sound_mode_radio)
                    .into()
                } else {
                    text(if selected_row { "◉" } else { "○" })
                        .size(15)
                        .color(design::MUTED)
                        .into()
                };
                table.push(
                    container(
                        row![
                            text(mode).size(9).width(Length::Fill),
                            container(
                                button(text(if favorite { "♥" } else { "♡" }).size(13))
                                    .padding([0, 3])
                                    .style(if favorite {
                                        design::sound_mode_favorite_heart
                                    } else {
                                        design::sound_mode_inactive_heart
                                    })
                                    .on_press(Message::ToggleSoundModeFavorite(mode.into())),
                            )
                            .width(Length::Fixed(54.0)),
                            container(select_control)
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
            });
    container(
        column![
            row![
                text("SOUND MODE").size(14).color(iced::Color::WHITE),
                text(active.map_or_else(String::new, str::to_owned))
                    .size(9)
                    .color(design::MUTED),
            ]
            .width(Length::Fill)
            .align_y(iced::Alignment::Center)
            .spacing(8),
            category_buttons,
            row![
                text("Detailed Sound Mode").size(9).width(Length::Fill),
                text("Favorite").size(9).width(Length::Fixed(54.0)),
                container(text("Select").size(9))
                    .width(Length::Fixed(40.0))
                    .padding(iced::Padding::ZERO.right(12)),
            ]
            .spacing(6),
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
    rows: &[(SoundModeCategory, &str)],
    active: Option<&str>,
) -> Option<usize> {
    active.and_then(|active| rows.iter().position(|(_, mode)| *mode == active))
}

fn sound_mode_categories() -> [(&'static str, SoundModeCategory); 4] {
    [
        ("MOVIE", SoundModeCategory::Movie),
        ("MUSIC", SoundModeCategory::Music),
        ("GAME", SoundModeCategory::Game),
        ("PURE", SoundModeCategory::Pure),
    ]
}

fn sound_mode_category_is_interactive(
    capabilities: ModelCapabilities,
    category: SoundModeCategory,
    controls_enabled: bool,
) -> bool {
    controls_enabled
        && (category == SoundModeCategory::Pure
            || capabilities.supports_control(&MainZoneControl::RecallSoundModeCategory(category)))
}

fn sound_mode_rows(
    capabilities: ModelCapabilities,
    category_filter: Option<SoundModeCategory>,
) -> Vec<(SoundModeCategory, &'static str)> {
    sound_mode_categories()
        .into_iter()
        .filter(|(_, category)| category_filter.is_none_or(|filter| filter == *category))
        .flat_map(|(_, category)| {
            capabilities
                .sound_modes(category)
                .iter()
                .map(move |mode| (category, *mode))
        })
        .fold(Vec::new(), |mut rows, row| {
            if !rows.iter().any(|(_, mode)| *mode == row.1) {
                rows.push(row);
            }
            rows
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_modes_follow_catalog_order_and_deduplicate_shared_modes() {
        let capabilities = ModelCapabilities::for_model(Model::AvrX3800h);
        let rows = sound_mode_rows(capabilities, None);
        assert_eq!(
            rows.first(),
            Some(&(SoundModeCategory::Movie, "DOLBY SURROUND"))
        );
        assert_eq!(
            rows.iter()
                .filter(|(_, mode)| *mode == "DOLBY SURROUND")
                .count(),
            1
        );
        assert!(rows.iter().any(|(category, mode)| {
            *category == SoundModeCategory::Pure && *mode == "PURE DIRECT"
        }));
    }

    #[test]
    fn category_buttons_are_in_movie_music_game_pure_order() {
        assert_eq!(
            sound_mode_categories().map(|(label, _)| label),
            ["MOVIE", "MUSIC", "GAME", "PURE"]
        );
    }

    #[test]
    fn status_and_sound_mode_pending_share_one_dashboard_overlay() {
        assert!(dashboard_pending(true, false));
        assert!(dashboard_pending(true, true));
        assert!(dashboard_pending(false, true));
        assert!(!dashboard_pending(false, false));
    }

    #[test]
    fn unsupported_receivers_do_not_offer_category_recalls() {
        let unknown = ModelCapabilities::for_model(Model::Unknown);
        assert!(!sound_mode_category_is_interactive(
            unknown,
            SoundModeCategory::Movie,
            true
        ));
        assert!(sound_mode_category_is_interactive(
            unknown,
            SoundModeCategory::Pure,
            true
        ));

        let x3800h = ModelCapabilities::for_model(Model::AvrX3800h);
        assert!(sound_mode_category_is_interactive(
            x3800h,
            SoundModeCategory::Movie,
            true
        ));
        assert!(!sound_mode_category_is_interactive(
            x3800h,
            SoundModeCategory::Movie,
            false
        ));
    }

    #[test]
    fn category_filter_and_authoritative_detailed_selection_are_independent() {
        let capabilities = ModelCapabilities::for_model(Model::AvrX3800h);
        let rows = sound_mode_rows(capabilities, Some(SoundModeCategory::Music));
        assert!(rows
            .iter()
            .all(|(category, _)| *category == SoundModeCategory::Music));
        assert!(selected_sound_mode_index(&rows, Some("DOLBY SURROUND")).is_some());
        assert_eq!(selected_sound_mode_index(&rows, Some("PURE DIRECT")), None);
    }
}
