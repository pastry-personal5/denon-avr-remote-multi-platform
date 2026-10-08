//! What the audit log records and how it is read back.
//!
//! These are the values the control service writes about each decision, and the
//! values the Operator reads. The log itself, its durability, and its files are
//! ports and adapters that join this module with the components that provide
//! them. Records hold text the service built from typed values, never raw agent
//! text, and reasons arrive as the sentences the Operator would read.

use crate::control::{AgentLabel, Principal};
use crate::policy_source::PolicyDigest;
use crate::ports::BoxFuture;
use crate::tokens::TokenId;
use denon_avr_domain::{
    CoreField, DispatchCertainty, MasterVolume, MuteState, OperationId, ReceiverId, ReceiverIntent,
    SoundModeIntent, SystemPower, WallTime, ZonePower,
};
use std::sync::Arc;

/// The record layout this code writes.
pub const AUDIT_SCHEMA: u32 = 1;

/// The most entries one page of [`AuditPage`] holds.
pub const MAX_PAGE: usize = 500;

/// The longest intent text a record carries, in characters.
pub const MAX_INTENT_TEXT: usize = 128;

/// One line of the audit log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    pub schema: u32,
    /// When the service that wrote this started. Operation ids restart at 1 on
    /// every run, so an operation is named by the run and its id together.
    pub run: WallTime,
    pub at: WallTime,
    pub operation: Option<OperationId>,
    /// Who acted. `None` when no valid credential was presented, as with a caller
    /// refused at an endpoint.
    pub principal: Option<Principal>,
    /// `None` for events about the service, such as loading the policy.
    pub receiver: Option<ReceiverId>,
    pub event: AuditEvent,
}

/// What a decision came to, as the log names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditDecision {
    Allow,
    RequireApproval,
    Deny,
}

/// What happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditEvent {
    /// The gate decided. `baseline` is the state the decision read.
    Decided {
        intent: String,
        decision: AuditDecision,
        reasons: Vec<String>,
        rules: Vec<String>,
        baseline: Vec<CoreField>,
        policy: Option<PolicyDigest>,
    },
    /// The gate is about to call the session. Written and synced first, so a
    /// crash leaves a record that the write may have happened. `before` and
    /// `target` are half steps and are set for volume changes only.
    Dispatching {
        intent: String,
        before: Option<i16>,
        target: Option<i16>,
        precondition_fields: Vec<CoreField>,
    },
    /// The operation ended. `status` is the stable name clients see.
    Finished {
        status: String,
        dispatch: DispatchCertainty,
        confirmed: bool,
        reason: Option<String>,
    },
    PolicyLoaded {
        digest: PolicyDigest,
    },
    PolicyLoadFailed {
        error: String,
    },
    /// A caller was refused at an endpoint. It is written at most once a minute
    /// for each caller and reason, with the count of those that were not.
    AccessRefused {
        endpoint: EndpointKind,
        reason: RefusalReason,
        /// The uid the operating system reported for the peer, when it did.
        peer_uid: Option<u32>,
        /// The route pattern an agent probed, never the path it sent.
        resource: Option<String>,
        /// How many like it were refused since the last record and not written.
        suppressed: u32,
    },
    /// An Agent token was issued. The record holds no secret.
    TokenIssued {
        id: TokenId,
        label: AgentLabel,
    },
    TokenRevoked {
        id: TokenId,
        label: AgentLabel,
    },
}

/// Which of the Control API's two endpoints a request reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EndpointKind {
    Operator,
    Agent,
}

impl EndpointKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Operator => "operator",
            Self::Agent => "agent",
        }
    }

    /// The kind a record's name stands for, or `None` for a name this code does
    /// not know.
    pub fn parse(name: &str) -> Option<Self> {
        [Self::Operator, Self::Agent]
            .into_iter()
            .find(|kind| kind.as_str() == name)
    }
}

/// Why a caller was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefusalReason {
    /// The peer's uid is not one the endpoint admits. The connection was closed
    /// before it was read.
    PeerNotAdmitted,
    NoCredential,
    UnknownCredential,
    /// The Operator's token, presented on the Agent endpoint.
    OperatorTokenOnAgentEndpoint,
    /// An Agent token, presented on the Operator endpoint.
    AgentTokenOnOperatorEndpoint,
    /// A valid credential asked for a resource the endpoint does not serve.
    ResourceNotServed,
}

impl RefusalReason {
    /// Every reason, for tests and for readers that must name them all.
    pub const ALL: [Self; 6] = [
        Self::PeerNotAdmitted,
        Self::NoCredential,
        Self::UnknownCredential,
        Self::OperatorTokenOnAgentEndpoint,
        Self::AgentTokenOnOperatorEndpoint,
        Self::ResourceNotServed,
    ];

    /// The reason a record's name stands for, or `None` for a name this code does
    /// not know.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| reason.as_str() == name)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::PeerNotAdmitted => "peer_not_admitted",
            Self::NoCredential => "no_credential",
            Self::UnknownCredential => "unknown_credential",
            Self::OperatorTokenOnAgentEndpoint => "operator_token_on_agent_endpoint",
            Self::AgentTokenOnOperatorEndpoint => "agent_token_on_operator_endpoint",
            Self::ResourceNotServed => "resource_not_served",
        }
    }
}

/// A refusal at an endpoint, as the server reports it to the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessRefusal {
    pub endpoint: EndpointKind,
    pub reason: RefusalReason,
    /// The principal, when the credential was valid.
    pub principal: Option<Principal>,
    pub peer_uid: Option<u32>,
    /// The route pattern, when a route was matched.
    pub resource: Option<String>,
}

/// A record and its place in the log. `seq` rises with every append and
/// continues after a restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    pub seq: u64,
    pub record: AuditRecord,
}

/// Where a page ended, to be handed back to continue from there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditCursor(u64);

impl AuditCursor {
    pub fn from_seq(seq: u64) -> Self {
        Self(seq)
    }

    pub fn seq(self) -> u64 {
        self.0
    }
}

/// A request for a page of the log, newest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditQuery {
    limit: usize,
    cursor: Option<AuditCursor>,
}

impl AuditQuery {
    /// A first page of up to `limit` entries, at least one and at most
    /// [`MAX_PAGE`].
    pub fn new(limit: usize) -> Self {
        Self {
            limit: limit.clamp(1, MAX_PAGE),
            cursor: None,
        }
    }

    /// The page after `cursor`, the `next` of the page before it.
    pub fn after(mut self, cursor: AuditCursor) -> Self {
        self.cursor = Some(cursor);
        self
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    pub fn cursor(&self) -> Option<AuditCursor> {
        self.cursor
    }
}

/// One page, newest entry first. `next` is set when older entries remain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditPage {
    pub entries: Vec<AuditEntry>,
    pub next: Option<AuditCursor>,
}

/// How much of the disk's cooperation an append waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Durability {
    /// Written to the file. A crash of the machine may lose it.
    Flushed,
    /// Written and synced to the disk before the append returns. The record
    /// that must exist before a write to the receiver is this kind.
    Synced,
}

/// Why the log could not be written or read. The text is for the Operator's
/// log and never reaches an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditError(String);

impl AuditError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for AuditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "audit log: {}", self.0)
    }
}

impl std::error::Error for AuditError {}

/// The append-only record of what the gate decided and did.
pub trait AuditLog: Send + Sync {
    /// Add a record, returning once it is as durable as asked.
    fn append(
        &self,
        record: AuditRecord,
        durability: Durability,
    ) -> BoxFuture<'_, Result<(), AuditError>>;

    /// Every readable record written at or after `from`, oldest first. A log
    /// that does not exist yet has none.
    fn since(&self, from: WallTime) -> BoxFuture<'_, Result<Vec<AuditRecord>, AuditError>>;

    /// A page of the log, newest first.
    fn query(&self, query: AuditQuery) -> BoxFuture<'_, Result<AuditPage, AuditError>>;
}

pub type SharedAuditLog = Arc<dyn AuditLog>;

/// The longest reason, error, or rule text a record carries, in characters.
pub const MAX_REASON_TEXT: usize = 512;

/// `text` cut to at most `max` characters, never inside one.
pub fn bounded(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// The request in the words the policy uses: the intent's name, then its value.
/// Agent-chosen text such as a source id is bounded and loses control
/// characters, so a record's intent is one short line whatever was asked.
pub fn intent_text(intent: &ReceiverIntent) -> String {
    let (name, value) = match intent {
        ReceiverIntent::SystemPower(SystemPower::On) => ("system_power", "on".to_string()),
        ReceiverIntent::SystemPower(SystemPower::Standby) => {
            ("system_power", "standby".to_string())
        }
        ReceiverIntent::MainZonePower(power) => ("main_zone_power", zone_power(*power)),
        ReceiverIntent::Zone2Power(power) => ("zone2_power", zone_power(*power)),
        ReceiverIntent::Source(source) => ("source", source.as_str().to_string()),
        ReceiverIntent::Volume(MasterVolume::Minimum) => ("volume", "minimum".to_string()),
        ReceiverIntent::Volume(MasterVolume::DbHalfSteps(steps)) => {
            ("volume", format!("{:.1} dB", f64::from(*steps) / 2.0))
        }
        ReceiverIntent::Mute(MuteState::On) => ("mute", "on".to_string()),
        ReceiverIntent::Mute(MuteState::Off) => ("mute", "off".to_string()),
        ReceiverIntent::SoundMode(mode) => ("sound_mode", sound_mode(mode)),
    };
    let text: String = format!("{name} {value}")
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    bounded(&text, MAX_INTENT_TEXT)
}

fn zone_power(power: ZonePower) -> String {
    match power {
        ZonePower::On => "on",
        ZonePower::Off => "off",
    }
    .to_string()
}

fn sound_mode(mode: &SoundModeIntent) -> String {
    match mode {
        SoundModeIntent::Auto => "auto".to_string(),
        SoundModeIntent::Direct => "direct".to_string(),
        SoundModeIntent::PureDirect => "pure_direct".to_string(),
        SoundModeIntent::Stereo => "stereo".to_string(),
        SoundModeIntent::RecallMovie => "recall_movie".to_string(),
        SoundModeIntent::RecallMusic => "recall_music".to_string(),
        SoundModeIntent::RecallGame => "recall_game".to_string(),
        SoundModeIntent::Select(name) => name.clone(),
    }
}

const FIELDS: [CoreField; 7] = [
    CoreField::SystemPower,
    CoreField::MainZonePower,
    CoreField::Zone2Power,
    CoreField::Source,
    CoreField::Volume,
    CoreField::Mute,
    CoreField::SoundMode,
];

/// The name a record gives a state field.
pub fn core_field_name(field: CoreField) -> &'static str {
    match field {
        CoreField::SystemPower => "system_power",
        CoreField::MainZonePower => "main_zone_power",
        CoreField::Zone2Power => "zone2_power",
        CoreField::Source => "source",
        CoreField::Volume => "volume",
        CoreField::Mute => "mute",
        CoreField::SoundMode => "sound_mode",
    }
}

/// The field a record's name stands for, or `None` for a name this code does
/// not know.
pub fn parse_core_field(name: &str) -> Option<CoreField> {
    FIELDS
        .into_iter()
        .find(|field| core_field_name(*field) == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_is_at_least_one_record_and_never_more_than_the_cap() {
        assert_eq!(AuditQuery::new(0).limit(), 1);
        assert_eq!(AuditQuery::new(10).limit(), 10);
        assert_eq!(AuditQuery::new(MAX_PAGE).limit(), MAX_PAGE);
        assert_eq!(AuditQuery::new(MAX_PAGE + 1).limit(), MAX_PAGE);
        assert_eq!(AuditQuery::new(usize::MAX).limit(), MAX_PAGE);
    }

    #[test]
    fn a_query_continues_after_a_cursor() {
        let first = AuditQuery::new(5);
        assert_eq!(first.cursor(), None);
        let next = first.after(AuditCursor::from_seq(41));
        assert_eq!(next.cursor().map(AuditCursor::seq), Some(41));
        assert_eq!(next.limit(), 5);
    }

    #[test]
    fn intents_are_named_in_the_words_the_policy_uses() {
        use denon_avr_domain::{
            MasterVolume, MuteState, SoundModeIntent, SourceId, SystemPower, ZonePower,
        };
        let text = |intent: ReceiverIntent| intent_text(&intent);
        assert_eq!(
            text(ReceiverIntent::SystemPower(SystemPower::Standby)),
            "system_power standby"
        );
        assert_eq!(
            text(ReceiverIntent::MainZonePower(ZonePower::On)),
            "main_zone_power on"
        );
        assert_eq!(
            text(ReceiverIntent::Zone2Power(ZonePower::Off)),
            "zone2_power off"
        );
        assert_eq!(
            text(ReceiverIntent::Source(SourceId::new("TV").unwrap())),
            "source TV"
        );
        assert_eq!(
            text(ReceiverIntent::Volume(
                MasterVolume::db_half_steps(-71).unwrap()
            )),
            "volume -35.5 dB"
        );
        assert_eq!(
            text(ReceiverIntent::Volume(MasterVolume::Minimum)),
            "volume minimum"
        );
        assert_eq!(text(ReceiverIntent::Mute(MuteState::On)), "mute on");
        assert_eq!(
            text(ReceiverIntent::SoundMode(SoundModeIntent::PureDirect)),
            "sound_mode pure_direct"
        );
        assert_eq!(
            text(ReceiverIntent::SoundMode(SoundModeIntent::Select(
                "DOLBY ATMOS".into()
            ))),
            "sound_mode DOLBY ATMOS"
        );
    }

    #[test]
    fn agent_text_in_an_intent_is_bounded_and_has_no_control_characters() {
        use denon_avr_domain::{SoundModeIntent, SourceId};
        let hostile = format!("a\nb\"c\u{7}{}", "x".repeat(10_000));
        for intent in [
            ReceiverIntent::SoundMode(SoundModeIntent::Select(hostile.clone())),
            ReceiverIntent::Source(SourceId::new(hostile.replace(['\n', '\u{7}'], "")).unwrap()),
        ] {
            let text = intent_text(&intent);
            assert!(text.chars().count() <= MAX_INTENT_TEXT, "{}", text.len());
            assert!(!text.chars().any(char::is_control), "{text:?}");
        }
        // The cut never splits a character.
        let wide = "é".repeat(500);
        let text = bounded(&wide, 128);
        assert_eq!(text.chars().count(), 128);
        assert_eq!(bounded("short", 128), "short");
    }

    #[test]
    fn every_core_field_has_a_stable_name_that_reads_back() {
        let names: Vec<&str> = FIELDS.iter().map(|f| core_field_name(*f)).collect();
        assert_eq!(
            names,
            [
                "system_power",
                "main_zone_power",
                "zone2_power",
                "source",
                "volume",
                "mute",
                "sound_mode"
            ]
        );
        for field in FIELDS {
            assert_eq!(parse_core_field(core_field_name(field)), Some(field));
            // An exhaustive match here fails to compile when a field is added
            // without a name above, and the length check below catches a field
            // added to the match but not to the list.
            match field {
                CoreField::SystemPower
                | CoreField::MainZonePower
                | CoreField::Zone2Power
                | CoreField::Source
                | CoreField::Volume
                | CoreField::Mute
                | CoreField::SoundMode => {}
            }
        }
        assert_eq!(FIELDS.len(), 7);
        assert_eq!(parse_core_field("surround_back"), None);
        assert_eq!(parse_core_field("Volume"), None);
    }

    #[test]
    fn refusal_reasons_and_endpoints_have_unique_snake_case_names_that_read_back() {
        let names: Vec<&str> = RefusalReason::ALL.iter().map(|r| r.as_str()).collect();
        let unique: std::collections::BTreeSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len());
        for reason in RefusalReason::ALL {
            assert!(reason
                .as_str()
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_'));
            assert_eq!(RefusalReason::parse(reason.as_str()), Some(reason));
            // An exhaustive match fails to compile when a reason is added without
            // being listed in `ALL`, which the length check then confirms.
            match reason {
                RefusalReason::PeerNotAdmitted
                | RefusalReason::NoCredential
                | RefusalReason::UnknownCredential
                | RefusalReason::OperatorTokenOnAgentEndpoint
                | RefusalReason::AgentTokenOnOperatorEndpoint
                | RefusalReason::ResourceNotServed => {}
            }
        }
        assert_eq!(RefusalReason::ALL.len(), 6);
        assert_eq!(RefusalReason::parse("from_the_future"), None);
        for endpoint in [EndpointKind::Operator, EndpointKind::Agent] {
            assert_eq!(EndpointKind::parse(endpoint.as_str()), Some(endpoint));
        }
        assert_eq!(EndpointKind::parse("Agent"), None);
    }

    #[test]
    fn the_audit_log_is_a_usable_object() {
        struct Nothing;
        impl AuditLog for Nothing {
            fn append(
                &self,
                _: AuditRecord,
                _: Durability,
            ) -> BoxFuture<'_, Result<(), AuditError>> {
                Box::pin(async { Err(AuditError::new("no log")) })
            }
            fn since(&self, _: WallTime) -> BoxFuture<'_, Result<Vec<AuditRecord>, AuditError>> {
                Box::pin(async { Ok(Vec::new()) })
            }
            fn query(&self, _: AuditQuery) -> BoxFuture<'_, Result<AuditPage, AuditError>> {
                Box::pin(async { Err(AuditError::new("no log")) })
            }
        }
        let log: std::sync::Arc<dyn AuditLog> = std::sync::Arc::new(Nothing);
        let error = AuditError::new("no log");
        assert_eq!(error.to_string(), "audit log: no log");
        drop(log);
    }
}
