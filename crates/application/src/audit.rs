//! What the audit log records and how it is read back.
//!
//! These are the values the control service writes about each decision, and the
//! values the Operator reads. The log itself, its durability, and its files are
//! ports and adapters that join this module with the components that provide
//! them. Records hold text the service built from typed values, never raw agent
//! text, and reasons arrive as the sentences the Operator would read.

use crate::control::Principal;
use crate::policy_source::PolicyDigest;
use crate::ports::BoxFuture;
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
    pub principal: Principal,
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
