//! Settings, advanced controls, and diagnostic views.

use super::*;
use iced::widget::column;

impl Gui {
    pub(super) fn settings(&self) -> iced::widget::Column<'_, Message> {
        let scale_controls = [
            (100_u8, "100%"),
            (125, "125%"),
            (150, "150%"),
            (175, "175%"),
            (200, "200%"),
        ]
        .into_iter()
        .fold(row![].spacing(8), |row, (scale, label)| {
            row.push(components::toggle_action(
                "Aa",
                label,
                self.text_scale == scale,
                Message::SetTextScale(scale),
            ))
        });
        column![
            text("Settings").size(32),
            components::panel("Appearance", column![
                text("Dark console theme").size(16),
                text("Session accessibility overrides are never persisted. When a reliable host preference is available it is the starting point; these controls take precedence.").color(design::MUTED),
                text("Text scale").color(design::MUTED),
                scale_controls,
                text("Motion").color(design::MUTED),
                row![
                    components::toggle_action("◌", "Normal", self.motion_preference == MotionPreference::Normal, Message::SetMotion(MotionPreference::Normal)),
                    components::toggle_action("◐", "Reduced", self.motion_preference == MotionPreference::Reduced, Message::SetMotion(MotionPreference::Reduced)),
                ].spacing(8),
                text("Contrast").color(design::MUTED),
                row![
                    components::toggle_action("◒", "Normal", self.contrast_preference == ContrastPreference::Normal, Message::SetContrast(ContrastPreference::Normal)),
                    components::toggle_action("◑", "High", self.contrast_preference == ContrastPreference::High, Message::SetContrast(ContrastPreference::High)),
                ].spacing(8),
                components::quiet_action("Capture current screen", Message::CaptureVisual),
                text("Capture is enabled only when DENON_AVR_CAPTURE_DIR names an explicit directory. Files are native RGBA PNGs organized by platform, route, and text scale.").color(design::MUTED),
                text("Screen-reader semantic support is release-blocked pending an Iced native accessibility bridge and three-platform audit.").color(design::MUTED),
            ].spacing(10)),
            components::panel("Configuration", column![text("YAML configuration is managed by the desktop host."), text("Quick Select slot editing requires validated receiver support.").color(design::MUTED)]),
            components::panel("Source presentation", column![
                text("Source names and visibility are managed on the receiver at Settings → Inputs → Source Rename / Hide Sources.").color(design::MUTED),
                text(match self.source_catalog.freshness {
                    denon_avr_domain::Freshness::Live => "Receiver source list is current.",
                    denon_avr_domain::Freshness::Partial => "Receiver source list is last known; the latest refresh was partial.",
                    denon_avr_domain::Freshness::Invalidated => "Source list will refresh after reconnect.",
                    denon_avr_domain::Freshness::Unknown => "Source list has not been confirmed.",
                }).color(design::MUTED),
                components::quiet_action("Refresh source list", Message::RefreshSourceCatalog),
            ].spacing(10))
        ].spacing(18)
    }

    pub(super) fn advanced(&self) -> iced::widget::Column<'_, Message> {
        column![components::panel(
            "Receiver status",
            column![
                text("Request a fresh authoritative Main Zone status snapshot.")
                    .color(design::MUTED),
                components::action("Refresh Status", Message::Refresh),
            ]
            .spacing(12),
        ),]
        .spacing(18)
    }

    pub(super) fn diagnostics(&self) -> iced::widget::Column<'_, Message> {
        column![
            text("Diagnostics").size(32),
            components::panel(
                "Connection",
                column![
                    components::state_row("Lifecycle", lifecycle_label(&self.lifecycle).into()),
                    components::state_row("Generation", self.generation.to_string()),
                    components::state_row(
                        "Selected receiver",
                        self.selection
                            .as_ref()
                            .map(|s| s.identity().host.clone())
                            .unwrap_or_else(|| "None".into())
                    )
                ]
            ),
            components::panel(
                "Observed state",
                column![
                    components::state_row(
                        "Snapshot authority",
                        format!("{:?}", self.snapshot.authority)
                    ),
                    components::state_row(
                        "Quick Select freshness",
                        format!("{:?}", self.quick_select.freshness)
                    ),
                    components::state_row(
                        "Source catalog freshness",
                        format!("{:?}", self.source_catalog.freshness)
                    ),
                    components::state_row(
                        "Source catalog entries",
                        self.source_catalog.entries.len().to_string()
                    ),
                    text(feedback::eq_summary(&self.eq_status)),
                    text(feedback::eq_evidence_summary(&self.eq_status)).color(design::MUTED),
                    text(
                        "Unknown, unavailable, and not-applicable states remain distinct from Off."
                    )
                    .color(design::MUTED)
                ]
            )
        ]
        .spacing(18)
    }
}
