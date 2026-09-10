//! Saved, discovered, and manual receiver setup views.

use super::*;
use iced::widget::column;

impl Gui {
    pub(super) fn receivers(&self) -> iced::widget::Column<'_, Message> {
        let mut page = column![text("Receivers").size(32), text("Saved receivers")].spacing(12);
        for (name, identity) in &self.configured.receivers {
            page = page.push(
                row![
                    text(format!("{name} · {}", identity.host)),
                    components::quiet_action(
                        "Select",
                        Message::Select(ReceiverSelection::Saved {
                            name: name.clone(),
                            identity: identity.clone()
                        })
                    )
                ]
                .spacing(12),
            );
        }
        for receiver in &self.discovered {
            page = page.push(
                row![
                    column![
                        text(receiver.model.as_deref().unwrap_or("Unknown model")).size(16),
                        text(format!("Address · {}:{}", receiver.address.host, receiver.address.port)).color(design::MUTED),
                        text("Discovered on the local network; selecting opens the Main Zone console.").size(12).color(design::MUTED)
                    ].spacing(4).width(Length::Fill),
                    components::action("Save and open console", Message::SaveDiscovered(receiver.clone()))
                ]
                .spacing(16)
                .align_y(iced::Alignment::Center),
            );
        }
        page.push(text("Find a receiver"))
            .push(components::quiet_action("Discover", Message::Discover))
            .push(
                row![
                    text_input("Receiver address", &self.address)
                        .on_input(Message::AddressChanged)
                        .style(design::text_field),
                    text_input("Name (optional)", &self.name)
                        .on_input(Message::NameChanged)
                        .style(design::text_field),
                    components::action("Save and connect", Message::ManualSetup)
                ]
                .spacing(8),
            )
    }
}
