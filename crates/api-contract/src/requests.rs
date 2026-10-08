//! What a caller sends to submit or test an operation, and what a dry run answers.
//!
//! A request is strict: a field the type does not have is refused by name, so a
//! mistyped `dry_run` can never ride along on a submission and become a write. An
//! agent's dry-run request has no `as_agent`, and its answer has no rule ids.

use crate::operations::IntentDto;
use crate::state::NoText;
use denon_avr_application::{
    AgentLabel, DryRun, DryRunDecision, IdempotencyKey, OperationSubmission, PolicyDigest,
};
use denon_avr_domain::ReceiverIntent;
use serde::{Deserialize, Serialize};

/// `POST /v1/receivers/{id}/operations`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitRequest {
    pub intent: IntentDto,
    /// A key that makes a retry return the operation it began instead of making
    /// another.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

impl From<&OperationSubmission> for SubmitRequest {
    fn from(submission: &OperationSubmission) -> Self {
        Self {
            intent: (&submission.intent).into(),
            idempotency_key: submission
                .idempotency_key
                .as_ref()
                .map(|key| key.as_str().to_owned()),
        }
    }
}

impl SubmitRequest {
    pub fn into_submission(self) -> Result<OperationSubmission, String> {
        let intent = ReceiverIntent::try_from(self.intent)?;
        let mut submission = OperationSubmission::new(intent);
        if let Some(key) = self.idempotency_key {
            submission =
                submission.with_idempotency_key(IdempotencyKey::new(key).map_err(str::to_owned)?);
        }
        Ok(submission)
    }
}

/// `POST /v1/receivers/{id}/operations/dry-run` as an agent sends it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DryRunRequest {
    pub intent: IntentDto,
}

/// The same, as the Operator sends it: it may name the agent whose decision it
/// wants to see.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorDryRunRequest {
    pub intent: IntentDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_agent: Option<String>,
}

impl OperatorDryRunRequest {
    pub fn agent(&self) -> Result<Option<AgentLabel>, String> {
        self.as_agent
            .as_deref()
            .map(|label| AgentLabel::new(label).map_err(str::to_owned))
            .transpose()
    }
}

/// What the policy would decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionDto {
    Allow,
    RequireApproval,
    Deny,
    /// No decision can be made, so a real request would be refused.
    Unavailable,
}

/// The ids of the rules that fired, which only the Operator is told.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleIds {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<String>,
}

/// What a dry-run view adds to the decision.
pub trait Rules: Default + Clone {
    fn of(rules: &[String]) -> Self;
    fn rules(&self) -> Vec<String> {
        Vec::new()
    }
}

impl Rules for NoText {
    fn of(_: &[String]) -> Self {
        Self {}
    }
}

impl Rules for RuleIds {
    fn of(rules: &[String]) -> Self {
        Self {
            rules: rules.to_vec(),
        }
    }

    fn rules(&self) -> Vec<String> {
        self.rules.clone()
    }
}

/// A dry run's answer. It creates no operation and writes no record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(bound(serialize = "R: Serialize", deserialize = "R: Deserialize<'de>"))]
pub struct DryRunView<R> {
    pub decision: DecisionDto,
    /// Why, in sentences, for `require_approval` and `deny`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
    /// Why no decision can be made, for `unavailable`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The policy the decision rests on, as 64 hexadecimal digits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_digest: Option<String>,
    #[serde(flatten)]
    pub rules: R,
}

/// What an agent is told.
pub type AgentDryRun = DryRunView<NoText>;

/// What the Operator is told.
pub type OperatorDryRun = DryRunView<RuleIds>;

impl<R: Rules> From<&DryRun> for DryRunView<R> {
    fn from(dry_run: &DryRun) -> Self {
        let (decision, reasons, reason, rules) = match &dry_run.decision {
            DryRunDecision::Allow => (DecisionDto::Allow, Vec::new(), None, Vec::new()),
            DryRunDecision::RequireApproval { reasons, rules } => (
                DecisionDto::RequireApproval,
                reasons.clone(),
                None,
                rules.clone(),
            ),
            DryRunDecision::Deny { reasons, rules } => {
                (DecisionDto::Deny, reasons.clone(), None, rules.clone())
            }
            DryRunDecision::Unavailable { reason } => (
                DecisionDto::Unavailable,
                Vec::new(),
                Some(reason.clone()),
                Vec::new(),
            ),
        };
        Self {
            decision,
            reasons,
            reason,
            policy_digest: dry_run.policy.map(|digest| digest.to_string()),
            rules: R::of(&rules),
        }
    }
}

impl<R: Rules> DryRunView<R> {
    /// The port's value, with no rule ids when the view had none to carry.
    pub fn into_dry_run(self) -> Result<DryRun, String> {
        let rules = self.rules.rules();
        let decision = match self.decision {
            DecisionDto::Allow => DryRunDecision::Allow,
            DecisionDto::RequireApproval => DryRunDecision::RequireApproval {
                reasons: self.reasons,
                rules,
            },
            DecisionDto::Deny => DryRunDecision::Deny {
                reasons: self.reasons,
                rules,
            },
            DecisionDto::Unavailable => DryRunDecision::Unavailable {
                reason: self.reason.unwrap_or_default(),
            },
        };
        let policy = match self.policy_digest {
            Some(hex) => Some(
                PolicyDigest::from_hex(&hex)
                    .ok_or("the policy digest is not 64 hexadecimal digits")?,
            ),
            None => None,
        };
        Ok(DryRun { decision, policy })
    }
}
