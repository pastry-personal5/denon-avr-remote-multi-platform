//! Receiver setup: discovering receivers or entering one by hand, and saving
//! the chosen one as the current receiver. The views are in receiver_setup.rs.

use super::*;

/// Save `config` through the port and hand it back.
pub(super) async fn save_configuration(
    control: SharedOperatorControl,
    config: ConfiguredReceivers,
) -> Result<ConfiguredReceivers, String> {
    control
        .save_configuration(&config)
        .await
        .map(|_| config)
        .map_err(|error| error.to_string())
}

/// Add or replace one receiver in the saved configuration and make it current,
/// keeping every other receiver and every sound mode favorite. The file is read
/// first, so an edit made by hand since the last load is not overwritten.
async fn save_receiver(
    control: SharedOperatorControl,
    name: String,
    identity: ReceiverIdentity,
) -> Result<(ConfiguredReceivers, Selection), String> {
    let mut config = control
        .configuration()
        .await
        .map_err(|error| error.to_string())?;
    config.receivers.insert(name.clone(), identity.clone());
    config.current = Some(name.clone());
    let config = save_configuration(control, config).await?;
    Ok((config, Selection { name, identity }))
}

impl Gui {
    pub(super) fn address_changed(&mut self, value: String) -> Task<Message> {
        self.address = value;
        Task::none()
    }

    pub(super) fn name_changed(&mut self, value: String) -> Task<Message> {
        self.name = value;
        Task::none()
    }

    pub(super) fn discover(&mut self) -> Task<Message> {
        self.announce("Searching for receivers…");
        let control = self.control.clone();
        Task::perform(
            async move {
                control
                    .discover(Duration::from_secs(3))
                    .await
                    .map_err(|error| error.to_string())
            },
            Message::DiscoveryFinished,
        )
    }

    pub(super) fn discovery_finished(
        &mut self,
        receivers: Vec<denon_avr_domain::DiscoveredReceiver>,
    ) -> Task<Message> {
        self.discovered = receivers;
        if let [receiver] = self.discovered.as_slice() {
            let receiver = receiver.clone();
            self.announce("Receiver discovered; saving it as the current receiver…");
            self.update(Message::SaveDiscovered(receiver))
        } else {
            self.announce(format!("Found {} receiver(s).", self.discovered.len()));
            Task::none()
        }
    }

    pub(super) fn discovery_failed(&mut self, error: String) -> Task<Message> {
        self.announce(format!("Discovery failed: {error}"));
        Task::none()
    }

    pub(super) fn save_discovered(
        &mut self,
        receiver: denon_avr_domain::DiscoveredReceiver,
    ) -> Task<Message> {
        let (name, identity) = receiver_entry_for_discovered(&receiver);
        let control = self.control.clone();
        self.announce(format!(
            "Saving {} as the current receiver…",
            identity_label(&identity)
        ));
        Task::perform(
            save_receiver(control, name, identity),
            Message::DiscoveredSaved,
        )
    }

    pub(super) fn discovered_saved(
        &mut self,
        config: ConfiguredReceivers,
        selection: Selection,
    ) -> Task<Message> {
        self.configured = config;
        self.announce("Receiver saved; connecting…");
        self.update(Message::Select(selection))
    }

    pub(super) fn discovered_save_failed(&mut self, error: String) -> Task<Message> {
        self.announce(format!("Could not save discovered receiver: {error}"));
        Task::none()
    }

    pub(super) fn manual_setup(&mut self) -> Task<Message> {
        if self.address.trim().is_empty() {
            self.announce("Enter a receiver address first.");
            return Task::none();
        }
        let identity = denon_avr_domain::ReceiverIdentity {
            host: self.address.trim().to_owned(),
            model: None,
            friendly_name: (!self.name.trim().is_empty()).then(|| self.name.trim().to_owned()),
        };
        let name = identity
            .friendly_name
            .clone()
            .unwrap_or_else(|| identity.host.clone());
        let control = self.control.clone();
        self.announce(format!(
            "Saving {} as the current receiver…",
            identity_label(&identity)
        ));
        Task::perform(save_receiver(control, name, identity), Message::ManualSaved)
    }

    pub(super) fn manual_saved(
        &mut self,
        config: ConfiguredReceivers,
        selection: Selection,
    ) -> Task<Message> {
        self.configured = config;
        self.announce("Receiver saved; connecting…");
        self.update(Message::Select(selection))
    }

    pub(super) fn manual_save_failed(&mut self, error: String) -> Task<Message> {
        self.announce(format!("Could not save receiver: {error}"));
        Task::none()
    }
}

/// The configuration entry a discovered receiver is saved under: its model name,
/// or its address when it reports none.
pub(super) fn receiver_entry_for_discovered(
    receiver: &denon_avr_domain::DiscoveredReceiver,
) -> (String, ReceiverIdentity) {
    let name = receiver
        .model
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| receiver.address.host.clone());
    let mut identity = receiver.identity();
    identity.friendly_name = Some(name.clone());
    (name, identity)
}
