//! Receivers, capabilities, and health on the wire.
//!
//! None of these types has a field for an address. What an agent may learn about
//! a receiver is its id, its model, what it can do, and whether it is connected.

use denon_avr_application::{
    ApprovalHealth, AuditHealth, ConnectionStatus, PolicyHealth, ReceiverCapabilities,
    ReceiverSummary, ServiceHealth,
};
use denon_avr_domain::ReceiverId;
use serde::{Deserialize, Serialize};

/// A receiver id from the wire. A saved receiver's id is its configuration
/// entry's name, and one chosen by address carries the reserved `adhoc:` prefix,
/// which `ReceiverId::new` refuses, so each kind is read by its own constructor.
pub fn parse_receiver_id(text: &str) -> Result<ReceiverId, String> {
    if ReceiverId::is_reserved_name(text) {
        // The reserved prefix ends at its colon; what follows is the host.
        let host = text.split_once(':').map_or("", |(_, host)| host);
        return ReceiverId::ad_hoc(host).map_err(str::to_owned);
    }
    ReceiverId::new(text).map_err(str::to_owned)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionDto {
    Released,
    Connecting,
    Connected,
}

impl From<ConnectionStatus> for ConnectionDto {
    fn from(value: ConnectionStatus) -> Self {
        match value {
            ConnectionStatus::Released => Self::Released,
            ConnectionStatus::Connecting => Self::Connecting,
            ConnectionStatus::Connected => Self::Connected,
        }
    }
}

impl From<ConnectionDto> for ConnectionStatus {
    fn from(value: ConnectionDto) -> Self {
        match value {
            ConnectionDto::Released => Self::Released,
            ConnectionDto::Connecting => Self::Connecting,
            ConnectionDto::Connected => Self::Connected,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitiesDto {
    pub writable: bool,
    pub zone2_power: bool,
    pub source_catalog_read: bool,
    pub inputs: Vec<String>,
    pub surround_modes: Vec<String>,
}

impl From<&ReceiverCapabilities> for CapabilitiesDto {
    fn from(value: &ReceiverCapabilities) -> Self {
        Self {
            writable: value.writable,
            zone2_power: value.zone2_power,
            source_catalog_read: value.source_catalog_read,
            inputs: value.inputs.clone(),
            surround_modes: value.surround_modes.clone(),
        }
    }
}

impl From<CapabilitiesDto> for ReceiverCapabilities {
    fn from(value: CapabilitiesDto) -> Self {
        Self {
            writable: value.writable,
            zone2_power: value.zone2_power,
            source_catalog_read: value.source_catalog_read,
            inputs: value.inputs,
            surround_modes: value.surround_modes,
        }
    }
}

/// A saved receiver as an agent sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiverSummaryDto {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub capabilities: CapabilitiesDto,
    pub connection: ConnectionDto,
}

impl From<&ReceiverSummary> for ReceiverSummaryDto {
    fn from(value: &ReceiverSummary) -> Self {
        Self {
            id: value.id.as_str().to_owned(),
            model: value.model.clone(),
            capabilities: (&value.capabilities).into(),
            connection: value.connection.into(),
        }
    }
}

impl TryFrom<ReceiverSummaryDto> for ReceiverSummary {
    type Error = String;

    fn try_from(value: ReceiverSummaryDto) -> Result<Self, String> {
        Ok(Self {
            id: parse_receiver_id(&value.id)?,
            model: value.model,
            capabilities: value.capabilities.into(),
            connection: value.connection.into(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyHealthDto {
    NotConfigured,
    Active,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditDto {
    NotConfigured,
    Ok,
    Failing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDto {
    Unavailable,
}

/// Whether the Agent endpoint is serving, and why not when it is not. Only the
/// Operator's health response carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AgentEndpointDto {
    On,
    Off { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenStoreDto {
    Ok,
    Unavailable,
}

/// What the server knows that the service cannot: its endpoints and its token
/// store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerHealthDto {
    pub agent_endpoint: AgentEndpointDto,
    pub token_store: TokenStoreDto,
}

/// The answer to `GET /v1/health`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthDto {
    pub contract: u32,
    pub policy: PolicyHealthDto,
    pub audit: AuditDto,
    pub ledger_ready: bool,
    pub approval: ApprovalDto,
    /// Present on the Operator endpoint only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<ServerHealthDto>,
}

impl HealthDto {
    pub fn new(health: &ServiceHealth, server: Option<ServerHealthDto>) -> Self {
        Self {
            contract: crate::CONTRACT_VERSION,
            policy: match health.policy {
                PolicyHealth::NotConfigured => PolicyHealthDto::NotConfigured,
                PolicyHealth::Active => PolicyHealthDto::Active,
                PolicyHealth::Unavailable => PolicyHealthDto::Unavailable,
            },
            audit: match health.audit {
                AuditHealth::NotConfigured => AuditDto::NotConfigured,
                AuditHealth::Ok => AuditDto::Ok,
                AuditHealth::Failing => AuditDto::Failing,
            },
            ledger_ready: health.ledger_ready,
            // A kind of approval added later reads as unavailable here until this
            // crate knows it, which is the side that refuses.
            approval: match health.approval {
                ApprovalHealth::Unavailable => ApprovalDto::Unavailable,
                #[allow(unreachable_patterns)]
                _ => ApprovalDto::Unavailable,
            },
            server,
        }
    }

    /// The service's part of the answer, as the port returns it.
    pub fn service_health(&self) -> ServiceHealth {
        ServiceHealth {
            policy: match self.policy {
                PolicyHealthDto::NotConfigured => PolicyHealth::NotConfigured,
                PolicyHealthDto::Active => PolicyHealth::Active,
                PolicyHealthDto::Unavailable => PolicyHealth::Unavailable,
            },
            audit: match self.audit {
                AuditDto::NotConfigured => AuditHealth::NotConfigured,
                AuditDto::Ok => AuditHealth::Ok,
                AuditDto::Failing => AuditHealth::Failing,
            },
            ledger_ready: self.ledger_ready,
            approval: match self.approval {
                ApprovalDto::Unavailable => ApprovalHealth::Unavailable,
            },
        }
    }
}
