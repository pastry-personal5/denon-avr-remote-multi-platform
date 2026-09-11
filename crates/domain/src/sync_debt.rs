//! Pure synchronization-debt bookkeeping for the canonical receiver actor.

use crate::{CoreField, Epoch, MonotonicMillis, ReceiverId, SyncCause, SyncCycleId};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncDebt {
    pub cycle: SyncCycleId,
    pub receiver: ReceiverId,
    pub epoch: Epoch,
    pub fields: BTreeSet<CoreField>,
    pub causes: BTreeSet<SyncCause>,
    pub activity_seq: u64,
    pub quick_reconcile_at: MonotonicMillis,
    pub final_reconcile_at: Option<MonotonicMillis>,
}

impl SyncDebt {
    pub fn new(
        receiver: ReceiverId,
        epoch: Epoch,
        cycle: SyncCycleId,
        field: CoreField,
        cause: SyncCause,
        now: MonotonicMillis,
    ) -> Self {
        Self {
            cycle,
            receiver,
            epoch,
            fields: BTreeSet::from([field]),
            causes: BTreeSet::from([cause]),
            activity_seq: 0,
            quick_reconcile_at: MonotonicMillis(now.0 + 250),
            final_reconcile_at: None,
        }
    }
    pub fn merge(&mut self, fields: impl IntoIterator<Item = CoreField>, cause: SyncCause) {
        self.fields.extend(fields);
        self.causes.insert(cause);
    }
    pub fn note_activity(&mut self, now: MonotonicMillis) {
        self.activity_seq = self.activity_seq.saturating_add(1);
        self.quick_reconcile_at = MonotonicMillis(now.0 + 250);
    }
    pub fn defer_final_until(&mut self, at: MonotonicMillis) {
        self.final_reconcile_at = Some(
            self.final_reconcile_at
                .map_or(at, |current| current.max(at)),
        );
    }
    pub fn defer_quick_until(&mut self, at: MonotonicMillis) {
        self.quick_reconcile_at = at;
    }
    pub fn quick_due(&self, now: MonotonicMillis) -> bool {
        now >= self.quick_reconcile_at
    }
    pub fn final_due(&self, now: MonotonicMillis) -> bool {
        self.final_reconcile_at
            .is_some_and(|deadline| now >= deadline)
    }
    pub fn is_for(&self, receiver: &ReceiverId, epoch: Epoch) -> bool {
        &self.receiver == receiver && self.epoch == epoch
    }
    pub fn settle_field(&mut self, field: CoreField) {
        self.fields.remove(&field);
    }
    pub fn settled(&self) -> bool {
        self.fields.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn debt_merges_fields_and_extends_quiet_window() {
        let mut debt = SyncDebt::new(
            ReceiverId::new("test").unwrap(),
            Epoch(1),
            SyncCycleId(1),
            CoreField::Volume,
            SyncCause::LocalControl,
            MonotonicMillis(0),
        );
        debt.merge([CoreField::Mute], SyncCause::ReceiverEvent);
        debt.note_activity(MonotonicMillis(100));
        assert_eq!(debt.quick_reconcile_at, MonotonicMillis(350));
        assert!(!debt.quick_due(MonotonicMillis(349)));
        assert!(debt.quick_due(MonotonicMillis(350)));
        debt.defer_final_until(MonotonicMillis(5_000));
        assert!(!debt.final_due(MonotonicMillis(4_999)));
        assert!(debt.final_due(MonotonicMillis(5_000)));
        assert_eq!(debt.causes.len(), 2);
    }
}
