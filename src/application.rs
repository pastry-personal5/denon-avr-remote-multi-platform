//! Application-facing receiver operations. This module contains policy, while
//! protocol and transport modules remain usable independently.
use crate::config::{self, Config, ConfigError, ReceiverIdentity};
use crate::discovery::{self, DiscoveredReceiver};
#[allow(deprecated)]
use crate::status::{query_main_zone, render, MainZoneStatus, TcpAvrTransport};
use crate::transport::AsyncAvrTransport;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct ApplicationService {
    config_path: PathBuf,
    connect_timeout: Duration,
    read_timeout: Duration,
}
impl Default for ApplicationService {
    fn default() -> Self {
        Self::new(config::default_path())
    }
}
impl ApplicationService {
    pub fn new(config_path: impl Into<PathBuf>) -> Self {
        Self {
            config_path: config_path.into(),
            connect_timeout: Duration::from_secs(2),
            read_timeout: Duration::from_secs(1),
        }
    }
    pub fn with_timeouts(mut self, connect: Duration, read: Duration) -> Self {
        self.connect_timeout = connect;
        self.read_timeout = read;
        self
    }
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }
    pub fn load_saved(&self) -> Result<Option<Config>, ConfigError> {
        config::load(&self.config_path)
    }
    pub fn discover(&self, timeout: Duration) -> Result<Vec<DiscoveredReceiver>, String> {
        discovery::discover(timeout).map_err(|e| e.to_string())
    }
    pub fn select_receiver<'a>(
        &self,
        receivers: &'a [DiscoveredReceiver],
        index: Option<usize>,
    ) -> Result<&'a DiscoveredReceiver, String> {
        if receivers.is_empty() {
            return Err("no receivers discovered".into());
        }
        match index {
            Some(i) => receivers
                .get(i)
                .ok_or_else(|| "receiver selection is out of range".into()),
            None if receivers.len() == 1 => Ok(&receivers[0]),
            None => {
                Err("receiver selection is required when multiple receivers are discovered".into())
            }
        }
    }
    pub fn identity_from_discovery(receiver: &DiscoveredReceiver) -> ReceiverIdentity {
        ReceiverIdentity {
            host: receiver.address.ip().to_string(),
            model: receiver.model.clone(),
            friendly_name: None,
        }
    }
    pub fn query_identity(
        &self,
        identity: ReceiverIdentity,
    ) -> Result<(ReceiverIdentity, MainZoneStatus), String> {
        #[allow(deprecated)]
        let mut transport =
            TcpAvrTransport::connect(&identity.host, self.connect_timeout, self.read_timeout)
                .map_err(|e| e.to_string())?;
        #[allow(deprecated)]
        let status = query_main_zone(&mut transport);
        config::save(
            &self.config_path,
            &Config {
                receiver: identity.clone(),
            },
        )
        .map_err(|e| e.to_string())?;
        Ok((identity, status))
    }
    pub fn query_main_zone(&self, identity: ReceiverIdentity) -> Result<MainZoneStatus, String> {
        self.query_identity(identity).map(|(_, status)| status)
    }
    pub async fn query_main_zone_async<T: AsyncAvrTransport + ?Sized>(
        &self,
        transport: &T,
    ) -> MainZoneStatus {
        async fn field<T: AsyncAvrTransport + ?Sized>(
            transport: &T,
            command: &str,
            parser: fn(&str) -> Result<String, String>,
        ) -> crate::status::StatusField {
            let result = match transport.request(command).await {
                Ok(value) => parser(&value).map_err(crate::transport::TransportError::Unexpected),
                Err(error) => Err(error),
            };
            match result {
                Ok(value) => crate::status::StatusField {
                    value: Some(value),
                    error: None,
                },
                Err(error) => crate::status::StatusField {
                    value: None,
                    error: Some(error.to_string()),
                },
            }
        }
        MainZoneStatus {
            power: field(transport, "PW?", crate::response::parse_power).await,
            input: field(transport, "SI?", crate::response::parse_input).await,
            volume: field(transport, "MV?", crate::response::parse_volume).await,
            mute: field(transport, "MU?", crate::response::parse_mute).await,
            surround_mode: field(transport, "MS?", crate::response::parse_surround).await,
        }
    }
    pub fn query_status(
        &self,
        requested: Option<usize>,
        manual_host: Option<String>,
        timeout: Duration,
    ) -> Result<(ReceiverIdentity, MainZoneStatus), String> {
        if let Some(host) = manual_host {
            return self.query_identity(ReceiverIdentity {
                host,
                model: None,
                friendly_name: None,
            });
        }
        if let Ok(Some(saved)) = self.load_saved() {
            if requested.is_none() {
                if let Ok(result) = self.query_identity(saved.receiver) {
                    return Ok(result);
                }
            }
        }
        let receivers = self.discover(timeout)?;
        let receiver = self.select_receiver(&receivers, requested)?;
        self.query_identity(Self::identity_from_discovery(receiver))
    }
    pub fn render_status(&self, status: &MainZoneStatus) -> String {
        render(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    fn receiver(host: &str) -> DiscoveredReceiver {
        DiscoveredReceiver {
            address: format!("{host}:23").parse::<SocketAddr>().unwrap(),
            location: None,
            server: Some("Denon".into()),
            model: Some("AVR-X3800H".into()),
            search_target: None,
            unique_service_name: None,
        }
    }

    #[test]
    fn selects_the_only_receiver_and_rejects_ambiguous_selection() {
        let service = ApplicationService::new("target/test-config.yaml");
        let one = [receiver("192.0.2.10")];
        assert_eq!(
            service
                .select_receiver(&one, None)
                .unwrap()
                .address
                .ip()
                .to_string(),
            "192.0.2.10"
        );
        let two = [receiver("192.0.2.10"), receiver("192.0.2.11")];
        assert!(service.select_receiver(&two, None).is_err());
        assert_eq!(
            service
                .select_receiver(&two, Some(1))
                .unwrap()
                .address
                .ip()
                .to_string(),
            "192.0.2.11"
        );
    }
}
