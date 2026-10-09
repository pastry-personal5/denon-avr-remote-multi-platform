//! Discovery and ad hoc receivers: what `POST /v1/receivers/discover` is sent and
//! answers, and a receiver chosen only by its address.

use denon_avr_domain::{DiscoveredReceiver, ReceiverEndpoint, ReceiverIdentity};
use serde::{Deserialize, Serialize};

/// `POST /v1/receivers/discover`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoverRequest {
    /// How long to listen. The server caps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredDto {
    pub host: String,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unique_service_name: Option<String>,
}

impl From<&DiscoveredReceiver> for DiscoveredDto {
    fn from(found: &DiscoveredReceiver) -> Self {
        Self {
            host: found.address.host.clone(),
            port: found.address.port,
            location: found.location.clone(),
            server: found.server.clone(),
            model: found.model.clone(),
            search_target: found.search_target.clone(),
            unique_service_name: found.unique_service_name.clone(),
        }
    }
}

impl From<DiscoveredDto> for DiscoveredReceiver {
    fn from(found: DiscoveredDto) -> Self {
        Self {
            address: ReceiverEndpoint {
                host: found.host,
                port: found.port,
            },
            location: found.location,
            server: found.server,
            model: found.model,
            search_target: found.search_target,
            unique_service_name: found.unique_service_name,
        }
    }
}

/// `POST /v1/receivers/discover`'s answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredListDto {
    pub receivers: Vec<DiscoveredDto>,
}

/// `POST /v1/receivers/ad-hoc`: use a receiver chosen only by its address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdHocRequest {
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub friendly_name: Option<String>,
}

impl From<&ReceiverIdentity> for AdHocRequest {
    fn from(identity: &ReceiverIdentity) -> Self {
        Self {
            host: identity.host.clone(),
            model: identity.model.clone(),
            friendly_name: identity.friendly_name.clone(),
        }
    }
}

impl From<AdHocRequest> for ReceiverIdentity {
    fn from(request: AdHocRequest) -> Self {
        Self {
            host: request.host,
            model: request.model,
            friendly_name: request.friendly_name,
        }
    }
}

/// The id an ad hoc receiver was given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdHocResponse {
    pub id: String,
}
