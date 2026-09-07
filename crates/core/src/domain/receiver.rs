use std::collections::BTreeMap;
use std::net::SocketAddr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverIdentity {
    pub host: String,
    pub model: Option<String>,
    pub friendly_name: Option<String>,
}

impl ReceiverIdentity {
    pub fn ad_hoc(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            model: None,
            friendly_name: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConfiguredReceivers {
    pub current: Option<String>,
    pub receivers: BTreeMap<String, ReceiverIdentity>,
}

impl ConfiguredReceivers {
    pub fn current(&self) -> Option<(&str, &ReceiverIdentity)> {
        let name = self.current.as_deref()?;
        self.receivers.get(name).map(|identity| (name, identity))
    }

    pub fn validate(&self) -> Result<(), String> {
        for (name, receiver) in &self.receivers {
            if name.trim().is_empty() {
                return Err("receiver name must not be empty".into());
            }
            if receiver.host.trim().is_empty() {
                return Err(format!("receiver {name} has an empty host"));
            }
            if name.contains(['\r', '\n']) || receiver.host.contains(['\r', '\n']) {
                return Err(format!("receiver {name} contains a line break"));
            }
        }
        if let Some(current) = &self.current {
            if !self.receivers.contains_key(current) {
                return Err(format!("current receiver {current} is not configured"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredReceiver {
    pub address: SocketAddr,
    pub location: Option<String>,
    pub server: Option<String>,
    pub model: Option<String>,
    pub search_target: Option<String>,
    pub unique_service_name: Option<String>,
}

impl DiscoveredReceiver {
    pub fn identity(&self) -> ReceiverIdentity {
        ReceiverIdentity {
            host: self.address.ip().to_string(),
            model: self.model.clone(),
            friendly_name: None,
        }
    }
}
