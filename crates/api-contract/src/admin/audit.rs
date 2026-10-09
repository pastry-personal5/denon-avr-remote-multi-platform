//! The audit log: principals, decisions, events tagged by kind, records, entries,
//! and pages of entries.

use crate::operations::DispatchDto;
use crate::receivers::parse_receiver_id;
use denon_avr_application::audit::{core_field_name, parse_core_field};
use denon_avr_application::{
    AgentLabel, AuditCursor, AuditDecision, AuditEntry, AuditEvent, AuditPage, AuditRecord,
    EndpointKind, PolicyDigest, Principal, RefusalReason, TokenId,
};
use denon_avr_domain::{OperationId, WallTime};
use serde::{Deserialize, Serialize};

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
