//! The volume changes the budget is measured over.
//!
//! The policy's cumulative budget asks how far a volume change rises above the
//! lowest level of the last few minutes, whoever made the changes. This ledger
//! is that history. It is a plain structure the service guards with a mutex and
//! never holds across an await, so reading it, evaluating, and reserving a
//! place in it happen as one step.
//!
//! An agent operation counts from the moment it is allowed, before the session
//! is called, and stays if anything may have been written. The Operator's
//! volume writes count too, but are never limited. After a restart the ledger is
//! rebuilt from the audit log, where a `Dispatching` record with no `Finished`
//! one is a write that may have happened and so counts.
//!
//! Entries are kept for 24 hours, the longest budget window the policy allows.

use crate::audit::{AuditEvent, AuditRecord};
use denon_avr_domain::{DispatchCertainty, OperationId, ReceiverId, WallTime};
use denon_avr_policy::{Level, RecentChange};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

/// How long an entry is kept: the longest budget window.
pub const RETENTION: Duration = Duration::from_secs(24 * 60 * 60);

/// Names one operation across restarts. Operation ids start again at 1 on every
/// run, so the id alone would name two operations; the run's start time tells
/// them apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OpKey {
    run: WallTime,
    operation: OperationId,
}

impl OpKey {
    pub fn new(run: WallTime, operation: OperationId) -> Self {
        Self { run, operation }
    }
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    key: OpKey,
    change: RecentChange,
}

/// Volume changes by receiver.
#[derive(Debug, Default)]
pub struct Ledger {
    entries: HashMap<ReceiverId, Vec<Entry>>,
}

impl Ledger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Count `change` against `receiver` from now on. Called under the same lock
    /// as the evaluation that allowed it.
    pub fn reserve(&mut self, receiver: &ReceiverId, key: OpKey, change: RecentChange) {
        self.entries
            .entry(receiver.clone())
            .or_default()
            .push(Entry { key, change });
    }

    /// The operation ended. One that may have written stays, because the volume
    /// it asked for may be the level now; one that certainly did not is removed.
    pub fn settle(&mut self, receiver: &ReceiverId, key: OpKey, dispatched: bool) {
        if dispatched {
            return;
        }
        if let Some(entries) = self.entries.get_mut(receiver) {
            entries.retain(|entry| entry.key != key);
        }
    }

    /// The changes to `receiver` inside `longest_window` before `now`, oldest
    /// first. A change dated after `now` is inside, so a clock that stepped back
    /// cannot shrink the budget. A rule with a shorter window narrows this
    /// itself.
    pub fn recent(
        &self,
        receiver: &ReceiverId,
        now: WallTime,
        longest_window: Duration,
    ) -> Vec<RecentChange> {
        let since = now.saturating_sub(longest_window);
        let mut changes: Vec<RecentChange> = self
            .entries
            .get(receiver)
            .into_iter()
            .flatten()
            .map(|entry| entry.change)
            .filter(|change| change.at > since)
            .collect();
        changes.sort_by_key(|change| change.at);
        changes
    }

    /// Drop what is more than [`RETENTION`] old. A change dated after `now` is
    /// kept.
    pub fn prune(&mut self, now: WallTime) {
        let cutoff = now.saturating_sub(RETENTION);
        for entries in self.entries.values_mut() {
            entries.retain(|entry| entry.change.at >= cutoff);
        }
        self.entries.retain(|_, entries| !entries.is_empty());
    }

    /// The ledger the audit records describe, as of `now`.
    ///
    /// Every volume `Dispatching` record in the last 24 hours counts, whoever
    /// wrote it, except those a `Finished` record of the same operation says
    /// did not dispatch. A `Dispatching` with no `Finished` is kept: the process
    /// ended after the write may have been made. A `Finished` with no
    /// `Dispatching` says nothing.
    pub fn rebuild(records: &[AuditRecord], now: WallTime) -> Self {
        let mut not_dispatched = HashSet::new();
        for record in records {
            if let (
                Some(operation),
                AuditEvent::Finished {
                    dispatch: DispatchCertainty::NotDispatched,
                    ..
                },
            ) = (record.operation, &record.event)
            {
                not_dispatched.insert(OpKey::new(record.run, operation));
            }
        }

        let mut ledger = Self::new();
        for record in records {
            let AuditEvent::Dispatching { before, target, .. } = &record.event else {
                continue;
            };
            let (Some(target), Some(receiver)) = (target, &record.receiver) else {
                continue;
            };
            let key = record
                .operation
                .map(|operation| OpKey::new(record.run, operation));
            if key.is_some_and(|key| not_dispatched.contains(&key)) {
                continue;
            }
            ledger.reserve(
                receiver,
                // An operation-less record cannot be settled, and none is.
                key.unwrap_or(OpKey::new(record.run, OperationId(0))),
                RecentChange {
                    at: record.at,
                    before: before.map(level_or_lowest),
                    target: level_or_lowest(*target),
                },
            );
        }
        ledger.prune(now);
        ledger
    }
}

/// A level from a record. One that is not on the scale cannot be read, and the
/// lowest level is the safe guess: it makes the floor low and so the budget
/// hard to stay inside.
fn level_or_lowest(half_steps: i16) -> Level {
    Level::from_half_steps(half_steps).unwrap_or(Level::MINIMUM)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{AuditEvent, AuditRecord, AUDIT_SCHEMA};
    use crate::control::{AgentLabel, Principal};
    use denon_avr_domain::{DispatchCertainty, OperationId, ReceiverId, WallTime};
    use denon_avr_policy::{Level, RecentChange};
    use std::time::Duration;

    const NOW: WallTime = WallTime(1_000_000_000);

    fn ago(by: Duration) -> WallTime {
        NOW.saturating_sub(by)
    }

    fn minutes(n: u64) -> Duration {
        Duration::from_secs(n * 60)
    }

    fn hours(n: u64) -> Duration {
        minutes(n * 60)
    }

    fn room() -> ReceiverId {
        ReceiverId::new("living-room").unwrap()
    }

    fn kitchen() -> ReceiverId {
        ReceiverId::new("kitchen").unwrap()
    }

    fn key(run: u64, id: u64) -> OpKey {
        OpKey::new(WallTime(run), OperationId(id))
    }

    fn level(half_steps: i16) -> Level {
        Level::from_half_steps(half_steps).unwrap()
    }

    fn change(at: WallTime, before: Option<i16>, target: i16) -> RecentChange {
        RecentChange {
            at,
            before: before.map(level),
            target: level(target),
        }
    }

    fn agent() -> Principal {
        Principal::Agent(AgentLabel::new("openclaw").unwrap())
    }

    fn record(
        run: u64,
        id: u64,
        at: WallTime,
        principal: Principal,
        event: AuditEvent,
    ) -> AuditRecord {
        AuditRecord {
            schema: AUDIT_SCHEMA,
            run: WallTime(run),
            at,
            operation: Some(OperationId(id)),
            principal,
            receiver: Some(room()),
            event,
        }
    }

    fn dispatching(
        run: u64,
        id: u64,
        at: WallTime,
        principal: Principal,
        before: Option<i16>,
        target: Option<i16>,
    ) -> AuditRecord {
        record(
            run,
            id,
            at,
            principal,
            AuditEvent::Dispatching {
                intent: "volume".into(),
                before,
                target,
                precondition_fields: vec![],
            },
        )
    }

    fn finished(run: u64, id: u64, at: WallTime, dispatch: DispatchCertainty) -> AuditRecord {
        record(
            run,
            id,
            at,
            agent(),
            AuditEvent::Finished {
                status: "completed".into(),
                dispatch,
                confirmed: false,
                reason: None,
            },
        )
    }

    fn window() -> Duration {
        minutes(10)
    }

    fn recent(ledger: &Ledger, receiver: &ReceiverId) -> Vec<RecentChange> {
        ledger.recent(receiver, NOW, window())
    }

    #[test]
    fn a_reservation_counts_before_it_settles() {
        let mut ledger = Ledger::new();
        let held = change(ago(minutes(2)), Some(-70), -64);
        ledger.reserve(&room(), key(1, 1), held);
        assert_eq!(recent(&ledger, &room()), vec![held]);
    }

    #[test]
    fn a_settled_dispatch_keeps_its_entry() {
        let mut ledger = Ledger::new();
        let held = change(ago(minutes(2)), Some(-70), -64);
        ledger.reserve(&room(), key(1, 1), held);
        ledger.settle(&room(), key(1, 1), true);
        assert_eq!(recent(&ledger, &room()), vec![held]);
    }

    #[test]
    fn a_settled_non_dispatch_removes_it() {
        let mut ledger = Ledger::new();
        ledger.reserve(&room(), key(1, 1), change(ago(minutes(2)), Some(-70), -64));
        ledger.settle(&room(), key(1, 1), false);
        assert_eq!(recent(&ledger, &room()), vec![]);
    }

    #[test]
    fn settling_what_is_not_there_changes_nothing() {
        let mut ledger = Ledger::new();
        let held = change(ago(minutes(2)), None, -64);
        ledger.reserve(&room(), key(1, 1), held);
        ledger.settle(&room(), key(1, 2), false);
        ledger.settle(&kitchen(), key(1, 1), false);
        assert_eq!(recent(&ledger, &room()), vec![held]);
    }

    #[test]
    fn two_runs_with_the_same_operation_id_do_not_collide() {
        let mut ledger = Ledger::new();
        let first = change(ago(minutes(4)), Some(-70), -64);
        let second = change(ago(minutes(3)), Some(-64), -58);
        ledger.reserve(&room(), key(10, 1), first);
        ledger.reserve(&room(), key(20, 1), second);
        ledger.settle(&room(), key(10, 1), false);
        assert_eq!(recent(&ledger, &room()), vec![second]);
    }

    #[test]
    fn a_receiver_sees_only_its_own_changes() {
        let mut ledger = Ledger::new();
        let theirs = change(ago(minutes(1)), None, -40);
        ledger.reserve(&kitchen(), key(1, 1), theirs);
        assert_eq!(recent(&ledger, &room()), vec![]);
        assert_eq!(recent(&ledger, &kitchen()), vec![theirs]);
    }

    #[test]
    fn recent_returns_what_is_inside_the_window_oldest_first() {
        let mut ledger = Ledger::new();
        let newer = change(ago(minutes(2)), None, -60);
        let older = change(ago(minutes(8)), None, -62);
        let outside = change(ago(minutes(11)), None, -64);
        let at_the_edge = change(ago(minutes(10)), None, -66);
        for (id, held) in [(1, newer), (2, outside), (3, older), (4, at_the_edge)] {
            ledger.reserve(&room(), key(1, id), held);
        }
        assert_eq!(recent(&ledger, &room()), vec![older, newer]);
        // A wider window reaches further back; none sees nothing past it.
        assert_eq!(ledger.recent(&room(), NOW, minutes(30)).len(), 4);
        assert_eq!(ledger.recent(&room(), NOW, Duration::ZERO), vec![]);
    }

    #[test]
    fn a_future_dated_entry_still_counts_and_is_never_pruned() {
        let mut ledger = Ledger::new();
        // The clock stepped back after this was written.
        let future = change(
            WallTime(NOW.as_millis() + hours(3).as_millis() as u64),
            None,
            -50,
        );
        ledger.reserve(&room(), key(1, 1), future);
        assert_eq!(recent(&ledger, &room()), vec![future]);
        ledger.prune(NOW);
        assert_eq!(recent(&ledger, &room()), vec![future]);
    }

    #[test]
    fn entries_older_than_24_hours_are_pruned() {
        let mut ledger = Ledger::new();
        let day = hours(24);
        let exactly = change(ago(day), None, -60);
        let older = change(WallTime(ago(day).as_millis() - 1), None, -62);
        let recent_one = change(ago(hours(23)), None, -64);
        for (id, held) in [(1, exactly), (2, older), (3, recent_one)] {
            ledger.reserve(&room(), key(1, id), held);
        }
        ledger.prune(NOW);
        let kept = ledger.recent(&room(), NOW, hours(48));
        assert_eq!(kept, vec![exactly, recent_one]);
    }

    #[test]
    fn rebuild_keeps_dispatching_without_finished() {
        // The process died between the write and its record.
        let records = [dispatching(
            5,
            1,
            ago(minutes(3)),
            agent(),
            Some(-70),
            Some(-64),
        )];
        let ledger = Ledger::rebuild(&records, NOW);
        assert_eq!(
            recent(&ledger, &room()),
            vec![change(ago(minutes(3)), Some(-70), -64)]
        );
    }

    #[test]
    fn rebuild_drops_a_finished_not_dispatched() {
        let records = [
            dispatching(5, 1, ago(minutes(3)), agent(), Some(-70), Some(-64)),
            finished(5, 1, ago(minutes(3)), DispatchCertainty::NotDispatched),
        ];
        assert_eq!(recent(&Ledger::rebuild(&records, NOW), &room()), vec![]);
    }

    #[test]
    fn rebuild_keeps_every_finish_that_may_have_written() {
        for dispatch in [
            DispatchCertainty::PossiblyDispatched,
            DispatchCertainty::CompleteWrite,
            DispatchCertainty::Unknown,
        ] {
            let records = [
                dispatching(5, 1, ago(minutes(3)), agent(), Some(-70), Some(-64)),
                finished(5, 1, ago(minutes(3)), dispatch.clone()),
            ];
            assert_eq!(
                recent(&Ledger::rebuild(&records, NOW), &room()).len(),
                1,
                "{dispatch:?}"
            );
        }
    }

    #[test]
    fn rebuild_ignores_finished_without_dispatching() {
        let records = [finished(
            5,
            1,
            ago(minutes(3)),
            DispatchCertainty::CompleteWrite,
        )];
        assert_eq!(recent(&Ledger::rebuild(&records, NOW), &room()), vec![]);
    }

    #[test]
    fn rebuild_does_not_pair_a_finish_from_another_run() {
        let records = [
            dispatching(5, 1, ago(minutes(3)), agent(), Some(-70), Some(-64)),
            finished(6, 1, ago(minutes(2)), DispatchCertainty::NotDispatched),
        ];
        assert_eq!(recent(&Ledger::rebuild(&records, NOW), &room()).len(), 1);
    }

    #[test]
    fn rebuild_counts_operator_volume_writes() {
        let records = [dispatching(
            5,
            1,
            ago(minutes(3)),
            Principal::Operator,
            Some(-90),
            Some(-60),
        )];
        assert_eq!(
            recent(&Ledger::rebuild(&records, NOW), &room()),
            vec![change(ago(minutes(3)), Some(-90), -60)]
        );
    }

    #[test]
    fn rebuild_counts_only_volume_writes_inside_the_day() {
        let records = [
            // Not a volume change: nothing to count.
            dispatching(5, 1, ago(minutes(3)), agent(), None, None),
            // A day and a millisecond ago.
            dispatching(
                5,
                2,
                WallTime(ago(hours(24)).as_millis() - 1),
                agent(),
                Some(-70),
                Some(-64),
            ),
            // The service's own events say nothing about the volume.
            record(
                5,
                3,
                ago(minutes(1)),
                Principal::Operator,
                AuditEvent::PolicyLoaded {
                    digest: crate::PolicyDigest::from_bytes([0; 32]),
                },
            ),
        ];
        let ledger = Ledger::rebuild(&records, NOW);
        assert_eq!(ledger.recent(&room(), NOW, hours(48)), vec![]);
    }

    #[test]
    fn rebuild_of_a_volume_it_cannot_read_assumes_the_lowest_level() {
        // A record that does not hold a level on the scale still counts, and as
        // the lowest level, which makes the budget harder to stay inside.
        let records = [dispatching(
            5,
            1,
            ago(minutes(3)),
            agent(),
            Some(500),
            Some(-64),
        )];
        let ledger = Ledger::rebuild(&records, NOW);
        assert_eq!(
            recent(&ledger, &room()),
            vec![RecentChange {
                at: ago(minutes(3)),
                before: Some(Level::MINIMUM),
                target: level(-64),
            }]
        );
    }

    #[test]
    fn a_rebuilt_ledger_answers_as_the_live_one_did() {
        let mut live = Ledger::new();
        let first = change(ago(minutes(6)), Some(-70), -64);
        let second = change(ago(minutes(4)), Some(-64), -58);
        let refused = change(ago(minutes(2)), Some(-58), -52);
        live.reserve(&room(), key(5, 1), first);
        live.settle(&room(), key(5, 1), true);
        live.reserve(&room(), key(5, 2), second);
        live.settle(&room(), key(5, 2), true);
        live.reserve(&room(), key(5, 3), refused);
        live.settle(&room(), key(5, 3), false);

        let records = [
            dispatching(5, 1, first.at, agent(), Some(-70), Some(-64)),
            finished(5, 1, first.at, DispatchCertainty::CompleteWrite),
            dispatching(5, 2, second.at, Principal::Operator, Some(-64), Some(-58)),
            finished(5, 2, second.at, DispatchCertainty::CompleteWrite),
            dispatching(5, 3, refused.at, agent(), Some(-58), Some(-52)),
            finished(5, 3, refused.at, DispatchCertainty::NotDispatched),
        ];
        let rebuilt = Ledger::rebuild(&records, NOW);
        assert_eq!(recent(&rebuilt, &room()), recent(&live, &room()));
        assert_eq!(recent(&rebuilt, &room()), vec![first, second]);
    }
}
