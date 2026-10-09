//! Fakes for the Agent path tests: a clock the test sets, an audit log and a
//! policy source.

use super::*;
use crate::ports::BoxFuture;
use std::sync::atomic::AtomicUsize;

// ---- Fakes ----

pub(super) struct FakeClock(Mutex<WallTime>);

impl FakeClock {
    pub(super) fn at(millis: u64) -> Arc<Self> {
        Arc::new(Self(Mutex::new(WallTime(millis))))
    }

    pub(super) fn advance(&self, by: Duration) {
        let mut now = locked(&self.0);
        *now = now.saturating_add(by);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> WallTime {
        *locked(&self.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AuditMode {
    Working,
    FailAppend,
    HangAppend,
    FailRead,
    /// Neither written nor read.
    FailAll,
    /// Only the `Dispatching` record cannot be written.
    FailDispatching,
}

pub(super) struct FakeAudit {
    pub(super) records: Mutex<Vec<AuditRecord>>,
    mode: Mutex<AuditMode>,
    order: Mutex<Option<OrderLog>>,
    /// When set, an append waits for a permit before it does anything.
    gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
    /// How durably each record was asked to be written, in order.
    pub(super) durability: Mutex<Vec<(&'static str, Durability)>>,
}

impl FakeAudit {
    pub(super) fn new(mode: AuditMode) -> Arc<Self> {
        Arc::new(Self {
            records: Mutex::new(Vec::new()),
            mode: Mutex::new(mode),
            order: Mutex::new(None),
            gate: Mutex::new(None),
            durability: Mutex::new(Vec::new()),
        })
    }

    /// Appends wait until the returned semaphore is given permits.
    pub(super) fn hold_appends(&self) -> Arc<tokio::sync::Semaphore> {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *locked(&self.gate) = Some(Arc::clone(&gate));
        gate
    }

    pub(super) fn set_mode(&self, mode: AuditMode) {
        *locked(&self.mode) = mode;
    }

    pub(super) fn share_order(&self, order: OrderLog) {
        *locked(&self.order) = Some(order);
    }

    pub(super) fn records(&self) -> Vec<AuditRecord> {
        locked(&self.records).clone()
    }

    pub(super) fn events(&self) -> Vec<AuditEvent> {
        self.records()
            .into_iter()
            .map(|record| record.event)
            .collect()
    }
}

pub(super) fn event_name(event: &AuditEvent) -> &'static str {
    match event {
        AuditEvent::Decided { .. } => "audit:decided",
        AuditEvent::Dispatching { .. } => "audit:dispatching",
        AuditEvent::Finished { .. } => "audit:finished",
        AuditEvent::PolicyLoaded { .. } => "audit:policy_loaded",
        AuditEvent::PolicyLoadFailed { .. } => "audit:policy_load_failed",
        AuditEvent::AccessRefused { .. } => "audit:access_refused",
        AuditEvent::TokenIssued { .. } => "audit:token_issued",
        AuditEvent::TokenRevoked { .. } => "audit:token_revoked",
    }
}

impl AuditLog for FakeAudit {
    fn append(
        &self,
        record: AuditRecord,
        durability: Durability,
    ) -> BoxFuture<'_, Result<(), AuditError>> {
        Box::pin(async move {
            let gate = locked(&self.gate).clone();
            if let Some(gate) = gate {
                gate.acquire().await.unwrap().forget();
            }
            let mode = *locked(&self.mode);
            match mode {
                AuditMode::FailAppend | AuditMode::FailAll => {
                    Err(AuditError::new("/private/audit/dir is full"))
                }
                AuditMode::FailDispatching
                    if matches!(record.event, AuditEvent::Dispatching { .. }) =>
                {
                    Err(AuditError::new("/private/audit/dir is full"))
                }
                AuditMode::HangAppend => std::future::pending().await,
                AuditMode::Working | AuditMode::FailRead | AuditMode::FailDispatching => {
                    if let Some(order) = locked(&self.order).as_ref() {
                        locked(order).push(event_name(&record.event).into());
                    }
                    locked(&self.durability).push((event_name(&record.event), durability));
                    locked(&self.records).push(record);
                    Ok(())
                }
            }
        })
    }

    fn since(&self, from: WallTime) -> BoxFuture<'_, Result<Vec<AuditRecord>, AuditError>> {
        Box::pin(async move {
            if matches!(
                *locked(&self.mode),
                AuditMode::FailRead | AuditMode::FailAll
            ) {
                return Err(AuditError::new("/private/audit/dir is unreadable"));
            }
            Ok(self
                .records()
                .into_iter()
                .filter(|record| record.at >= from)
                .collect())
        })
    }

    fn query(&self, query: AuditQuery) -> BoxFuture<'_, Result<AuditPage, AuditError>> {
        Box::pin(async move {
            if matches!(
                *locked(&self.mode),
                AuditMode::FailRead | AuditMode::FailAll
            ) {
                return Err(AuditError::new("/private/audit/dir is unreadable"));
            }
            let records = self.records();
            let entries = records
                .into_iter()
                .enumerate()
                .rev()
                .take(query.limit())
                .map(|(index, record)| AuditEntry {
                    seq: index as u64 + 1,
                    record,
                })
                .collect();
            Ok(AuditPage {
                entries,
                next: None,
            })
        })
    }
}

pub(super) struct FakePolicy {
    result: Mutex<Result<LoadedPolicy, PolicyLoadError>>,
    loads: AtomicUsize,
    /// When set, the next load reads its result and then waits for a permit.
    gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
}

impl FakePolicy {
    pub(super) fn new(result: Result<LoadedPolicy, PolicyLoadError>) -> Arc<Self> {
        Arc::new(Self {
            result: Mutex::new(result),
            loads: AtomicUsize::new(0),
            gate: Mutex::new(None),
        })
    }

    /// The next load reads the file as it is now and then waits for a permit.
    pub(super) fn hold_next_load(&self) -> Arc<tokio::sync::Semaphore> {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *locked(&self.gate) = Some(Arc::clone(&gate));
        gate
    }

    pub(super) fn set(&self, result: Result<LoadedPolicy, PolicyLoadError>) {
        *locked(&self.result) = result;
    }
}

impl PolicySource for FakePolicy {
    fn load(&self) -> BoxFuture<'_, Result<LoadedPolicy, PolicyLoadError>> {
        Box::pin(async move {
            self.loads.fetch_add(1, Ordering::SeqCst);
            let result = locked(&self.result).clone();
            let gate = locked(&self.gate).take();
            if let Some(gate) = gate {
                gate.acquire().await.unwrap().forget();
            }
            result
        })
    }
}
