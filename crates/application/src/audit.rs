//! What the audit log records and how it is read back.
//!
//! These are the values the control service writes about each decision, and the
//! values the Operator reads. The log itself, its durability, and its files are
//! ports and adapters that join this module with the components that provide
//! them. Records hold text the service built from typed values, never raw agent
//! text, and reasons arrive as the sentences the Operator would read.

use crate::control::Principal;
use crate::policy_source::PolicyDigest;
use denon_avr_domain::{CoreField, DispatchCertainty, OperationId, ReceiverId, WallTime};

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
}
