use crate::application::{OperationError, OperationErrorKind};
use crate::domain::{ConfiguredReceivers, ReceiverIdentity};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedReceiver {
    pub name: Option<String>,
    pub identity: ReceiverIdentity,
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
