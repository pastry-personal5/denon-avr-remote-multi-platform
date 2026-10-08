//! The Operator's resources: configuration, discovery, the inspection reads, the
//! policy, the audit log, and tokens.
//!
//! Nothing here is served to an agent, so these types carry whatever the
//! Operator needs, addresses and error text included.

use crate::operations::DispatchDto;
use crate::receivers::parse_receiver_id;
use crate::state::{CatalogEvidenceDto, FreshnessDto};
use crate::values::{system_time_from_ms, system_time_ms};
use denon_avr_application::audit::{core_field_name, parse_core_field};
use denon_avr_application::{
    AgentLabel, AuditCursor, AuditDecision, AuditEntry, AuditEvent, AuditPage, AuditRecord,
    EndpointKind, IssuedToken, PolicyDigest, PolicyView, Principal, Readiness, RefusalReason,
    TokenId, TokenRecord, TokenSecret,
};
use denon_avr_domain::{
    AudioInformation, AudysseyInformation, ChannelSlot, ChannelSlotState, ConfiguredReceivers,
    DiscoveredReceiver, FieldError, FieldErrorKind, FieldStatus, Freshness,
    HttpInformationSnapshot, OperationId, QuickSelectName, QuickSelectNameObservation,
    QuickSelectNameResponseEvidence, ReceiverEndpoint, ReceiverIdentity, SoundModeFavorite,
    VideoInformation, WallTime,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

// ---- Configuration ----

/// A saved receiver: where it is, and what it calls itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityDto {
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub friendly_name: Option<String>,
}

impl From<&ReceiverIdentity> for IdentityDto {
    fn from(identity: &ReceiverIdentity) -> Self {
        Self {
            host: identity.host.clone(),
            model: identity.model.clone(),
            friendly_name: identity.friendly_name.clone(),
        }
    }
}

impl From<IdentityDto> for ReceiverIdentity {
    fn from(identity: IdentityDto) -> Self {
        Self {
            host: identity.host,
            model: identity.model,
            friendly_name: identity.friendly_name,
        }
    }
}

/// The receiver configuration, as `GET /v1/config` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    #[serde(default)]
    pub receivers: BTreeMap<String, IdentityDto>,
    #[serde(default)]
    pub sound_mode_favorites: BTreeMap<String, Vec<String>>,
}

impl From<&ConfiguredReceivers> for ConfigDto {
    fn from(config: &ConfiguredReceivers) -> Self {
        Self {
            current: config.current.clone(),
            receivers: config
                .receivers
                .iter()
                .map(|(name, identity)| (name.clone(), identity.into()))
                .collect(),
            sound_mode_favorites: config
                .sound_mode_favorites
                .iter()
                .map(|(name, favorites)| {
                    (
                        name.clone(),
                        favorites
                            .iter()
                            .map(|favorite| favorite.mode.clone())
                            .collect(),
                    )
                })
                .collect(),
        }
    }
}

impl TryFrom<ConfigDto> for ConfiguredReceivers {
    type Error = String;

    fn try_from(config: ConfigDto) -> Result<Self, String> {
        let favorites = config
            .sound_mode_favorites
            .into_iter()
            .map(|(name, modes)| {
                let set = modes
                    .into_iter()
                    .map(|mode| SoundModeFavorite::new(mode).map_err(str::to_owned))
                    .collect::<Result<BTreeSet<_>, String>>()?;
                Ok((name, set))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        Ok(Self {
            current: config.current,
            receivers: config
                .receivers
                .into_iter()
                .map(|(name, identity)| (name, identity.into()))
                .collect(),
            sound_mode_favorites: favorites,
        })
    }
}

/// A saved receiver, as `PUT /v1/config` is sent it: strict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityRequest {
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub friendly_name: Option<String>,
}

/// The configuration, as `PUT /v1/config` is sent it. It is the same shape as the
/// answer, but a field it does not know is refused by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    #[serde(default)]
    pub receivers: BTreeMap<String, IdentityRequest>,
    #[serde(default)]
    pub sound_mode_favorites: BTreeMap<String, Vec<String>>,
}

impl From<&ConfiguredReceivers> for ConfigRequest {
    fn from(config: &ConfiguredReceivers) -> Self {
        let dto = ConfigDto::from(config);
        Self {
            current: dto.current,
            receivers: dto
                .receivers
                .into_iter()
                .map(|(name, identity)| {
                    (
                        name,
                        IdentityRequest {
                            host: identity.host,
                            model: identity.model,
                            friendly_name: identity.friendly_name,
                        },
                    )
                })
                .collect(),
            sound_mode_favorites: dto.sound_mode_favorites,
        }
    }
}

impl TryFrom<ConfigRequest> for ConfiguredReceivers {
    type Error = String;

    fn try_from(request: ConfigRequest) -> Result<Self, String> {
        ConfigDto {
            current: request.current,
            receivers: request
                .receivers
                .into_iter()
                .map(|(name, identity)| {
                    (
                        name,
                        IdentityDto {
                            host: identity.host,
                            model: identity.model,
                            friendly_name: identity.friendly_name,
                        },
                    )
                })
                .collect(),
            sound_mode_favorites: request.sound_mode_favorites,
        }
        .try_into()
    }
}

// ---- Discovery and ad hoc receivers ----

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

// ---- The inspection reads ----

/// `POST /v1/receivers/{id}/refresh`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessDto {
    pub ready: bool,
    pub degraded: bool,
    pub detail: String,
}

impl From<&Readiness> for ReadinessDto {
    fn from(readiness: &Readiness) -> Self {
        Self {
            ready: readiness.ready,
            degraded: readiness.degraded,
            detail: readiness.detail.clone(),
        }
    }
}

impl From<ReadinessDto> for Readiness {
    fn from(readiness: ReadinessDto) -> Self {
        Self {
            ready: readiness.ready,
            degraded: readiness.degraded,
            detail: readiness.detail,
        }
    }
}

fn freshness_dto(freshness: Freshness) -> FreshnessDto {
    match freshness {
        Freshness::Unknown => FreshnessDto::Unknown,
        Freshness::Live => FreshnessDto::Live,
        Freshness::Partial => FreshnessDto::Partial,
        Freshness::Invalidated => FreshnessDto::Invalidated,
    }
}

fn freshness_of(freshness: FreshnessDto) -> Freshness {
    match freshness {
        FreshnessDto::Unknown => Freshness::Unknown,
        FreshnessDto::Live => Freshness::Live,
        FreshnessDto::Partial => Freshness::Partial,
        FreshnessDto::Invalidated => Freshness::Invalidated,
    }
}

/// The receiver's Quick Select names: four slots, each with a name and the source
/// it recalls, when the receiver reported them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickSelectNamesDto {
    pub names: Vec<Option<String>>,
    pub sources: Vec<Option<String>>,
    pub freshness: FreshnessDto,
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub raw_response: String,
    pub response: CatalogEvidenceDto,
}

impl From<&QuickSelectNameObservation> for QuickSelectNamesDto {
    fn from(observation: &QuickSelectNameObservation) -> Self {
        Self {
            names: observation
                .names
                .iter()
                .map(|name| name.as_ref().map(|name| name.as_str().to_owned()))
                .collect(),
            sources: observation.sources.to_vec(),
            freshness: freshness_dto(observation.freshness),
            generation: observation.generation,
            observed_at_ms: system_time_ms(observation.observed_at),
            error: observation.error.clone(),
            raw_response: observation.raw_response.clone(),
            response: match observation.response_evidence {
                QuickSelectNameResponseEvidence::Complete => CatalogEvidenceDto::Complete,
                QuickSelectNameResponseEvidence::Partial => CatalogEvidenceDto::Partial,
                QuickSelectNameResponseEvidence::Unsupported => CatalogEvidenceDto::Unsupported,
                QuickSelectNameResponseEvidence::Malformed => CatalogEvidenceDto::Malformed,
                QuickSelectNameResponseEvidence::Timeout => CatalogEvidenceDto::Timeout,
                QuickSelectNameResponseEvidence::Disconnected => CatalogEvidenceDto::Disconnected,
            },
        }
    }
}

impl TryFrom<QuickSelectNamesDto> for QuickSelectNameObservation {
    type Error = String;

    fn try_from(dto: QuickSelectNamesDto) -> Result<Self, String> {
        let slots = |what: &str, count: usize| {
            if count == 4 {
                Ok(())
            } else {
                Err(format!("{what} has {count} slots, not four"))
            }
        };
        slots("names", dto.names.len())?;
        slots("sources", dto.sources.len())?;
        let mut names: [Option<QuickSelectName>; 4] = [None, None, None, None];
        for (slot, name) in dto.names.into_iter().enumerate() {
            names[slot] = name
                .map(|name| QuickSelectName::new(name).map_err(str::to_owned))
                .transpose()?;
        }
        let mut sources: [Option<String>; 4] = [None, None, None, None];
        for (slot, source) in dto.sources.into_iter().enumerate() {
            sources[slot] = source;
        }
        Ok(Self {
            names,
            sources,
            freshness: freshness_of(dto.freshness),
            generation: dto.generation,
            observed_at: system_time_from_ms(dto.observed_at_ms),
            error: dto.error,
            raw_response: dto.raw_response,
            response_evidence: match dto.response {
                CatalogEvidenceDto::Complete => QuickSelectNameResponseEvidence::Complete,
                CatalogEvidenceDto::Partial => QuickSelectNameResponseEvidence::Partial,
                CatalogEvidenceDto::Unsupported => QuickSelectNameResponseEvidence::Unsupported,
                CatalogEvidenceDto::Malformed => QuickSelectNameResponseEvidence::Malformed,
                CatalogEvidenceDto::Timeout => QuickSelectNameResponseEvidence::Timeout,
                CatalogEvidenceDto::Disconnected => QuickSelectNameResponseEvidence::Disconnected,
            },
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldErrorKindDto {
    Unsupported,
    Timeout,
    Disconnected,
    Malformed,
    Unavailable,
}

/// A value, or why there is none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldStatusDto<T> {
    Value(T),
    Unavailable {
        kind: FieldErrorKindDto,
        message: String,
    },
}

impl<T: Clone> From<&FieldStatus<T>> for FieldStatusDto<T> {
    fn from(status: &FieldStatus<T>) -> Self {
        match status {
            FieldStatus::Value(value) => Self::Value(value.clone()),
            FieldStatus::Unavailable(error) => Self::Unavailable {
                kind: match error.kind {
                    FieldErrorKind::Unsupported => FieldErrorKindDto::Unsupported,
                    FieldErrorKind::Timeout => FieldErrorKindDto::Timeout,
                    FieldErrorKind::Disconnected => FieldErrorKindDto::Disconnected,
                    FieldErrorKind::Malformed => FieldErrorKindDto::Malformed,
                    FieldErrorKind::Unavailable => FieldErrorKindDto::Unavailable,
                },
                message: error.message.clone(),
            },
        }
    }
}

impl<T> From<FieldStatusDto<T>> for FieldStatus<T> {
    fn from(status: FieldStatusDto<T>) -> Self {
        match status {
            FieldStatusDto::Value(value) => Self::Value(value),
            FieldStatusDto::Unavailable { kind, message } => Self::Unavailable(FieldError {
                kind: match kind {
                    FieldErrorKindDto::Unsupported => FieldErrorKind::Unsupported,
                    FieldErrorKindDto::Timeout => FieldErrorKind::Timeout,
                    FieldErrorKindDto::Disconnected => FieldErrorKind::Disconnected,
                    FieldErrorKindDto::Malformed => FieldErrorKind::Malformed,
                    FieldErrorKindDto::Unavailable => FieldErrorKind::Unavailable,
                },
                message,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotStateDto {
    Active,
    Available,
    Absent,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelSlotDto {
    pub label: String,
    pub state: SlotStateDto,
}

impl From<&ChannelSlot> for ChannelSlotDto {
    fn from(slot: &ChannelSlot) -> Self {
        Self {
            label: slot.label.clone(),
            state: match slot.state {
                ChannelSlotState::Active => SlotStateDto::Active,
                ChannelSlotState::Available => SlotStateDto::Available,
                ChannelSlotState::Absent => SlotStateDto::Absent,
                ChannelSlotState::Unknown => SlotStateDto::Unknown,
            },
        }
    }
}

impl From<ChannelSlotDto> for ChannelSlot {
    fn from(slot: ChannelSlotDto) -> Self {
        Self {
            label: slot.label,
            state: match slot.state {
                SlotStateDto::Active => ChannelSlotState::Active,
                SlotStateDto::Available => ChannelSlotState::Available,
                SlotStateDto::Absent => ChannelSlotState::Absent,
                SlotStateDto::Unknown => ChannelSlotState::Unknown,
            },
        }
    }
}

type Text = FieldStatusDto<String>;

fn text(status: &FieldStatus<String>) -> Text {
    status.into()
}

fn slots(status: &FieldStatus<Vec<ChannelSlot>>) -> FieldStatusDto<Vec<ChannelSlotDto>> {
    match status {
        FieldStatus::Value(slots) => {
            FieldStatusDto::Value(slots.iter().map(ChannelSlotDto::from).collect())
        }
        FieldStatus::Unavailable(error) => {
            FieldStatusDto::from(&FieldStatus::<()>::Unavailable(error.clone())).retype()
        }
    }
}

impl<T> FieldStatusDto<T> {
    /// The same unavailable status for another value type.
    fn retype<U>(self) -> FieldStatusDto<U> {
        match self {
            Self::Value(_) => unreachable!("only an unavailable status is retyped"),
            Self::Unavailable { kind, message } => FieldStatusDto::Unavailable { kind, message },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioDto {
    pub input_mode: Text,
    pub output: Text,
    pub signal: Text,
    pub sound: Text,
    pub sample_rate: Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoDto {
    pub monitor: Text,
    pub hdmi_input: Text,
    pub hdmi_output: Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudysseyDto {
    pub multeq: Text,
    pub dynamic_eq: Text,
    pub dynamic_volume: Text,
}

/// The receiver's audio, video, and Audyssey information, read over HTTP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpInformationDto {
    pub input_slots: FieldStatusDto<Vec<ChannelSlotDto>>,
    pub output_slots: FieldStatusDto<Vec<ChannelSlotDto>>,
    pub audio: AudioDto,
    pub video: VideoDto,
    pub audyssey: AudysseyDto,
    pub freshness: FreshnessDto,
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl From<&HttpInformationSnapshot> for HttpInformationDto {
    fn from(info: &HttpInformationSnapshot) -> Self {
        Self {
            input_slots: slots(&info.input_slots),
            output_slots: slots(&info.output_slots),
            audio: AudioDto {
                input_mode: text(&info.audio.input_mode),
                output: text(&info.audio.output),
                signal: text(&info.audio.signal),
                sound: text(&info.audio.sound),
                sample_rate: text(&info.audio.sample_rate),
            },
            video: VideoDto {
                monitor: text(&info.video.monitor),
                hdmi_input: text(&info.video.hdmi_input),
                hdmi_output: text(&info.video.hdmi_output),
            },
            audyssey: AudysseyDto {
                multeq: text(&info.audyssey.multeq),
                dynamic_eq: text(&info.audyssey.dynamic_eq),
                dynamic_volume: text(&info.audyssey.dynamic_volume),
            },
            freshness: freshness_dto(info.freshness),
            generation: info.generation,
            observed_at_ms: system_time_ms(info.observed_at),
            error: info.error.clone(),
        }
    }
}

impl From<HttpInformationDto> for HttpInformationSnapshot {
    fn from(dto: HttpInformationDto) -> Self {
        let slots = |status: FieldStatusDto<Vec<ChannelSlotDto>>| match status {
            FieldStatusDto::Value(slots) => {
                FieldStatus::Value(slots.into_iter().map(ChannelSlot::from).collect())
            }
            other => other.retype::<()>().into_unavailable(),
        };
        Self {
            input_slots: slots(dto.input_slots),
            output_slots: slots(dto.output_slots),
            audio: AudioInformation {
                input_mode: dto.audio.input_mode.into(),
                output: dto.audio.output.into(),
                signal: dto.audio.signal.into(),
                sound: dto.audio.sound.into(),
                sample_rate: dto.audio.sample_rate.into(),
            },
            video: VideoInformation {
                monitor: dto.video.monitor.into(),
                hdmi_input: dto.video.hdmi_input.into(),
                hdmi_output: dto.video.hdmi_output.into(),
            },
            audyssey: AudysseyInformation {
                multeq: dto.audyssey.multeq.into(),
                dynamic_eq: dto.audyssey.dynamic_eq.into(),
                dynamic_volume: dto.audyssey.dynamic_volume.into(),
            },
            freshness: freshness_of(dto.freshness),
            generation: dto.generation,
            observed_at: system_time_from_ms(dto.observed_at_ms),
            error: dto.error,
        }
    }
}

impl FieldStatusDto<()> {
    /// An unavailable status as the domain's, for any value type.
    fn into_unavailable<T>(self) -> FieldStatus<T> {
        match self {
            Self::Value(()) => unreachable!("only an unavailable status is converted"),
            Self::Unavailable { kind, message } => {
                FieldStatusDto::<T>::Unavailable { kind, message }.into()
            }
        }
    }
}

// ---- Policy ----

/// The policy in force, or why there is none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loaded_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl From<&PolicyView> for PolicyDto {
    fn from(view: &PolicyView) -> Self {
        Self {
            digest: view.digest.map(|digest| digest.to_string()),
            loaded_at_ms: view.loaded_at.map(WallTime::as_millis),
            text: view.text.clone(),
            error: view.error.clone(),
        }
    }
}

impl TryFrom<PolicyDto> for PolicyView {
    type Error = String;

    fn try_from(dto: PolicyDto) -> Result<Self, String> {
        Ok(Self {
            digest: dto
                .digest
                .map(|hex| {
                    PolicyDigest::from_hex(&hex)
                        .ok_or_else(|| "the policy digest is not 64 hexadecimal digits".to_owned())
                })
                .transpose()?,
            loaded_at: dto.loaded_at_ms.map(WallTime),
            text: dto.text,
            error: dto.error,
        })
    }
}

// ---- The audit log ----

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalDto {
    Operator,
    Agent(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditDecisionDto {
    Allow,
    RequireApproval,
    Deny,
}

/// One audit event, tagged by its kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuditEventDto {
    Decided {
        intent: String,
        decision: AuditDecisionDto,
        reasons: Vec<String>,
        rules: Vec<String>,
        baseline: Vec<String>,
        #[serde(default)]
        policy: Option<String>,
    },
    Dispatching {
        intent: String,
        #[serde(default)]
        before: Option<i16>,
        #[serde(default)]
        target: Option<i16>,
        precondition_fields: Vec<String>,
    },
    Finished {
        status: String,
        dispatch: DispatchDto,
        confirmed: bool,
        #[serde(default)]
        reason: Option<String>,
    },
    PolicyLoaded {
        digest: String,
    },
    PolicyLoadFailed {
        error: String,
    },
    AccessRefused {
        endpoint: String,
        reason: String,
        #[serde(default)]
        peer_uid: Option<u32>,
        #[serde(default)]
        resource: Option<String>,
        suppressed: u32,
    },
    TokenIssued {
        id: String,
        label: String,
    },
    TokenRevoked {
        id: String,
        label: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRecordDto {
    pub schema: u32,
    pub run_ms: u64,
    pub at_ms: u64,
    #[serde(default)]
    pub operation: Option<u64>,
    #[serde(default)]
    pub principal: Option<PrincipalDto>,
    #[serde(default)]
    pub receiver: Option<String>,
    pub event: AuditEventDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntryDto {
    pub seq: u64,
    #[serde(flatten)]
    pub record: AuditRecordDto,
}

/// A page of the audit log, newest first. `next` is the cursor of the page after.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditPageDto {
    pub entries: Vec<AuditEntryDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<u64>,
}

fn field_names(fields: &[denon_avr_domain::CoreField]) -> Vec<String> {
    fields
        .iter()
        .map(|field| core_field_name(*field).to_owned())
        .collect()
}

fn fields_of(names: Vec<String>) -> Vec<denon_avr_domain::CoreField> {
    // A name this code does not know is dropped: it is informational.
    names
        .iter()
        .filter_map(|name| parse_core_field(name))
        .collect()
}

impl From<&AuditEvent> for AuditEventDto {
    /// An exhaustive match, so a new kind of event does not compile until it has a
    /// wire form.
    fn from(event: &AuditEvent) -> Self {
        match event {
            AuditEvent::Decided {
                intent,
                decision,
                reasons,
                rules,
                baseline,
                policy,
            } => Self::Decided {
                intent: intent.clone(),
                decision: match decision {
                    AuditDecision::Allow => AuditDecisionDto::Allow,
                    AuditDecision::RequireApproval => AuditDecisionDto::RequireApproval,
                    AuditDecision::Deny => AuditDecisionDto::Deny,
                },
                reasons: reasons.clone(),
                rules: rules.clone(),
                baseline: field_names(baseline),
                policy: policy.map(|digest| digest.to_string()),
            },
            AuditEvent::Dispatching {
                intent,
                before,
                target,
                precondition_fields,
            } => Self::Dispatching {
                intent: intent.clone(),
                before: *before,
                target: *target,
                precondition_fields: field_names(precondition_fields),
            },
            AuditEvent::Finished {
                status,
                dispatch,
                confirmed,
                reason,
            } => Self::Finished {
                status: status.clone(),
                dispatch: dispatch.into(),
                confirmed: *confirmed,
                reason: reason.clone(),
            },
            AuditEvent::PolicyLoaded { digest } => Self::PolicyLoaded {
                digest: digest.to_string(),
            },
            AuditEvent::PolicyLoadFailed { error } => Self::PolicyLoadFailed {
                error: error.clone(),
            },
            AuditEvent::AccessRefused {
                endpoint,
                reason,
                peer_uid,
                resource,
                suppressed,
            } => Self::AccessRefused {
                endpoint: endpoint.as_str().to_owned(),
                reason: reason.as_str().to_owned(),
                peer_uid: *peer_uid,
                resource: resource.clone(),
                suppressed: *suppressed,
            },
            AuditEvent::TokenIssued { id, label } => Self::TokenIssued {
                id: id.as_str().to_owned(),
                label: label.as_str().to_owned(),
            },
            AuditEvent::TokenRevoked { id, label } => Self::TokenRevoked {
                id: id.as_str().to_owned(),
                label: label.as_str().to_owned(),
            },
        }
    }
}

impl TryFrom<AuditEventDto> for AuditEvent {
    type Error = String;

    fn try_from(dto: AuditEventDto) -> Result<Self, String> {
        let digest = |hex: &str| {
            PolicyDigest::from_hex(hex)
                .ok_or_else(|| "a policy digest is not 64 hexadecimal digits".to_owned())
        };
        Ok(match dto {
            AuditEventDto::Decided {
                intent,
                decision,
                reasons,
                rules,
                baseline,
                policy,
            } => Self::Decided {
                intent,
                decision: match decision {
                    AuditDecisionDto::Allow => AuditDecision::Allow,
                    AuditDecisionDto::RequireApproval => AuditDecision::RequireApproval,
                    AuditDecisionDto::Deny => AuditDecision::Deny,
                },
                reasons,
                rules,
                baseline: fields_of(baseline),
                policy: policy.as_deref().map(digest).transpose()?,
            },
            AuditEventDto::Dispatching {
                intent,
                before,
                target,
                precondition_fields,
            } => Self::Dispatching {
                intent,
                before,
                target,
                precondition_fields: fields_of(precondition_fields),
            },
            AuditEventDto::Finished {
                status,
                dispatch,
                confirmed,
                reason,
            } => Self::Finished {
                status,
                dispatch: dispatch.into(),
                confirmed,
                reason,
            },
            AuditEventDto::PolicyLoaded { digest: hex } => Self::PolicyLoaded {
                digest: digest(&hex)?,
            },
            AuditEventDto::PolicyLoadFailed { error } => Self::PolicyLoadFailed { error },
            AuditEventDto::AccessRefused {
                endpoint,
                reason,
                peer_uid,
                resource,
                suppressed,
            } => Self::AccessRefused {
                endpoint: EndpointKind::parse(&endpoint).ok_or("an unknown endpoint")?,
                reason: RefusalReason::parse(&reason).ok_or("an unknown refusal reason")?,
                peer_uid,
                resource,
                suppressed,
            },
            AuditEventDto::TokenIssued { id, label } => Self::TokenIssued {
                id: TokenId::new(id).map_err(str::to_owned)?,
                label: AgentLabel::new(label).map_err(str::to_owned)?,
            },
            AuditEventDto::TokenRevoked { id, label } => Self::TokenRevoked {
                id: TokenId::new(id).map_err(str::to_owned)?,
                label: AgentLabel::new(label).map_err(str::to_owned)?,
            },
        })
    }
}

impl From<&AuditEntry> for AuditEntryDto {
    fn from(entry: &AuditEntry) -> Self {
        let record = &entry.record;
        Self {
            seq: entry.seq,
            record: AuditRecordDto {
                schema: record.schema,
                run_ms: record.run.as_millis(),
                at_ms: record.at.as_millis(),
                operation: record.operation.map(|id| id.0),
                principal: record.principal.as_ref().map(|principal| match principal {
                    Principal::Operator => PrincipalDto::Operator,
                    Principal::Agent(label) => PrincipalDto::Agent(label.as_str().to_owned()),
                }),
                receiver: record.receiver.as_ref().map(|id| id.as_str().to_owned()),
                event: (&record.event).into(),
            },
        }
    }
}

impl TryFrom<AuditEntryDto> for AuditEntry {
    type Error = String;

    fn try_from(dto: AuditEntryDto) -> Result<Self, String> {
        let record = dto.record;
        Ok(Self {
            seq: dto.seq,
            record: AuditRecord {
                schema: record.schema,
                run: WallTime(record.run_ms),
                at: WallTime(record.at_ms),
                operation: record.operation.map(OperationId),
                principal: record
                    .principal
                    .map(|principal| match principal {
                        PrincipalDto::Operator => Ok(Principal::Operator),
                        PrincipalDto::Agent(label) => AgentLabel::new(label)
                            .map(Principal::Agent)
                            .map_err(str::to_owned),
                    })
                    .transpose()?,
                receiver: record
                    .receiver
                    .as_deref()
                    .map(parse_receiver_id)
                    .transpose()?,
                event: record.event.try_into()?,
            },
        })
    }
}

impl From<&AuditPage> for AuditPageDto {
    fn from(page: &AuditPage) -> Self {
        Self {
            entries: page.entries.iter().map(AuditEntryDto::from).collect(),
            next: page.next.map(AuditCursor::seq),
        }
    }
}

impl TryFrom<AuditPageDto> for AuditPage {
    type Error = String;

    fn try_from(dto: AuditPageDto) -> Result<Self, String> {
        Ok(Self {
            entries: dto
                .entries
                .into_iter()
                .map(AuditEntry::try_from)
                .collect::<Result<_, _>>()?,
            next: dto.next.map(AuditCursor::from_seq),
        })
    }
}

// ---- Tokens ----

/// `POST /v1/tokens`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenIssueRequest {
    pub label: String,
}

/// What a token is, with no secret and no digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenDto {
    pub id: String,
    pub label: String,
    pub created_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_ms: Option<u64>,
}

impl From<&TokenRecord> for TokenDto {
    fn from(record: &TokenRecord) -> Self {
        Self {
            id: record.id.as_str().to_owned(),
            label: record.label.as_str().to_owned(),
            created_ms: record.created.as_millis(),
            revoked_ms: record.revoked.map(WallTime::as_millis),
        }
    }
}

impl TryFrom<TokenDto> for TokenRecord {
    type Error = String;

    fn try_from(dto: TokenDto) -> Result<Self, String> {
        Ok(Self {
            id: TokenId::new(dto.id).map_err(str::to_owned)?,
            label: AgentLabel::new(dto.label).map_err(str::to_owned)?,
            created: WallTime(dto.created_ms),
            revoked: dto.revoked_ms.map(WallTime),
        })
    }
}

/// `GET /v1/tokens`'s answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenListDto {
    pub tokens: Vec<TokenDto>,
}

/// The only response that holds a token's secret, once, when it is issued.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuedTokenDto {
    #[serde(flatten)]
    pub token: TokenDto,
    pub secret: String,
}

impl From<&IssuedToken> for IssuedTokenDto {
    fn from(issued: &IssuedToken) -> Self {
        Self {
            token: (&issued.record).into(),
            secret: issued.secret.expose().to_owned(),
        }
    }
}

impl TryFrom<IssuedTokenDto> for IssuedToken {
    type Error = String;

    fn try_from(dto: IssuedTokenDto) -> Result<Self, String> {
        Ok(Self {
            record: dto.token.try_into()?,
            secret: TokenSecret::new(dto.secret),
        })
    }
}
