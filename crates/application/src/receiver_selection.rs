//! Receiver selection policy.

use crate::ports::{ConfigRepository, OperationError, OperationErrorKind, ReceiverDiscovery};
use denon_avr_domain::{ConfiguredReceivers, ReceiverIdentity};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedReceiver {
    pub name: Option<String>,
    pub identity: ReceiverIdentity,
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
}
