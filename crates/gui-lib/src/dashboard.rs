//! Dashboard controls and receiver-status visualizations.

use super::*;
use iced::widget::{button, column, container, row, slider, text};

/// Denon `MV00` is -80.0 dB and `MV985` is +18.5 dB. Keep the desktop
/// control in that receiver-visible unit instead of treating the wire code as
/// a 0–60 UI value.
pub(crate) const MIN_VOLUME_DB: f32 = -80.0;
pub(crate) const MAX_VOLUME_DB: f32 = 18.5;
pub(crate) const VOLUME_HALF_DB_STEPS: u16 = 197;
pub(crate) const VOLUME_INDICATOR_HEIGHT: f32 = 28.0;

pub(crate) fn slider_volume(snapshot: &MainZoneSnapshot) -> Option<f32> {
    snapshot
        .volume
        .value()
        .map(|volume| volume.db_tenths() as f32 / 10.0)
}

pub(crate) fn volume_level_for_slider(value: f32) -> Result<denon_avr_domain::VolumeLevel, String> {
    if !value.is_finite() || !(MIN_VOLUME_DB..=MAX_VOLUME_DB).contains(&value) {
        return Err("Volume must be between -80.0 and +18.5 dB.".into());
    }
    let half_db_steps = (value * 2.0).round() as i16;
    let native_code = ((half_db_steps + 160) * 5) as u16;
    denon_avr_domain::VolumeLevel::from_native_code(native_code).map_err(str::to_owned)
}

pub(crate) fn volume_slider<'a>(value: f32, shown_value: Option<f32>) -> Element<'a, Message> {
    let bubble: Element<'a, Message> = shown_value.map_or_else(
        || space().into(),
        |shown_value| {
            let steps = ((shown_value.clamp(MIN_VOLUME_DB, MAX_VOLUME_DB) - MIN_VOLUME_DB) * 2.0)
                .round() as u16;
            let leading = steps.max(1);
            let trailing = VOLUME_HALF_DB_STEPS.saturating_sub(steps).max(1);
            row![
                space().width(Length::FillPortion(leading)),
                container(text(format!("{shown_value:.1} dB")).size(13))
                    .padding([3, 7])
                    .style(design::panel),
                space().width(Length::FillPortion(trailing)),
            ]
            .width(Length::Fill)
            .align_y(iced::Alignment::Center)
            .into()
        },
    );
    column![
        // Always reserve the bubble lane. Changing the slider's vertical
        // position while a pointer drag is in progress causes visible jitter.
        container(bubble)
            .width(Length::Fill)
            .height(Length::Fixed(VOLUME_INDICATOR_HEIGHT)),
        slider(MIN_VOLUME_DB..=MAX_VOLUME_DB, value, Message::VolumeChanged)
            .step(0.5_f32)
            .on_release(Message::CommitVolume)
            .width(Length::Fill),
    ]
    .spacing(2)
    .into()
}

/// The desktop dashboard intentionally exposes only the receiver's Main Zone
/// (Zone 1). Denon `PW` commands target that zone; secondary-zone power
/// commands are not available from this control.
pub(crate) fn main_zone_power_control(power: Option<&PowerState>) -> Option<MainZoneControl> {
    match power {
        Some(PowerState::On) => Some(MainZoneControl::Power(PowerState::Standby)),
        Some(PowerState::Standby) => Some(MainZoneControl::Power(PowerState::On)),
        None => None,
    }
}

pub(crate) fn power_recovery(
    lifecycle: &denon_avr_application::Lifecycle,
    snapshot: &MainZoneSnapshot,
) -> (&'static str, String, Option<(&'static str, Message)>) {
    let error = match &snapshot.power {
        denon_avr_domain::FieldStatus::Unavailable(error) => error.message.as_str(),
        denon_avr_domain::FieldStatus::Value(_) => "power status is not currently available",
    };
    match lifecycle {
        denon_avr_application::Lifecycle::Connecting
        | denon_avr_application::Lifecycle::Selected => (
            "CONNECTING",
            "Connecting to the selected receiver and requesting its status.".into(),
            None,
        ),
        denon_avr_application::Lifecycle::Reconnecting { .. } => (
            "RECONNECTING",
            "The receiver connection was interrupted; status refresh is being retried.".into(),
            None,
        ),
        denon_avr_application::Lifecycle::Disconnected => (
            "RECEIVER UNAVAILABLE",
            format!("Could not read power status: {error}"),
            Some(("Retry Status", Message::Refresh)),
        ),
        denon_avr_application::Lifecycle::NoReceiver => (
            "NO RECEIVER SELECTED",
            "Choose a receiver before requesting Main Zone status.".into(),
            Some(("Choose Receiver", Message::Navigate(Route::Receivers))),
        ),
        denon_avr_application::Lifecycle::Connected { .. } => (
            "POWER STATUS UNAVAILABLE",
            format!("Could not read power status: {error}"),
            Some(("Retry Status", Message::Refresh)),
        ),
        denon_avr_application::Lifecycle::Stopping | denon_avr_application::Lifecycle::Stopped => (
            "CONNECTION CLOSED",
            "The receiver session is stopping or has stopped.".into(),
            None,
        ),
    }
}

pub(crate) fn dashboard_header(
    power: Option<&PowerState>,
    power_action: Option<Message>,
) -> Element<'static, Message> {
    let (icon, color) = match power {
        Some(PowerState::On) => ("⏻", design::SUCCESS),
        Some(PowerState::Standby) => ("⏻", design::MUTED),
        None => ("?", design::WARNING),
    };
    let power_button = match power_action {
        Some(action) => button(text(icon).size(24).color(color))
            .padding([6, 10])
            .style(design::secondary)
            .on_press(action),
        None => button(text(icon).size(24).color(color))
            .padding([6, 10])
            .style(design::secondary),
    };
    row![
        container(power_button).align_right(Length::Fill),
        container(text("MAIN ZONE · ZONE 1").size(13).color(design::MUTED))
            .center_x(Length::Fixed(160.0)),
        space().width(Length::Fill)
    ]
    .align_y(iced::Alignment::Center)
    .into()
}

pub(crate) fn dashboard_context_line(
    source: &str,
    catalog: SourceCatalog,
) -> Element<'static, Message> {
    let label = catalog_entry_label(source, &catalog);
    let active_hidden = catalog
        .entry(source)
        .is_some_and(|entry| entry.visibility == SourceVisibility::Hidden);
    let context = row![
        button(
            row![
                text(source_icon(source)).size(18).color(design::ACCENT),
                text(if active_hidden {
                    format!("{label} · Hidden on receiver")
                } else {
                    label
                })
                .size(15)
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center)
        )
        .padding([6, 10])
        .style(design::secondary)
        .on_press(Message::OpenSourcePicker),
        container(space()).width(Length::Fill),
        container(text("Main Zone").size(14).color(design::MUTED)).center_x(Length::Fixed(120.0)),
        space().width(Length::Fill)
    ]
    .align_y(iced::Alignment::Center);

    context.into()
}

pub(crate) fn source_picker_overlay(
    capabilities: ModelCapabilities,
    catalog: SourceCatalog,
) -> Element<'static, Message> {
    // This offset places the panel immediately below the Dashboard's header
    // and source context without affecting the layout beneath it.
    container(column![
        space().height(Length::Fixed(86.0)),
        row![
            space().width(Length::Fixed(18.0)),
            source_picker_popup(capabilities, catalog),
            space().width(Length::Fill),
        ],
    ])
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

pub(crate) fn source_icon(source: &str) -> &'static str {
    match source.to_ascii_uppercase().as_str() {
        "TV" | "TV AUDIO" => "▣",
        "BT" | "BLUETOOTH" => "ᛒ",
        "HEOS" | "NETWORK" => "◉",
        "PHONO" => "◌",
        "CD" => "◍",
        "GAME" => "◇",
        _ => "●",
    }
}

pub(crate) fn source_label(source: &str) -> String {
    match source.to_ascii_uppercase().as_str() {
        "TV" | "TV AUDIO" => "TV Audio".into(),
        "BT" | "BLUETOOTH" => "Bluetooth".into(),
        "HEOS" | "NETWORK" => "Network".into(),
        _ => source.to_owned(),
    }
}

pub(crate) fn catalog_entry_label(source: &str, catalog: &SourceCatalog) -> String {
    catalog
        .entry(source)
        .map(|entry| entry.display_name_or(&source_label(source)).to_owned())
        .unwrap_or_else(|| source_label(source))
}

pub(crate) fn source_picker_popup(
    capabilities: ModelCapabilities,
    catalog: SourceCatalog,
) -> Element<'static, Message> {
    let choices: Element<'static, Message> = if capabilities.inputs.is_empty() {
        text("Source selection is unavailable for this receiver.")
            .color(design::MUTED)
            .into()
    } else if catalog.entries.is_empty()
        && matches!(
            catalog.freshness,
            denon_avr_domain::Freshness::Unknown | denon_avr_domain::Freshness::Invalidated
        )
    {
        text("Loading receiver source names…")
            .color(design::MUTED)
            .into()
    } else {
        capabilities
            .inputs
            .iter()
            // A partial or older catalog is still authoritative about the
            // entries it contains. Keep receiver rename/hide choices visible
            // rather than falling back to canonical names for those entries.
            .filter(|source| source_is_visible(&catalog, source))
            .fold(column![].spacing(7), |column, source| {
                column.push(
                    components::quiet_action(
                        catalog_entry_label(source, &catalog),
                        Message::SelectInput((*source).into()),
                    )
                    .width(Length::Fill),
                )
            })
            .into()
    };
    container(
        column![
            row![
                space().width(Length::Fill),
                components::quiet_icon_action("×", Message::CloseSourcePicker),
            ]
            .align_y(iced::Alignment::Center),
            scrollable(choices).height(Length::Fixed(440.0)),
        ]
        .spacing(12)
        .padding(14),
    )
    .width(Length::Fixed(300.0))
    .style(design::panel)
    .into()
}

pub(crate) fn source_is_visible(catalog: &SourceCatalog, source: &str) -> bool {
    catalog
        .entry(source)
        .is_none_or(|entry| entry.visibility != SourceVisibility::Hidden)
}

pub(crate) fn information_card<'a>(
    title: &'a str,
    values: &[(&'a str, &'a FieldStatus<String>)],
) -> iced::widget::Column<'a, Message> {
    let content = values
        .iter()
        .fold(column![].spacing(5), |column, (label, value)| {
            let displayed = match value {
                FieldStatus::Value(value) => value.as_str(),
                FieldStatus::Unavailable(_) => "Unavailable",
            };
            column.push(
                row![
                    text(*label).size(11).color(design::MUTED),
                    space().width(Length::Fill),
                    text(displayed).size(11)
                ]
                .width(Length::Fill),
            )
        });
    column![
        container(text(title).size(14).color(iced::Color::WHITE))
            .width(Length::Fill)
            .padding(iced::Padding::ZERO.top(10))
            .center_x(Length::Fill),
        content.padding(10),
    ]
}

#[allow(dead_code)]
pub(crate) fn channel_slot_grid<'a>(
    slots: &'a FieldStatus<Vec<ChannelSlot>>,
) -> iced::widget::Column<'a, Message> {
    match slots {
        FieldStatus::Unavailable(_) => column![text("Unavailable").size(13).color(design::MUTED)],
        FieldStatus::Value(slots) => slots.iter().fold(column![].spacing(5), |column, slot| {
            column.push(channel_slot_box(slot))
        }),
    }
}

#[allow(dead_code)]
pub(crate) fn channel_slot_box<'a>(slot: &'a ChannelSlot) -> Element<'a, Message> {
    let (foreground, state) = match slot.state {
        ChannelSlotState::Active => (design::SUCCESS, "Active"),
        ChannelSlotState::Available => (iced::Color::from_rgb(0.58, 0.76, 0.95), "Available"),
        ChannelSlotState::Absent => (design::MUTED, "Absent"),
        ChannelSlotState::Unknown => (design::MUTED, "Unknown"),
    };
    container(
        row![
            text(&slot.label).size(11).color(foreground),
            space().width(Length::Fill),
            text(state).size(10).color(foreground)
        ]
        .width(Length::Fill)
        .padding([3, 7]),
    )
    .style(design::panel)
    .into()
}

pub(crate) fn typed_input_channel_grid<'a>(
    slots: &'a FieldStatus<Vec<ChannelSlot>>,
) -> iced::widget::Column<'a, Message> {
    column![
        typed_channel_grid_row(
            &[Some("FHL"), Some("LFE"), None, Some("EXT"), Some("FHR")],
            slots
        ),
        typed_channel_grid_row(
            &[Some("FWL"), Some("FL"), Some("C"), Some("FR"), Some("FWR")],
            slots
        ),
        typed_channel_grid_row(&[None, Some("SL"), None, Some("SR"), None], slots),
        typed_channel_grid_row(&[None, Some("SBL"), Some("SB"), Some("SBR"), None], slots),
    ]
    .spacing(6)
}

pub(crate) fn typed_output_channel_grid<'a>(
    slots: &'a FieldStatus<Vec<ChannelSlot>>,
) -> iced::widget::Column<'a, Message> {
    column![
        typed_channel_grid_row(&[Some("FL"), Some("C"), Some("FR")], slots),
        typed_channel_grid_row(&[Some("SL"), Some("SW"), Some("SR")], slots),
        typed_channel_grid_row(&[Some("SBL"), None, Some("SBR")], slots),
        typed_channel_grid_row(&[Some("TRL"), None, Some("TRR")], slots),
    ]
    .spacing(6)
}

pub(crate) fn typed_channel_grid_row<'a>(
    channels: &[Option<&'static str>],
    slots: &'a FieldStatus<Vec<ChannelSlot>>,
) -> iced::widget::Row<'a, Message> {
    channels.iter().fold(row![].spacing(6), |row, channel| {
        row.push(match *channel {
            Some(label) => typed_channel_box(label, channel_slot_state(slots, label)),
            None => space()
                .width(Length::Fixed(44.0))
                .height(Length::Fixed(32.0))
                .into(),
        })
    })
}

pub(crate) fn channel_slot_state(
    slots: &FieldStatus<Vec<ChannelSlot>>,
    channel: &str,
) -> Option<ChannelSlotState> {
    let FieldStatus::Value(slots) = slots else {
        return None;
    };
    slots
        .iter()
        .find(|slot| slot.label == channel)
        .map(|slot| slot.state)
}

pub(crate) fn typed_channel_box<'a>(
    label: &'static str,
    state: Option<ChannelSlotState>,
) -> Element<'a, Message> {
    let (foreground, background, shadow) = match state {
        Some(ChannelSlotState::Active) => (
            design::SUCCESS,
            iced::Color {
                a: 0.28,
                ..design::SUCCESS
            },
            iced::Shadow {
                color: iced::Color {
                    a: 0.45,
                    ..design::SUCCESS
                },
                offset: iced::Vector::new(0.0, 0.0),
                blur_radius: 10.0,
            },
        ),
        Some(ChannelSlotState::Available) => (
            iced::Color::from_rgb(0.58, 0.76, 0.95),
            design::ACTIVE,
            iced::Shadow::default(),
        ),
        _ => (design::MUTED, design::ACTIVE, iced::Shadow::default()),
    };
    container(text(label).size(10).color(foreground))
        .center_x(Length::Fixed(44.0))
        .center_y(Length::Fixed(32.0))
        .style(move |_| iced::widget::container::Style {
            background: Some(iced::Background::Color(background)),
            text_color: Some(foreground),
            border: iced::Border {
                radius: 5.0.into(),
                width: 1.0,
                color: foreground,
            },
            shadow,
            ..Default::default()
        })
        .into()
}

#[allow(dead_code)]
pub(crate) fn input_channel_grid<'a>(layout: Option<&str>) -> iced::widget::Column<'a, Message> {
    column![
        channel_grid_row(
            &[Some("FHL"), Some("LEF"), None, Some("EXT"), Some("FHR")],
            layout
        ),
        channel_grid_row(
            &[Some("FWL"), Some("FL"), Some("C"), Some("FR"), Some("FWR")],
            layout
        ),
        channel_grid_row(&[None, Some("SL"), None, Some("SR"), None], layout),
        channel_grid_row(&[None, Some("SBL"), Some("SB"), Some("SBR"), None], layout),
    ]
    .spacing(6)
}

#[allow(dead_code)]
pub(crate) fn output_channel_grid<'a>(layout: Option<&str>) -> iced::widget::Column<'a, Message> {
    column![
        channel_grid_row(&[Some("FL"), Some("C"), Some("FR")], layout),
        channel_grid_row(&[Some("SL"), None, Some("SR")], layout),
    ]
    .spacing(6)
}

#[allow(dead_code)]
pub(crate) fn channel_grid_row<'a>(
    channels: &[Option<&'static str>],
    layout: Option<&str>,
) -> iced::widget::Row<'a, Message> {
    channels.iter().fold(row![].spacing(6), |row, channel| {
        row.push(match *channel {
            Some(label) => channel_box(label, channel_state(layout, label)),
            None => space()
                .width(Length::Fixed(44.0))
                .height(Length::Fixed(32.0))
                .into(),
        })
    })
}

#[allow(dead_code)]
pub(crate) fn channel_box<'a>(label: &'static str, state: &'static str) -> Element<'a, Message> {
    let active = state == "ON";
    let foreground = if active {
        design::SUCCESS
    } else {
        design::MUTED
    };
    let background = if active {
        iced::Color {
            a: 0.28,
            ..design::SUCCESS
        }
    } else {
        design::ACTIVE
    };
    let shadow = if active {
        iced::Shadow {
            color: iced::Color {
                a: 0.45,
                ..design::SUCCESS
            },
            offset: iced::Vector::new(0.0, 0.0),
            blur_radius: 10.0,
        }
    } else {
        iced::Shadow::default()
    };
    container(text(label).size(10).color(foreground))
        .center_x(Length::Fixed(44.0))
        .center_y(Length::Fixed(32.0))
        .style(move |_| iced::widget::container::Style {
            background: Some(iced::Background::Color(background)),
            text_color: Some(foreground),
            border: iced::Border {
                radius: 5.0.into(),
                width: 1.0,
                color: foreground,
            },
            shadow,
            ..Default::default()
        })
        .into()
}

#[allow(dead_code)]
pub(crate) fn channel_state(layout: Option<&str>, channel: &str) -> &'static str {
    let Some(layout) = layout else {
        return "UNKNOWN";
    };
    let tokens = layout
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_uppercase)
        .collect::<Vec<_>>();
    if !tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "FHL"
                | "LEF"
                | "LFE"
                | "EXT"
                | "FHR"
                | "FWL"
                | "FL"
                | "C"
                | "FR"
                | "FWR"
                | "SL"
                | "SR"
                | "SBL"
                | "SB"
                | "SBR"
        )
    }) {
        return "UNKNOWN";
    }
    let channel_matches = |token: &str| match channel {
        "LEF" => matches!(token, "LEF" | "LFE"),
        _ => token == channel,
    };
    if tokens.iter().any(|token| channel_matches(token)) {
        "ON"
    } else {
        "OFF"
    }
}

pub(crate) fn mode_icon(group: ListeningModeGroup) -> &'static str {
    match group {
        ListeningModeGroup::Movie => "▣",
        ListeningModeGroup::Music => "♫",
        ListeningModeGroup::Game => "◇",
    }
}

pub(crate) fn quick_select_bar<'a>(
    snapshot: &QuickSelectSnapshot,
    recall_supported: bool,
    names_supported: bool,
    catalog: SourceCatalog,
) -> Element<'a, Message> {
    let content: Element<'a, Message> = if recall_supported || names_supported {
        container(
            QuickSelectSlot::ALL
                .into_iter()
                .fold(row![].spacing(8), |row, slot| {
                    let label = text(quick_select_slot_label(snapshot, slot, &catalog)).size(12);
                    if recall_supported {
                        row.push(
                            button(label)
                                .padding([7, 10])
                                .style(design::secondary)
                                .on_press(Message::RecallQuickSelect(slot)),
                        )
                    } else {
                        row.push(container(label).padding([7, 10]))
                    }
                }),
        )
        .width(Length::Fill)
        .center_x(Length::Fill)
        .into()
    } else {
        text("Quick Select names are unavailable for this receiver.")
            .size(12)
            .color(design::MUTED)
            .into()
    };
    container(content)
        .width(Length::Fill)
        .padding(10)
        .style(design::panel)
        .into()
}

pub(crate) fn quick_select_slot_label(
    snapshot: &QuickSelectSnapshot,
    slot: QuickSelectSlot,
    catalog: &SourceCatalog,
) -> String {
    let Some(preset) = snapshot.preset(slot) else {
        return slot.number().to_string();
    };
    let name = if preset.available {
        preset
            .name
            .as_ref()
            .map(|name| name.as_str())
            .map(str::to_owned)
    } else {
        None
    };
    let source = match &preset.summary.input {
        denon_avr_domain::Registered::Included(input) => {
            Some(catalog_entry_label(input.as_str(), catalog))
        }
        _ => None,
    };
    let base = name.map_or_else(
        || slot.number().to_string(),
        |name| format!("{} {name}", slot.number()),
    );
    source.map_or(base.clone(), |source| format!("{base} · {source}"))
}

/// Unknown EQ observations carry no usable display value. Keep that state
/// distinct internally, but leave the dashboard summary blank until at least
/// one receiver-reported value is available.
#[allow(dead_code)]
pub(crate) fn eq_summary_if_reported(status: &EqStatus) -> Option<String> {
    denon_avr_domain::EqFeature::ALL
        .into_iter()
        .any(|feature| !matches!(status.state(feature), denon_avr_domain::EqState::Unknown))
        .then(|| feedback::eq_summary(status))
}
