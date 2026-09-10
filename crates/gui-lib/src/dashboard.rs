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
pub(crate) const VOLUME_ROW_HEIGHT: f32 = 90.0;
pub(crate) const DASHBOARD_INFORMATION_TOP_ROW_HEIGHT: f32 = 235.0;
pub(crate) const DASHBOARD_INFORMATION_BOTTOM_ROW_HEIGHT: f32 = 170.0;
pub(crate) const DASHBOARD_INFORMATION_ROW_GAP: f32 = 16.0;
pub(crate) const SOUND_MODE_PANEL_HEIGHT: f32 = DASHBOARD_INFORMATION_TOP_ROW_HEIGHT
    + DASHBOARD_INFORMATION_ROW_GAP
    + DASHBOARD_INFORMATION_BOTTOM_ROW_HEIGHT;
// The slider component reserves the 28 px indicator lane, its 2 px gap, the
// slider lane, and the scale-label lane. These tokens make the optical
// alignment below explicit and regression-testable.
const VOLUME_SLIDER_COMPONENT_HEIGHT: f32 = 67.0;
const VOLUME_THUMB_CENTER_IN_COMPONENT: f32 = 41.0;
/// Optical offset that aligns adjacent buttons with the slider thumb rather
/// than with the taller slider component (which also contains a value bubble
/// and scale labels).
pub(crate) const VOLUME_CONTROL_TOP_OFFSET: f32 =
    2.0 * VOLUME_THUMB_CENTER_IN_COMPONENT - VOLUME_SLIDER_COMPONENT_HEIGHT;

#[cfg(test)]
const fn volume_slider_thumb_center(row_height: f32) -> f32 {
    (row_height - VOLUME_SLIDER_COMPONENT_HEIGHT) / 2.0 + VOLUME_THUMB_CENTER_IN_COMPONENT
}

#[cfg(test)]
const fn volume_button_center(row_height: f32) -> f32 {
    // A button column is centered in the row; its button's center moves down
    // by half the chosen optical top offset.
    (row_height + VOLUME_CONTROL_TOP_OFFSET) / 2.0
}

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
    main_zone_popup_action: Option<Message>,
    zone2_power: Option<&PowerState>,
    zone2_popup_action: Option<Message>,
    source: &str,
    catalog: SourceCatalog,
    waiting: bool,
    frame: u16,
) -> Element<'static, Message> {
    let (icon, color) = match power {
        Some(PowerState::On) => ("⏻", design::SUCCESS),
        Some(PowerState::Standby) => ("⏻", design::MUTED),
        None => (if waiting { "·" } else { "" }, design::MUTED),
    };
    let power_button = match power_action.clone() {
        Some(action) => button(text(icon).size(24).color(color))
            .padding([6, 10])
            .style(design::secondary)
            .on_press(action),
        None => button(text(icon).size(24).color(color))
            .padding([6, 10])
            .style(design::secondary),
    };
    let state_label = |name: &str, state: Option<&PowerState>| {
        format!(
            "{name} · {}",
            match state {
                Some(PowerState::On) => "ON",
                Some(PowerState::Standby) => "STANDBY",
                None if waiting => waiting_dots(frame),
                None => "",
            }
        )
    };
    let zone2_label = button(
        text(state_label("ZONE 2", zone2_power))
            .size(13)
            .color(design::MUTED),
    )
    .padding([6, 10])
    .style(design::secondary);
    let zone2_label = if let Some(action) = zone2_popup_action {
        zone2_label.on_press(action)
    } else {
        zone2_label
    };
    let source_button = button(
        row![
            text(source_icon(source)).size(16).color(design::ACCENT),
            text(catalog_entry_label(source, &catalog)).size(13),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center),
    )
    .padding([6, 10])
    .style(design::secondary)
    .on_press(Message::OpenSourcePicker);
    container(
        row![
            container(power_button),
            button(
                text(state_label("MAIN ZONE", power))
                    .size(13)
                    .color(design::MUTED)
            )
            .padding([6, 10])
            .style(design::secondary)
            .on_press_maybe(main_zone_popup_action),
            zone2_label,
            source_button,
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center),
    )
    .width(Length::Fill)
    .center_x(Length::Fill)
    .into()
}

pub(crate) fn power_popup_overlay(
    target: PowerPopup,
    main_zone_power: Option<&PowerState>,
    zone2_power: Option<&PowerState>,
) -> Element<'static, Message> {
    let (title, state, on, standby) = match target {
        PowerPopup::MainZone => (
            "MAIN ZONE POWER",
            main_zone_power,
            Message::SetMainZonePower(PowerState::On),
            Message::SetMainZonePower(PowerState::Standby),
        ),
        PowerPopup::Zone2 => (
            "ZONE 2 POWER",
            zone2_power,
            Message::SetZone2Power(PowerState::On),
            Message::SetZone2Power(PowerState::Standby),
        ),
    };
    let active = state.copied();
    let choice = |label, selected, message| {
        button(text(label))
            .padding([8, 14])
            .style(if selected {
                design::primary
            } else {
                design::secondary
            })
            .on_press(message)
    };
    container(
        container(
            column![
                text(title).size(14),
                row![
                    choice("On", active == Some(PowerState::On), on),
                    choice("Standby", active == Some(PowerState::Standby), standby),
                ]
                .spacing(8),
                components::quiet_action("Close", Message::ClosePowerPopup),
            ]
            .spacing(10)
            .padding(14),
        )
        .style(design::panel),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(iced::alignment::Horizontal::Center)
    .align_y(iced::alignment::Vertical::Top)
    .into()
}

pub(crate) fn source_picker_overlay(
    capabilities: ModelCapabilities,
    catalog: SourceCatalog,
) -> Element<'static, Message> {
    // The source button now lives in the Dashboard header, so place its popup
    // immediately below that compact row without reflowing the information
    // panels below it.
    container(column![
        space().height(Length::Fixed(52.0)),
        // The source selector is the rightmost member of the centered header
        // control group. Bias the popup to that same right-hand position.
        row![
            space().width(Length::FillPortion(4)),
            source_picker_popup(capabilities, catalog),
            space().width(Length::FillPortion(1)),
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
            // Keep the compact desktop picker focused on the front-panel and
            // commonly wired auxiliary inputs. AUX3 through AUX7 remain
            // receiver capabilities, but are intentionally not listed here.
            .filter(|source| source_is_picker_entry(source))
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
    container(column![scrollable(choices).height(Length::Fixed(440.0)),].padding(14))
        .width(Length::Fixed(300.0))
        .style(design::panel)
        .into()
}

pub(crate) fn source_is_visible(catalog: &SourceCatalog, source: &str) -> bool {
    catalog
        .entry(source)
        .is_none_or(|entry| entry.visibility != SourceVisibility::Hidden)
}

pub(crate) fn source_is_picker_entry(source: &str) -> bool {
    !matches!(source, "AUX3" | "AUX4" | "AUX5" | "AUX6" | "AUX7")
}

pub(crate) fn information_card<'a>(
    title: &'a str,
    values: &[(&'a str, &'a FieldStatus<String>)],
    waiting: bool,
    frame: u16,
) -> iced::widget::Column<'a, Message> {
    let content = values
        .iter()
        .fold(column![].spacing(5), |column, (label, value)| {
            let displayed = match value {
                FieldStatus::Value(value) => value.as_str(),
                FieldStatus::Unavailable(error) if waiting && error.message == "not queried" => {
                    waiting_dots(frame)
                }
                FieldStatus::Unavailable(_) => "",
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

pub(crate) fn waiting_dots(frame: u16) -> &'static str {
    ["·  ", "·· ", "···", " ··"][(frame as usize / 2) % 4]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_buttons_share_the_slider_thumbs_visual_center() {
        let slider_center = volume_slider_thumb_center(VOLUME_ROW_HEIGHT);
        let mute_and_step_center = volume_button_center(VOLUME_ROW_HEIGHT);
        assert!((slider_center - mute_and_step_center).abs() < f32::EPSILON);
    }
}
