//! Receiver selection policy.

use crate::ports::{ConfigRepository, OperationError, OperationErrorKind, ReceiverDiscovery};
use denon_avr_domain::{ConfiguredReceivers, ReceiverId, ReceiverIdentity};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedReceiver {
    pub name: Option<String>,
    pub identity: ReceiverIdentity,
}

impl ResolvedReceiver {
    pub fn id(&self) -> Result<ReceiverId, OperationError> {
        receiver_id(self.name.as_deref(), &self.identity)
    }
}

/// The id under which a receiver is known to the control surface.
///
/// A saved receiver is identified by the name of its configuration entry, so its
/// id does not change when its address does. A receiver chosen only by address
/// gets an ad hoc id derived from that address; it is for Operator use and is
/// never listed to an Agent.
pub fn receiver_id(
    name: Option<&str>,
    identity: &ReceiverIdentity,
) -> Result<ReceiverId, OperationError> {
    match name {
        Some(name) => ReceiverId::new(name),
        None => ReceiverId::ad_hoc(&identity.host),
    }
    .map_err(|error| {
        OperationError::new(
            OperationErrorKind::InvalidSelection,
            "receiver identity",
            error,
        )
    })
}

pub fn resolve_status_receiver(
    config_repository: &impl ConfigRepository,
    discovery: &impl ReceiverDiscovery,
    requested_index: Option<usize>,
    manual_host: Option<&str>,
    timeout: Duration,
) -> Result<ResolvedReceiver, OperationError> {
    resolve_status_receiver_with_probe(
        config_repository,
        discovery,
        requested_index,
        manual_host,
        timeout,
        |_| true,
    )
}

pub fn resolve_status_receiver_with_probe(
    config_repository: &impl ConfigRepository,
    discovery: &impl ReceiverDiscovery,
    requested_index: Option<usize>,
    manual_host: Option<&str>,
    timeout: Duration,
    saved_probe: impl Fn(&ReceiverIdentity) -> bool,
) -> Result<ResolvedReceiver, OperationError> {
    let Some(host) = manual_host else {
        let config = config_repository.load()?;
        if requested_index.is_none() {
            if let Some((name, identity)) = config.current() {
                if saved_probe(identity) {
                    return Ok(ResolvedReceiver {
                        name: Some(name.to_owned()),
                        identity: identity.clone(),
                    });
                }
            }
        }
        let receivers = discovery.discover(timeout)?;
        let receiver = match requested_index {
            Some(index) => receivers.get(index).ok_or_else(|| {
                OperationError::new(
                    OperationErrorKind::InvalidSelection,
                    "selecting receiver",
                    "receiver selection is out of range",
                )
            })?,
            None if receivers.len() == 1 => &receivers[0],
            None if receivers.is_empty() => {
                return Err(OperationError::new(
                    OperationErrorKind::InvalidSelection,
                    "selecting receiver",
                    "no receivers discovered",
                ))
            }
            None => {
                return Err(OperationError::new(
                    OperationErrorKind::InvalidSelection,
                    "selecting receiver",
                    "receiver selection is required when multiple receivers are discovered",
                ))
            }
        };
        return Ok(ResolvedReceiver {
            name: None,
            identity: receiver.identity(),
        });
    };
    if host.trim().is_empty() {
        return Err(OperationError::new(
            OperationErrorKind::InvalidSelection,
            "selecting receiver",
            "--host must not be empty",
        ));
    }
    Ok(ResolvedReceiver {
        name: None,
        identity: ReceiverIdentity::ad_hoc(host),
    })
}

pub fn resolve_receiver(
    config: &ConfiguredReceivers,
    requested_name: Option<&str>,
    ad_hoc_host: Option<&str>,
) -> Result<ResolvedReceiver, OperationError> {
    if requested_name.is_some() && ad_hoc_host.is_some() {
        return Err(OperationError::new(
            OperationErrorKind::InvalidSelection,
            "selecting receiver",
            "a configured receiver name and --host cannot be combined",
        ));
    }
    if let Some(host) = ad_hoc_host {
        if host.trim().is_empty() {
            return Err(OperationError::new(
                OperationErrorKind::InvalidSelection,
                "selecting receiver",
                "--host must not be empty",
            ));
        }
        return Ok(ResolvedReceiver {
            name: None,
            identity: ReceiverIdentity::ad_hoc(host),
        });
    }
    let name = requested_name
        .map(str::to_owned)
        .or_else(|| config.current.clone())
        .ok_or_else(|| {
            OperationError::new(
                OperationErrorKind::InvalidSelection,
                "selecting receiver",
                "no receiver was requested and no current receiver is configured",
            )
        })?;
    let identity = config.receivers.get(&name).cloned().ok_or_else(|| {
        OperationError::new(
            OperationErrorKind::InvalidSelection,
            "selecting receiver",
            format!("receiver {name} is not configured"),
        )
    })?;
    Ok(ResolvedReceiver {
        name: Some(name),
        identity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn config() -> ConfiguredReceivers {
        ConfiguredReceivers {
            current: Some("living-room".into()),
            receivers: BTreeMap::from([(
                "living-room".into(),
                ReceiverIdentity::ad_hoc("192.0.2.10"),
            )]),
            ..ConfiguredReceivers::default()
        }
    }

    #[test]
    fn resolves_explicit_host_before_configuration() {
        let resolved = resolve_receiver(&config(), None, Some("192.0.2.20")).unwrap();
        assert_eq!(resolved.identity.host, "192.0.2.20");
        assert_eq!(resolved.name, None);
    }

    #[test]
    fn defaults_to_current_receiver() {
        let resolved = resolve_receiver(&config(), None, None).unwrap();
        assert_eq!(resolved.name.as_deref(), Some("living-room"));
    }

    #[test]
    fn a_saved_receiver_keeps_its_id_when_its_address_changes() {
        let before = resolve_receiver(&config(), None, None).unwrap();
        let mut moved = config();
        moved
            .receivers
            .insert("living-room".into(), ReceiverIdentity::ad_hoc("192.0.2.99"));
        let after = resolve_receiver(&moved, None, None).unwrap();

        assert_ne!(before.identity.host, after.identity.host);
        assert_eq!(before.id().unwrap(), after.id().unwrap());
        assert_eq!(before.id().unwrap().as_str(), "living-room");
        assert!(!before.id().unwrap().is_ad_hoc());
    }

    #[test]
    fn a_receiver_chosen_by_address_gets_an_ad_hoc_id() {
        let resolved = resolve_receiver(&config(), None, Some("192.0.2.20")).unwrap();
        let id = resolved.id().unwrap();
        assert!(id.is_ad_hoc());
        assert_eq!(id.as_str(), "adhoc:192.0.2.20");
        assert_ne!(
            id,
            resolve_receiver(&config(), None, None)
                .unwrap()
                .id()
                .unwrap()
        );
    }

    #[test]
    fn an_unusable_name_or_address_is_an_invalid_selection() {
        let identity = ReceiverIdentity::ad_hoc("192.0.2.10");
        for (name, identity) in [
            (Some("adhoc:192.0.2.10"), identity.clone()),
            (Some(""), identity.clone()),
            (None, ReceiverIdentity::ad_hoc("bad\nhost")),
        ] {
            let error = receiver_id(name, &identity).unwrap_err();
            assert_eq!(error.kind, OperationErrorKind::InvalidSelection);
        }
    }
}
