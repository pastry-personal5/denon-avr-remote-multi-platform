//! Intents and operations on the wire.

use crate::receivers::parse_receiver_id;
use crate::values::{MuteDto, SoundModeDto, SystemPowerDto, VolumeDto, ZonePowerDto};
use denon_avr_application::{OperationSnapshot, OperationStatus};
use denon_avr_domain::{DispatchCertainty, OperationId, ReceiverIntent, SourceId};
use serde::{Deserialize, Serialize};

/// What a caller asks the receiver to do. One tagged variant for each
/// [`ReceiverIntent`]; a field it does not have is refused by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IntentDto {
    SystemPower { value: SystemPowerDto },
    MainZonePower { value: ZonePowerDto },
    Zone2Power { value: ZonePowerDto },
    Source { id: String },
    Volume { level: VolumeDto },
    Mute { value: MuteDto },
    SoundMode { mode: SoundModeDto },
}

impl From<&ReceiverIntent> for IntentDto {
    /// An exhaustive match, so a new intent does not compile until it has a wire
    /// form.
    fn from(intent: &ReceiverIntent) -> Self {
        match intent {
            ReceiverIntent::SystemPower(value) => Self::SystemPower {
                value: (*value).into(),
            },
            ReceiverIntent::MainZonePower(value) => Self::MainZonePower {
                value: (*value).into(),
            },
            ReceiverIntent::Zone2Power(value) => Self::Zone2Power {
                value: (*value).into(),
            },
            ReceiverIntent::Source(id) => Self::Source {
                id: id.as_str().to_owned(),
            },
            ReceiverIntent::Volume(level) => Self::Volume {
                level: (*level).into(),
            },
            ReceiverIntent::Mute(value) => Self::Mute {
                value: (*value).into(),
            },
            ReceiverIntent::SoundMode(mode) => Self::SoundMode { mode: mode.into() },
        }
    }
}

impl TryFrom<IntentDto> for ReceiverIntent {
    type Error = String;

    fn try_from(intent: IntentDto) -> Result<Self, String> {
        Ok(match intent {
            IntentDto::SystemPower { value } => Self::SystemPower(value.into()),
            IntentDto::MainZonePower { value } => Self::MainZonePower(value.into()),
            IntentDto::Zone2Power { value } => Self::Zone2Power(value.into()),
            IntentDto::Source { id } => {
                Self::Source(SourceId::new(id).map_err(|why| why.to_owned())?)
            }
            IntentDto::Volume { level } => {
                Self::Volume(level.try_into().map_err(|why: &str| why.to_owned())?)
            }
            IntentDto::Mute { value } => Self::Mute(value.into()),
            IntentDto::SoundMode { mode } => Self::SoundMode(mode.into()),
        })
    }
}

/// Where an operation is, by its stable name. `superseded` carries the id of the
/// operation that replaced it in [`OperationDto::superseded_by`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusDto {
    Submitted,
    Allowed,
    AwaitingApproval,
    Approved,
    InSession,
    Denied,
    ApprovalUnavailable,
    ApprovalRejected,
    Expired,
    Cancelled,
    Completed,
    AlreadyInState,
    Rejected,
    Superseded,
    Indeterminate,
}

/// The session's certainty that a write was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchDto {
    NotDispatched,
    PossiblyDispatched,
    CompleteWrite,
    Unknown,
}

impl From<&DispatchCertainty> for DispatchDto {
    fn from(value: &DispatchCertainty) -> Self {
        match value {
            DispatchCertainty::NotDispatched => Self::NotDispatched,
            DispatchCertainty::PossiblyDispatched => Self::PossiblyDispatched,
            DispatchCertainty::CompleteWrite => Self::CompleteWrite,
            DispatchCertainty::Unknown => Self::Unknown,
        }
    }
}

impl From<DispatchDto> for DispatchCertainty {
    fn from(value: DispatchDto) -> Self {
        match value {
            DispatchDto::NotDispatched => Self::NotDispatched,
            DispatchDto::PossiblyDispatched => Self::PossiblyDispatched,
            DispatchDto::CompleteWrite => Self::CompleteWrite,
            DispatchDto::Unknown => Self::Unknown,
        }
    }
}

/// A point-in-time view of one operation: the `status`, `dispatch`, and
/// `confirmed` terms of the operation lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationDto {
    pub id: u64,
    pub receiver: String,
    pub intent: IntentDto,
    pub status: StatusDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<u64>,
    pub dispatch: DispatchDto,
    pub confirmed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<String>,
}

impl From<&OperationSnapshot> for OperationDto {
    fn from(snapshot: &OperationSnapshot) -> Self {
        let (status, superseded_by) = match &snapshot.status {
            OperationStatus::Submitted => (StatusDto::Submitted, None),
            OperationStatus::Allowed => (StatusDto::Allowed, None),
            OperationStatus::AwaitingApproval => (StatusDto::AwaitingApproval, None),
            OperationStatus::Approved => (StatusDto::Approved, None),
            OperationStatus::InSession => (StatusDto::InSession, None),
            OperationStatus::Denied => (StatusDto::Denied, None),
            OperationStatus::ApprovalUnavailable => (StatusDto::ApprovalUnavailable, None),
            OperationStatus::ApprovalRejected => (StatusDto::ApprovalRejected, None),
            OperationStatus::Expired => (StatusDto::Expired, None),
            OperationStatus::Cancelled => (StatusDto::Cancelled, None),
            OperationStatus::Completed => (StatusDto::Completed, None),
            OperationStatus::AlreadyInState => (StatusDto::AlreadyInState, None),
            OperationStatus::Rejected => (StatusDto::Rejected, None),
            OperationStatus::Superseded { by } => (StatusDto::Superseded, Some(by.0)),
            OperationStatus::Indeterminate => (StatusDto::Indeterminate, None),
        };
        Self {
            id: snapshot.id.0,
            receiver: snapshot.receiver.as_str().to_owned(),
            intent: (&snapshot.intent).into(),
            status,
            superseded_by,
            dispatch: (&snapshot.dispatch).into(),
            confirmed: snapshot.confirmed,
            reason: snapshot.reason.clone(),
            observation: snapshot.observation.clone(),
        }
    }
}

impl TryFrom<OperationDto> for OperationSnapshot {
    type Error = String;

    fn try_from(dto: OperationDto) -> Result<Self, String> {
        let status = match dto.status {
            StatusDto::Submitted => OperationStatus::Submitted,
            StatusDto::Allowed => OperationStatus::Allowed,
            StatusDto::AwaitingApproval => OperationStatus::AwaitingApproval,
            StatusDto::Approved => OperationStatus::Approved,
            StatusDto::InSession => OperationStatus::InSession,
            StatusDto::Denied => OperationStatus::Denied,
            StatusDto::ApprovalUnavailable => OperationStatus::ApprovalUnavailable,
            StatusDto::ApprovalRejected => OperationStatus::ApprovalRejected,
            StatusDto::Expired => OperationStatus::Expired,
            StatusDto::Cancelled => OperationStatus::Cancelled,
            StatusDto::Completed => OperationStatus::Completed,
            StatusDto::AlreadyInState => OperationStatus::AlreadyInState,
            StatusDto::Rejected => OperationStatus::Rejected,
            StatusDto::Superseded => OperationStatus::Superseded {
                by: OperationId(
                    dto.superseded_by
                        .ok_or("a superseded operation names the one that replaced it")?,
                ),
            },
            StatusDto::Indeterminate => OperationStatus::Indeterminate,
        };
        Ok(Self {
            id: OperationId(dto.id),
            receiver: parse_receiver_id(&dto.receiver)?,
            intent: dto.intent.try_into()?,
            status,
            dispatch: dto.dispatch.into(),
            confirmed: dto.confirmed,
            reason: dto.reason,
            observation: dto.observation,
        })
    }
}
