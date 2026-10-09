//! Callers refused at an endpoint: who a refusal counts against, and the book
//! that writes them to the audit log at a bounded rate.

use super::audit::OPERATOR_AUDIT_TIMEOUT;
use super::AgentState;
use crate::audit::{AccessRefusal, AuditEvent, Durability, EndpointKind, RefusalReason};
use crate::control::{AgentLabel, Principal};
use crate::service::locked;
use std::collections::{HashMap, VecDeque};
use std::time::Duration;
use tokio::time::Instant;
use tracing::warn;

/// How often a refused caller is written to the log, per caller and reason.
const ACCESS_LOG_EVERY: Duration = Duration::from_secs(60);

/// How many refused callers are tracked at once.
const ACCESS_KEYS: usize = 256;

/// How many refusals are written a minute over all callers.
const ACCESS_RECORDS_PER_WINDOW: usize = 30;

/// Who a refusal is counted against: the agent when the credential was valid,
/// otherwise the peer's uid.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Subject {
    Label(AgentLabel),
    Uid(u32),
    Nobody,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct AccessKey {
    endpoint: EndpointKind,
    reason: RefusalReason,
    who: Subject,
}

struct AccessEntry {
    last: Instant,
    suppressed: u32,
}

#[derive(Default)]
pub(super) struct AccessBook {
    keys: HashMap<AccessKey, AccessEntry>,
    /// When the records of the last minute were written.
    written: VecDeque<Instant>,
    /// Refusals not written because the whole log was at its rate and their
    /// caller was not tracked, to be counted in the next record.
    unwritten: u32,
}

impl AgentState {
    /// Record a caller refused at an endpoint, at a bounded rate.
    ///
    /// A caller that keeps asking would otherwise write a line per attempt and
    /// push real records out of the log, the budget's among them. So the first
    /// refusal of a caller and reason is written, later ones inside a minute are
    /// counted, and the next record after the minute carries the count. At most
    /// [`ACCESS_KEYS`] callers are tracked and [`ACCESS_RECORDS_PER_WINDOW`]
    /// records are written a minute over all of them, so a flood of different
    /// callers cannot do what one caller cannot. A failed append is logged and
    /// does not change the audit health: a caller must not be able to mark the log
    /// failing, and so have every agent write refused, by being refused itself.
    pub(in crate::service) async fn record_access_refusal(&self, refusal: AccessRefusal) {
        let who = match (&refusal.principal, refusal.peer_uid) {
            (Some(Principal::Agent(label)), _) => Subject::Label(label.clone()),
            (_, Some(uid)) => Subject::Uid(uid),
            _ => Subject::Nobody,
        };
        let key = AccessKey {
            endpoint: refusal.endpoint,
            reason: refusal.reason,
            who,
        };
        let Some(suppressed) = self.admit_refusal(key, Instant::now()) else {
            return;
        };
        let resource = refusal.resource.map(|route| {
            let clean: String = route
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            crate::audit::bounded(&clean, crate::audit::MAX_INTENT_TEXT)
        });
        let record = self.record_as(
            None,
            refusal.principal,
            None,
            AuditEvent::AccessRefused {
                endpoint: refusal.endpoint,
                reason: refusal.reason,
                peer_uid: refusal.peer_uid,
                resource,
                suppressed,
            },
        );
        let outcome = tokio::time::timeout(
            OPERATOR_AUDIT_TIMEOUT,
            self.audit.append(record, Durability::Flushed),
        )
        .await;
        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(error)) => warn!(%error, "appending a refusal to the audit log"),
            Err(_) => warn!("appending a refusal to the audit log timed out"),
        }
    }

    /// Whether a refusal is written now, and how many like it were not.
    fn admit_refusal(&self, key: AccessKey, now: Instant) -> Option<u32> {
        let mut book = locked(&self.access);
        book.written
            .retain(|at| now.duration_since(*at) < ACCESS_LOG_EVERY);
        if let Some(entry) = book.keys.get_mut(&key) {
            if now.duration_since(entry.last) < ACCESS_LOG_EVERY {
                entry.suppressed = entry.suppressed.saturating_add(1);
                return None;
            }
        }
        if book.written.len() >= ACCESS_RECORDS_PER_WINDOW {
            // The log as a whole is at its rate. Count the refusal against its key,
            // or against the next record written when the key is not tracked.
            if let Some(entry) = book.keys.get_mut(&key) {
                entry.suppressed = entry.suppressed.saturating_add(1);
            } else {
                book.unwritten = book.unwritten.saturating_add(1);
            }
            return None;
        }
        if !book.keys.contains_key(&key) && book.keys.len() >= ACCESS_KEYS {
            let oldest = book
                .keys
                .iter()
                .min_by_key(|(_, entry)| entry.last)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                book.keys.remove(&oldest);
            }
        }
        let unwritten = std::mem::take(&mut book.unwritten);
        let entry = book.keys.entry(key).or_insert(AccessEntry {
            last: now,
            suppressed: 0,
        });
        let suppressed = std::mem::take(&mut entry.suppressed).saturating_add(unwritten);
        entry.last = now;
        book.written.push_back(now);
        Some(suppressed)
    }

    /// How many refused callers are being tracked.
    #[cfg(test)]
    pub(in crate::service) fn tracked_refusals(&self) -> usize {
        locked(&self.access).keys.len()
    }
}
