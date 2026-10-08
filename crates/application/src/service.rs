//! The in-process control service.
//!
//! [`ControlService`] implements the control-service port over receiver
//! sessions it owns. It holds at most one session per receiver, opens it on
//! demand, and releases it after a configurable idle time so the receiver's
//! single control connection is free for other tools. It also hosts the
//! Operation Gate, the only caller of [`CanonicalReceiverSession::operate`]
//! outside tests and the session implementations.
//!
//! A service built by [`ControlService::new`] serves the Operator alone and
//! writes no audit log. One built by [`ControlService::start`] also has the Agent
//! path, in the `agent` module: a policy, an audit log, a budget ledger, and caps.
//! An Agent handle is refused by the first and served by the second.

use crate::audit::{AuditPage, AuditQuery};
use crate::control::{
    AgentLabel, ApprovalHealth, AuditHealth, ConnectionStatus, ControlError, DryRun,
    DryRunDecision, IdempotencyKey, OperationControl, OperationEvent, OperationEventSource,
    OperationEvents, OperationSnapshot, OperationStatus, OperationSubmission, OperatorAdmin,
    PolicyHealth, PolicyView, Principal, ReceiverCapabilities, ReceiverReads, ReceiverSummary,
    ServiceHealth, SharedOperatorControl,
};
use crate::ports::{
    AsyncConfigRepository, AsyncReceiverDiscovery, BoxFuture, OperationError, ReceiverConnector,
};
use crate::receiver_selection::receiver_id;
use crate::session_v3::{OperationRequest, Readiness, SharedReceiverSession, StateSubscription};
use denon_avr_domain::{
    ConfiguredReceivers, DiscoveredReceiver, DispatchCertainty, HttpInformationSnapshot, Model,
    ModelCapabilities, OperationId, OperationOutcome, QuickSelectNameObservation, ReceiverId,
    ReceiverIdentity, ReceiverIntent, SourceCatalogObservation,
};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio::sync::{broadcast, watch};
use tracing::{debug, warn};

mod agent;
#[cfg(test)]
mod agent_tests;
#[cfg(test)]
mod tests;

use agent::AgentState;
pub use agent::{AgentLimits, AgentPath};

/// How many operation events a slow reader may lag behind before it is told it
/// missed some.
const EVENT_BACKLOG: usize = 256;

/// Tunable behavior of the service.
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    /// How long a receiver with no subscriber and no operation in flight stays
    /// connected. Any read or operation restarts the clock.
    pub idle_release: Duration,
    /// The longest `operation(id, wait)` blocks, whatever the caller asks for.
    pub max_operation_wait: Duration,
    /// How many finished operations stay readable.
    pub retained_operations: usize,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            idle_release: Duration::from_secs(60),
            max_operation_wait: Duration::from_secs(30),
            retained_operations: 256,
        }
    }
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn unavailable(error: OperationError) -> ControlError {
    ControlError::Unavailable(error.to_string())
}

/// What the views for policy, audit, and health answer until the service has an
/// Agent path to report on. It is a refusal, which fails closed.
fn no_agent_path() -> ControlError {
    ControlError::Unavailable("this service has no policy, audit log, or agent path".into())
}

/// The in-process implementation of the control-service port.
pub struct ControlService {
    inner: Arc<Inner>,
}

impl ControlService {
    pub fn new(
        connector: Arc<dyn ReceiverConnector>,
        config: Arc<dyn AsyncConfigRepository>,
        discovery: Arc<dyn AsyncReceiverDiscovery>,
        settings: ServiceConfig,
    ) -> Self {
        Self::assemble(connector, config, discovery, settings, None)
    }

    /// A service with the Agent path. It loads the policy, rebuilds the budget
    /// ledger from the audit log, and records both. It cannot fail: a policy that
    /// does not load, or a log that cannot be read, is state that refuses agent
    /// writes until it is put right, and `health` says so.
    pub async fn start(
        connector: Arc<dyn ReceiverConnector>,
        config: Arc<dyn AsyncConfigRepository>,
        discovery: Arc<dyn AsyncReceiverDiscovery>,
        settings: ServiceConfig,
        agents: AgentPath,
    ) -> Self {
        let agent = AgentState::start(agents).await;
        Self::assemble(connector, config, discovery, settings, Some(agent))
    }

    fn assemble(
        connector: Arc<dyn ReceiverConnector>,
        config: Arc<dyn AsyncConfigRepository>,
        discovery: Arc<dyn AsyncReceiverDiscovery>,
        settings: ServiceConfig,
        agent: Option<AgentState>,
    ) -> Self {
        let (events, _) = broadcast::channel(EVENT_BACKLOG);
        Self {
            inner: Arc::new(Inner {
                connector,
                config,
                discovery,
                settings,
                slots: Mutex::new(HashMap::new()),
                ad_hoc: Mutex::new(HashMap::new()),
                operations: Mutex::new(Operations::default()),
                events: Mutex::new(Some(events)),
                in_flight: watch::channel(0).0,
                closed: AtomicBool::new(false),
                agent,
            }),
        }
    }

    /// A handle bound to `principal`. The principal is fixed for the life of
    /// the handle and is never a request parameter.
    ///
    /// An Agent is served only by a service built by [`ControlService::start`];
    /// one built by `new` has no policy, so it refuses rather than handing out
    /// an unguarded handle.
    pub fn handle(&self, principal: Principal) -> Result<Arc<ServiceHandle>, ControlError> {
        match principal {
            Principal::Agent(_) if self.inner.agent.is_none() => Err(ControlError::Forbidden),
            principal => Ok(Arc::new(ServiceHandle {
                inner: Arc::clone(&self.inner),
                principal,
            })),
        }
    }

    /// The handle for the GUI and CLI, as the full port.
    pub fn operator(&self) -> SharedOperatorControl {
        Arc::new(ServiceHandle {
            inner: Arc::clone(&self.inner),
            principal: Principal::Operator,
        })
    }

    /// Stop accepting work, wait for operations in flight to finish, and close
    /// every session. Not part of the port: only the owner of the service ends
    /// it. Safe to call more than once.
    pub async fn shutdown(&self) {
        {
            let mut operations = locked(&self.inner.operations);
            operations.closed = true;
            self.inner.closed.store(true, Ordering::SeqCst);
        }
        let mut idle = self.inner.in_flight.subscribe();
        let _ = idle.wait_for(|count| *count == 0).await;
        let slots: Vec<Arc<Slot>> = locked(&self.inner.slots).values().cloned().collect();
        for slot in slots {
            let mut held = slot.session.lock().await;
            if let Some(open) = held.take() {
                close_held(&slot, open, "during shutdown").await;
            }
        }
        // Ends every event stream.
        locked(&self.inner.events).take();
    }
}

/// One caller's view of the service, bound to a principal.
pub struct ServiceHandle {
    inner: Arc<Inner>,
    principal: Principal,
}

impl ServiceHandle {
    fn require_operator(&self) -> Result<(), ControlError> {
        match self.principal {
            Principal::Operator => Ok(()),
            Principal::Agent(_) => Err(ControlError::Forbidden),
        }
    }

    /// Receivers chosen only by address are for the Operator. To anyone else
    /// they do not exist.
    fn visible_receiver(&self, receiver: &ReceiverId) -> Result<(), ControlError> {
        if receiver.is_ad_hoc() && self.principal != Principal::Operator {
            return Err(ControlError::NotFound("receiver"));
        }
        Ok(())
    }
}

struct Inner {
    connector: Arc<dyn ReceiverConnector>,
    config: Arc<dyn AsyncConfigRepository>,
    discovery: Arc<dyn AsyncReceiverDiscovery>,
    settings: ServiceConfig,
    slots: Mutex<HashMap<ReceiverId, Arc<Slot>>>,
    ad_hoc: Mutex<HashMap<ReceiverId, ReceiverIdentity>>,
    operations: Mutex<Operations>,
    events: Mutex<Option<broadcast::Sender<(Principal, OperationEvent)>>>,
    /// Operations whose task has not ended. Shutdown waits for zero.
    in_flight: watch::Sender<usize>,
    closed: AtomicBool,
    /// The Agent path. `None` on a service built by `new`.
    agent: Option<AgentState>,
}

/// One receiver's connection. The async lock serializes connect, use, and
/// release, which is what makes each of those safe against the others.
struct Slot {
    session: tokio::sync::Mutex<Option<Held>>,
    /// Leases alive: reads, held subscriptions, and operations in flight.
    leases: AtomicUsize,
    /// How many of those leases belong to an operation. Retiring a session
    /// waits for this to reach zero, because closing under an operation in
    /// flight would lose its outcome.
    operations: watch::Sender<usize>,
    /// Bumped whenever a lease is taken or dropped, so an idle timer can tell
    /// whether anything happened after it was armed.
    activity: AtomicU64,
    status: Mutex<ConnectionStatus>,
}

impl Slot {
    fn new() -> Self {
        Self {
            session: tokio::sync::Mutex::new(None),
            leases: AtomicUsize::new(0),
            operations: watch::channel(0).0,
            activity: AtomicU64::new(0),
            status: Mutex::new(ConnectionStatus::Released),
        }
    }
}

/// An open session, with what it was opened for and the signal that ends its
/// subscribers when the service closes it.
struct Held {
    session: SharedReceiverSession,
    /// The identity the session connected with. A saved entry whose host later
    /// differs retires the session.
    identity: ReceiverIdentity,
    ended: watch::Sender<bool>,
}

/// Tell the session's subscribers it is over, then close it. The caller has
/// taken it out of the slot, under the slot lock.
async fn close_held(slot: &Slot, held: Held, why: &str) {
    held.ended.send_replace(true);
    if let Err(error) = held.session.close().await {
        warn!(%error, why, "closing receiver session");
    }
    *locked(&slot.status) = ConnectionStatus::Released;
}

/// Marks a slot as connecting for as long as an attempt runs. A caller can drop
/// the attempt at any await, and the slot must not stay `Connecting` for a
/// connection nobody is making, so an attempt that did not finish puts it back.
struct ConnectAttempt<'a> {
    status: &'a Mutex<ConnectionStatus>,
    finished: bool,
}

impl<'a> ConnectAttempt<'a> {
    fn start(status: &'a Mutex<ConnectionStatus>) -> Self {
        *locked(status) = ConnectionStatus::Connecting;
        Self {
            status,
            finished: false,
        }
    }

    fn finish(mut self, status: ConnectionStatus) {
        *locked(self.status) = status;
        self.finished = true;
    }
}

impl Drop for ConnectAttempt<'_> {
    fn drop(&mut self) {
        if !self.finished {
            *locked(self.status) = ConnectionStatus::Released;
        }
    }
}

/// Keeps a receiver connected while it lives.
struct Lease {
    inner: Arc<Inner>,
    slot: Arc<Slot>,
    session: SharedReceiverSession,
    ended: watch::Receiver<bool>,
    operation: bool,
}

impl Drop for Lease {
    fn drop(&mut self) {
        if self.operation {
            self.slot.operations.send_modify(|count| *count -= 1);
        }
        self.slot.activity.fetch_add(1, Ordering::SeqCst);
        if self.slot.leases.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.inner.arm_idle_timer(&self.slot);
        }
    }
}

impl Inner {
    fn existing_slot(&self, receiver: &ReceiverId) -> Option<Arc<Slot>> {
        locked(&self.slots).get(receiver).cloned()
    }

    /// The receiver's current address, from the saved configuration or, for an
    /// ad hoc receiver, from what the Operator registered.
    async fn identity(&self, receiver: &ReceiverId) -> Result<ReceiverIdentity, ControlError> {
        if receiver.is_ad_hoc() {
            return locked(&self.ad_hoc)
                .get(receiver)
                .cloned()
                .ok_or(ControlError::NotFound("receiver"));
        }
        let configuration = self.config.load().await.map_err(ControlError::Receiver)?;
        configuration
            .receivers
            .get(receiver.as_str())
            .cloned()
            .ok_or(ControlError::NotFound("receiver"))
    }

    /// Take a lease on the receiver's session, connecting and synchronizing
    /// first when there is none. Concurrent callers wait on the slot lock, so
    /// exactly one connection is attempted.
    async fn lease(self: &Arc<Self>, receiver: &ReceiverId) -> Result<Lease, ControlError> {
        self.lease_as(receiver, false).await
    }

    /// As [`Inner::lease`], counting the lease as an operation's when
    /// `operation` is set.
    async fn lease_as(
        self: &Arc<Self>,
        receiver: &ReceiverId,
        operation: bool,
    ) -> Result<Lease, ControlError> {
        let closed = || ControlError::Unavailable("the control service has shut down".into());
        if self.closed.load(Ordering::SeqCst) {
            return Err(closed());
        }
        let slot = match self.existing_slot(receiver) {
            Some(slot) => slot,
            None => {
                // Only receivers that exist get a slot.
                self.identity(receiver).await?;
                let mut slots = locked(&self.slots);
                let slot = slots
                    .entry(receiver.clone())
                    .or_insert_with(|| Arc::new(Slot::new()));
                Arc::clone(slot)
            }
        };
        let mut held = slot.session.lock().await;
        if self.closed.load(Ordering::SeqCst) {
            return Err(closed());
        }
        let (session, ended) = match held.as_ref() {
            Some(open) => (Arc::clone(&open.session), open.ended.subscribe()),
            None => {
                let (session, identity) = self.connect(receiver, &slot).await?;
                let ended = watch::channel(false).0;
                let subscriber = ended.subscribe();
                *held = Some(Held {
                    session: Arc::clone(&session),
                    identity,
                    ended,
                });
                (session, subscriber)
            }
        };
        // Counted while the lock is held, so a release cannot slip in between.
        slot.leases.fetch_add(1, Ordering::SeqCst);
        if operation {
            slot.operations.send_modify(|count| *count += 1);
        }
        slot.activity.fetch_add(1, Ordering::SeqCst);
        Ok(Lease {
            inner: Arc::clone(self),
            slot: Arc::clone(&slot),
            session,
            ended,
            operation,
        })
    }

    async fn connect(
        &self,
        receiver: &ReceiverId,
        slot: &Slot,
    ) -> Result<(SharedReceiverSession, ReceiverIdentity), ControlError> {
        let attempt = ConnectAttempt::start(&slot.status);
        let result = async {
            let identity = self.identity(receiver).await?;
            let session = self
                .connector
                .connect(receiver, &identity)
                .await
                .map_err(unavailable)?;
            // The first state a caller sees is complete.
            if let Err(error) = session.synchronize().await {
                let _ = session.close().await;
                return Err(unavailable(error));
            }
            Ok((session, identity))
        }
        .await;
        attempt.finish(match result {
            Ok(_) => ConnectionStatus::Connected,
            Err(_) => ConnectionStatus::Released,
        });
        result
    }

    /// Close the session of every receiver whose saved entry no longer reaches
    /// the address it connected to, or was removed, so the next lease connects
    /// to the address now in the file.
    ///
    /// Each close takes the slot lock, which keeps a new connection from opening
    /// beside the old one (the receiver accepts a single control connection),
    /// and waits for operations in flight, because a close that outlasts its
    /// grace period aborts the session and would lose an outcome. It does not
    /// wait for subscribers, who can hold a session indefinitely: they are told
    /// it ended and subscribe again.
    async fn retire_changed(&self, saved: &ConfiguredReceivers) {
        let slots: Vec<(ReceiverId, Arc<Slot>)> = locked(&self.slots)
            .iter()
            .map(|(id, slot)| (id.clone(), Arc::clone(slot)))
            .collect();
        for (id, slot) in slots {
            if id.is_ad_hoc() {
                continue;
            }
            let mut held = slot.session.lock().await;
            let unchanged = held.as_ref().is_none_or(|open| {
                saved
                    .receivers
                    .get(id.as_str())
                    .is_some_and(|entry| entry.host == open.identity.host)
            });
            if unchanged {
                continue;
            }
            let mut operations = slot.operations.subscribe();
            let _ = operations.wait_for(|count| *count == 0).await;
            if let Some(open) = held.take() {
                debug!(
                    receiver = id.as_str(),
                    "retiring session for a changed entry"
                );
                close_held(&slot, open, "its saved entry changed").await;
            }
        }
    }

    /// Start the idle clock. One timer runs per time the last lease drops; a
    /// timer that finds activity since it was armed does nothing, because the
    /// lease that caused the activity arms its own when it drops.
    fn arm_idle_timer(self: &Arc<Self>, slot: &Arc<Slot>) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let armed_at = slot.activity.load(Ordering::SeqCst);
        let inner = Arc::clone(self);
        let slot = Arc::clone(slot);
        runtime.spawn(async move {
            tokio::time::sleep(inner.settings.idle_release).await;
            inner.release_if_idle(&slot, armed_at).await;
        });
    }

    /// Close the session if nothing used it since `armed_at`. The check and the
    /// close happen under the slot lock, so a request that arrives during the
    /// close waits for it and then reconnects.
    async fn release_if_idle(&self, slot: &Slot, armed_at: u64) {
        let mut session = slot.session.lock().await;
        if slot.leases.load(Ordering::SeqCst) != 0
            || slot.activity.load(Ordering::SeqCst) != armed_at
        {
            return;
        }
        if let Some(open) = session.take() {
            debug!("releasing idle receiver session");
            close_held(slot, open, "idle").await;
        }
    }

    fn publish(&self, owner: &Principal, snapshot: &OperationSnapshot) {
        if let Some(events) = locked(&self.events).as_ref() {
            // No reader is not an error.
            let _ = events.send((owner.clone(), OperationEvent::Update(snapshot.clone())));
        }
    }
}

/// One operation in the gate's table.
struct Entry {
    owner: Principal,
    key: Option<IdempotencyKey>,
    snapshot: watch::Sender<OperationSnapshot>,
}

#[derive(Default)]
struct Operations {
    next_id: u64,
    closed: bool,
    entries: HashMap<OperationId, Entry>,
    /// Admission order, for evicting the oldest finished operations.
    order: VecDeque<OperationId>,
    keys: HashMap<(Principal, IdempotencyKey), OperationId>,
}

enum Admission {
    /// A new operation: the caller must run it.
    New(OperationSnapshot),
    /// An operation that already exists: nothing more to run.
    Existing(OperationSnapshot),
}

/// How an operation ended, in the terms clients see.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Resolution {
    status: OperationStatus,
    dispatch: DispatchCertainty,
    confirmed: bool,
    reason: Option<String>,
    observation: Option<String>,
}

impl Resolution {
    fn not_dispatched(status: OperationStatus, reason: impl Into<String>) -> Self {
        Self {
            status,
            dispatch: DispatchCertainty::NotDispatched,
            confirmed: false,
            reason: Some(reason.into()),
            observation: None,
        }
    }
}

/// Map the session's outcome to what clients see, as the design's lifecycle
/// table specifies. The session's report is not reinterpreted.
fn resolve(outcome: OperationOutcome) -> Resolution {
    match outcome {
        OperationOutcome::ObservedRequestedValue {
            dispatch,
            observation,
            ..
        } => Resolution {
            status: OperationStatus::Completed,
            dispatch,
            confirmed: true,
            reason: None,
            observation: Some(observation),
        },
        OperationOutcome::AlreadyObserved { observation, .. } => Resolution {
            status: OperationStatus::AlreadyInState,
            dispatch: DispatchCertainty::NotDispatched,
            confirmed: true,
            reason: None,
            observation: Some(observation),
        },
        OperationOutcome::RejectedBeforeDispatch { reason, .. } => {
            Resolution::not_dispatched(OperationStatus::Rejected, reason)
        }
        OperationOutcome::Cancelled { .. } => Resolution {
            status: OperationStatus::Cancelled,
            dispatch: DispatchCertainty::NotDispatched,
            confirmed: false,
            reason: None,
            observation: None,
        },
        OperationOutcome::SupersededBeforeDispatch { by, .. } => Resolution {
            status: OperationStatus::Superseded { by },
            dispatch: DispatchCertainty::NotDispatched,
            confirmed: false,
            reason: None,
            observation: None,
        },
        OperationOutcome::Indeterminate {
            dispatch, reason, ..
        } => Resolution {
            status: OperationStatus::Indeterminate,
            dispatch,
            confirmed: false,
            reason: Some(reason),
            observation: None,
        },
    }
}

fn is_cancellable(status: &OperationStatus) -> bool {
    matches!(
        status,
        OperationStatus::Submitted
            | OperationStatus::Allowed
            | OperationStatus::AwaitingApproval
            | OperationStatus::Approved
    )
}

impl Inner {
    /// Record a new operation, or find the existing one a retry refers to.
    fn admit(
        &self,
        owner: &Principal,
        receiver: &ReceiverId,
        submission: OperationSubmission,
    ) -> Result<Admission, ControlError> {
        let mut operations = locked(&self.operations);
        if operations.closed {
            return Err(ControlError::Unavailable(
                "the control service has shut down".into(),
            ));
        }
        if let Some(key) = &submission.idempotency_key {
            if let Some(id) = operations.keys.get(&(owner.clone(), key.clone())) {
                let existing = operations.entries[id].snapshot.borrow().clone();
                if existing.receiver != *receiver || existing.intent != submission.intent {
                    return Err(ControlError::InvalidRequest(
                        "the idempotency key was already used for a different request".into(),
                    ));
                }
                return Ok(Admission::Existing(existing));
            }
        }
        // An identical request already running is the same request retried.
        for entry in operations.entries.values() {
            if entry.owner != *owner {
                continue;
            }
            let existing = entry.snapshot.borrow();
            if !existing.status.is_terminal()
                && existing.receiver == *receiver
                && existing.intent == submission.intent
            {
                return Ok(Admission::Existing(existing.clone()));
            }
        }
        // A new request. An agent's is counted against its caps, which a retry
        // of one that exists (above) is not.
        let status = match owner {
            // An Operator request is evaluated trivially as allowed.
            Principal::Operator => OperationStatus::Allowed,
            // An agent's waits for the policy to decide it.
            Principal::Agent(label) => {
                let agent = self.agent.as_ref().ok_or(ControlError::Forbidden)?;
                agent.admit_write(label, &operations)?;
                OperationStatus::Submitted
            }
        };
        self.evict_finished(&mut operations);
        operations.next_id += 1;
        let id = OperationId(operations.next_id);
        let snapshot = OperationSnapshot {
            id,
            receiver: receiver.clone(),
            intent: submission.intent,
            status,
            dispatch: DispatchCertainty::NotDispatched,
            confirmed: false,
            reason: None,
            observation: None,
        };
        if let Some(key) = &submission.idempotency_key {
            operations.keys.insert((owner.clone(), key.clone()), id);
        }
        operations.entries.insert(
            id,
            Entry {
                owner: owner.clone(),
                key: submission.idempotency_key,
                snapshot: watch::channel(snapshot.clone()).0,
            },
        );
        operations.order.push_back(id);
        self.in_flight.send_modify(|count| *count += 1);
        Ok(Admission::New(snapshot))
    }

    /// Drop the oldest finished operations beyond the retention limit. Operations
    /// still running are never dropped.
    fn evict_finished(&self, operations: &mut Operations) {
        let mut excess = operations
            .entries
            .len()
            .saturating_sub(self.settings.retained_operations.saturating_sub(1));
        if excess == 0 {
            return;
        }
        let mut kept = VecDeque::with_capacity(operations.order.len());
        while let Some(id) = operations.order.pop_front() {
            let finished = operations
                .entries
                .get(&id)
                .is_some_and(|entry| entry.snapshot.borrow().status.is_terminal());
            if excess > 0 && finished {
                if let Some(entry) = operations.entries.remove(&id) {
                    if let Some(key) = entry.key {
                        operations.keys.remove(&(entry.owner, key));
                    }
                }
                excess -= 1;
            } else {
                kept.push_back(id);
            }
        }
        operations.order = kept;
    }

    /// Change an operation and tell its readers. Returns the new snapshot, or
    /// `None` when the change was refused or the operation is gone.
    fn transition(
        &self,
        id: OperationId,
        change: impl FnOnce(&mut OperationSnapshot) -> bool,
    ) -> Option<OperationSnapshot> {
        let (owner, snapshot) = {
            let operations = locked(&self.operations);
            let entry = operations.entries.get(&id)?;
            let mut applied = false;
            entry.snapshot.send_if_modified(|snapshot| {
                applied = change(snapshot);
                applied
            });
            if !applied {
                return None;
            }
            let snapshot = entry.snapshot.borrow().clone();
            (entry.owner.clone(), snapshot)
        };
        self.publish(&owner, &snapshot);
        Some(snapshot)
    }

    /// Move an operation into the session unless it was cancelled first. The
    /// move and a cancel are decided under one lock, so exactly one of them wins.
    fn begin_session(&self, id: OperationId) -> bool {
        self.transition(id, |snapshot| {
            if snapshot.status == OperationStatus::Allowed {
                snapshot.status = OperationStatus::InSession;
                true
            } else {
                false
            }
        })
        .is_some()
    }

    fn finish(&self, id: OperationId, resolution: Resolution) {
        self.transition(id, |snapshot| {
            if snapshot.status.is_terminal() {
                return false;
            }
            snapshot.status = resolution.status;
            snapshot.dispatch = resolution.dispatch;
            snapshot.confirmed = resolution.confirmed;
            snapshot.reason = resolution.reason;
            snapshot.observation = resolution.observation;
            true
        });
    }

    /// The operation as `viewer` may see it. The Operator sees every operation;
    /// anyone else sees only their own, and cannot tell a hidden one from a
    /// missing one.
    fn watch_operation(
        &self,
        viewer: &Principal,
        id: OperationId,
    ) -> Result<watch::Receiver<OperationSnapshot>, ControlError> {
        let operations = locked(&self.operations);
        operations
            .entries
            .get(&id)
            .filter(|entry| *viewer == Principal::Operator || entry.owner == *viewer)
            .map(|entry| entry.snapshot.subscribe())
            .ok_or(ControlError::NotFound("operation"))
    }

    fn cancel(
        &self,
        viewer: &Principal,
        id: OperationId,
    ) -> Result<OperationSnapshot, ControlError> {
        let current = self.watch_operation(viewer, id)?.borrow().clone();
        if !is_cancellable(&current.status) {
            return Err(ControlError::TooLate(Box::new(current)));
        }
        let withdrawn = self.transition(id, |snapshot| {
            if is_cancellable(&snapshot.status) {
                snapshot.status = OperationStatus::Cancelled;
                true
            } else {
                false
            }
        });
        match withdrawn {
            Some(snapshot) => Ok(snapshot),
            // The operation reached the session between the read and the change.
            None => {
                let actual = self.watch_operation(viewer, id)?.borrow().clone();
                Err(ControlError::TooLate(Box::new(actual)))
            }
        }
    }
}

/// Ends an operation's task. If the task ended without resolving the operation,
/// which only a panic can cause, the operation is resolved honestly instead of
/// being left running forever.
struct OperationTask {
    inner: Arc<Inner>,
    id: OperationId,
}

impl Drop for OperationTask {
    fn drop(&mut self) {
        let unresolved = self.inner.transition(self.id, |snapshot| {
            if snapshot.status.is_terminal() {
                return false;
            }
            let in_session = snapshot.status == OperationStatus::InSession;
            snapshot.status = if in_session {
                OperationStatus::Indeterminate
            } else {
                OperationStatus::Rejected
            };
            snapshot.dispatch = if in_session {
                DispatchCertainty::Unknown
            } else {
                DispatchCertainty::NotDispatched
            };
            snapshot.confirmed = false;
            snapshot.reason = Some("the operation task ended unexpectedly".into());
            true
        });
        if unresolved.is_some() {
            warn!(operation = self.id.0, "operation task ended unresolved");
        }
        self.inner.in_flight.send_modify(|count| *count -= 1);
    }
}

/// Run one admitted operation to its end. The task belongs to the service, so a
/// caller that goes away never abandons the call into the session: the session
/// reads a dropped call as cancellation and would lose the outcome.
async fn run_operation(
    inner: Arc<Inner>,
    id: OperationId,
    receiver: ReceiverId,
    intent: ReceiverIntent,
    owner: Principal,
) {
    let _task = OperationTask {
        inner: Arc::clone(&inner),
        id,
    };
    // An agent's request meets the policy first. `admit` only lets an agent's
    // operation exist on a service that has an Agent path.
    let agent = match (&owner, inner.agent.as_ref()) {
        (Principal::Agent(label), Some(state)) => Some((label, state)),
        _ => None,
    };
    let policy = match agent {
        Some((label, state)) => match state.precheck().await {
            Ok(policy) => Some(policy),
            Err(reason) => {
                finish_agent(&inner, state, id, label, &receiver, agent::refusal(reason)).await;
                return;
            }
        },
        None => None,
    };
    let lease = match inner.lease_as(&receiver, true).await {
        Ok(lease) => lease,
        Err(error) => {
            // An agent is not told where the receiver is, only that it is out of reach.
            let resolution = match agent {
                Some(_) => agent::refusal(agent::RECEIVER_UNREACHABLE),
                None => Resolution::not_dispatched(OperationStatus::Rejected, error.to_string()),
            };
            match agent {
                Some((label, state)) => {
                    finish_agent(&inner, state, id, label, &receiver, resolution).await
                }
                None => inner.finish(id, resolution),
            }
            return;
        }
    };
    if let (Some((label, state)), Some(policy)) = (agent, policy) {
        let resolution = agent::decide(state, &lease, id, &receiver, &intent, label, &policy).await;
        finish_agent(&inner, state, id, label, &receiver, resolution).await;
        return;
    }
    if !inner.begin_session(id) {
        // Withdrawn while the receiver was connecting. Nothing was dispatched.
        return;
    }
    // The only call that can write to the receiver, made exactly once.
    let outcome = lease
        .session
        .operate(OperationRequest::new(id, intent))
        .await;
    inner.finish(id, resolve(outcome));
}

/// End an agent's operation and record how it ended.
async fn finish_agent(
    inner: &Inner,
    agent: &AgentState,
    id: OperationId,
    label: &AgentLabel,
    receiver: &ReceiverId,
    resolution: Resolution,
) {
    inner.finish(id, resolution.clone());
    agent
        .record_finished(id, label, receiver, &resolution)
        .await;
}

impl ReceiverReads for ServiceHandle {
    fn receivers(&self) -> BoxFuture<'_, Result<Vec<ReceiverSummary>, ControlError>> {
        Box::pin(async move {
            let configuration = self
                .inner
                .config
                .load()
                .await
                .map_err(ControlError::Receiver)?;
            let mut summaries = Vec::with_capacity(configuration.receivers.len());
            for (name, identity) in &configuration.receivers {
                let Ok(id) = ReceiverId::new(name.as_str()) else {
                    continue;
                };
                let model = Model::from_reported(identity.model.as_deref().unwrap_or_default());
                let connection = self
                    .inner
                    .existing_slot(&id)
                    .map_or(ConnectionStatus::Released, |slot| *locked(&slot.status));
                summaries.push(ReceiverSummary {
                    id,
                    model: identity.model.clone(),
                    capabilities: ReceiverCapabilities::from(&ModelCapabilities::for_model(model)),
                    connection,
                });
            }
            Ok(summaries)
        })
    }

    fn state<'a>(
        &'a self,
        receiver: &'a ReceiverId,
    ) -> BoxFuture<'a, Result<StateSubscription, ControlError>> {
        Box::pin(async move {
            self.visible_receiver(receiver)?;
            let lease = self.inner.lease(receiver).await?;
            Ok(lease
                .session
                .state()
                .ending_with(lease.ended.clone())
                .holding(lease))
        })
    }

    fn source_catalog<'a>(
        &'a self,
        receiver: &'a ReceiverId,
    ) -> BoxFuture<'a, Result<SourceCatalogObservation, ControlError>> {
        Box::pin(async move {
            self.visible_receiver(receiver)?;
            let lease = self.inner.lease(receiver).await?;
            lease
                .session
                .source_catalog()
                .await
                .map_err(ControlError::Receiver)
        })
    }

    fn health(&self) -> BoxFuture<'_, Result<ServiceHealth, ControlError>> {
        Box::pin(async move {
            Ok(match &self.inner.agent {
                Some(agent) => agent.health(),
                // No Agent path: nothing to judge, record, or count with.
                None => ServiceHealth {
                    policy: PolicyHealth::NotConfigured,
                    audit: AuditHealth::NotConfigured,
                    ledger_ready: false,
                    approval: ApprovalHealth::Unavailable,
                },
            })
        })
    }
}

impl OperationControl for ServiceHandle {
    fn submit<'a>(
        &'a self,
        receiver: &'a ReceiverId,
        submission: OperationSubmission,
    ) -> BoxFuture<'a, Result<OperationSnapshot, ControlError>> {
        Box::pin(async move {
            self.visible_receiver(receiver)?;
            // An unknown receiver is refused before an operation exists for it.
            self.inner.identity(receiver).await?;
            let admission = self.inner.admit(&self.principal, receiver, submission)?;
            let snapshot = match admission {
                Admission::Existing(snapshot) => return Ok(snapshot),
                Admission::New(snapshot) => snapshot,
            };
            // The first snapshot is `allowed` for the Operator and `submitted`
            // for an agent; the task decides the rest.
            self.inner.publish(&self.principal, &snapshot);
            tokio::spawn(run_operation(
                Arc::clone(&self.inner),
                snapshot.id,
                snapshot.receiver.clone(),
                snapshot.intent.clone(),
                self.principal.clone(),
            ));
            Ok(snapshot)
        })
    }

    fn operation(
        &self,
        id: OperationId,
        wait: Option<Duration>,
    ) -> BoxFuture<'_, Result<OperationSnapshot, ControlError>> {
        Box::pin(async move {
            let mut watched = self.inner.watch_operation(&self.principal, id)?;
            let current = watched.borrow().clone();
            let Some(wait) = wait.filter(|_| !current.status.is_terminal()) else {
                return Ok(current);
            };
            let wait = wait.min(self.inner.settings.max_operation_wait);
            let _ = tokio::time::timeout(wait, watched.wait_for(|s| s.status.is_terminal())).await;
            let latest = watched.borrow().clone();
            Ok(latest)
        })
    }

    fn cancel(&self, id: OperationId) -> BoxFuture<'_, Result<OperationSnapshot, ControlError>> {
        Box::pin(async move { self.inner.cancel(&self.principal, id) })
    }

    fn operation_events(&self) -> BoxFuture<'_, Result<OperationEvents, ControlError>> {
        Box::pin(async move {
            let events = locked(&self.inner.events)
                .as_ref()
                .map(broadcast::Sender::subscribe)
                .ok_or_else(|| {
                    ControlError::Unavailable("the control service has shut down".into())
                })?;
            Ok(Box::new(EventStream {
                events,
                viewer: self.principal.clone(),
            }) as OperationEvents)
        })
    }

    fn dry_run<'a>(
        &'a self,
        receiver: &'a ReceiverId,
        intent: ReceiverIntent,
    ) -> BoxFuture<'a, Result<DryRun, ControlError>> {
        Box::pin(async move {
            self.visible_receiver(receiver)?;
            // An unknown receiver is refused here, as it is for a submission.
            self.inner.identity(receiver).await?;
            match &self.principal {
                // The Operator skips policy, so there is nothing to evaluate.
                Principal::Operator => Ok(DryRun {
                    decision: DryRunDecision::Allow,
                    policy: self.inner.agent.as_ref().and_then(AgentState::digest),
                }),
                Principal::Agent(label) => {
                    let agent = self.inner.agent.as_ref().ok_or(ControlError::Forbidden)?;
                    // A dry run opens the receiver, so it is a write against the cap.
                    agent.charge_write(label)?;
                    agent::dry_run(&self.inner, agent, label.as_str(), receiver, &intent).await
                }
            }
        })
    }
}

/// Operation events for one viewer: all of them for the Operator, the viewer's
/// own otherwise.
struct EventStream {
    events: broadcast::Receiver<(Principal, OperationEvent)>,
    viewer: Principal,
}

impl OperationEventSource for EventStream {
    fn next(&mut self) -> BoxFuture<'_, Option<OperationEvent>> {
        Box::pin(async move {
            loop {
                match self.events.recv().await {
                    Ok((owner, event)) => {
                        if self.viewer == Principal::Operator || owner == self.viewer {
                            return Some(event);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        return Some(OperationEvent::Missed)
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        })
    }
}

impl OperatorAdmin for ServiceHandle {
    fn discover(
        &self,
        timeout: Duration,
    ) -> BoxFuture<'_, Result<Vec<DiscoveredReceiver>, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            self.inner
                .discovery
                .discover(timeout)
                .await
                .map_err(ControlError::Receiver)
        })
    }

    fn register_ad_hoc(
        &self,
        identity: ReceiverIdentity,
    ) -> BoxFuture<'_, Result<ReceiverId, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let id = receiver_id(None, &identity)
                .map_err(|error| ControlError::InvalidRequest(error.message))?;
            locked(&self.inner.ad_hoc).insert(id.clone(), identity);
            Ok(id)
        })
    }

    fn configuration(&self) -> BoxFuture<'_, Result<ConfiguredReceivers, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            self.inner
                .config
                .load()
                .await
                .map_err(ControlError::Receiver)
        })
    }

    fn save_configuration<'a>(
        &'a self,
        configuration: &'a ConfiguredReceivers,
    ) -> BoxFuture<'a, Result<(), ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            configuration
                .validate()
                .map_err(ControlError::InvalidRequest)?;
            self.inner
                .config
                .save(configuration)
                .await
                .map_err(ControlError::Receiver)?;
            // A session open to an address the file no longer holds would
            // otherwise serve it until released, and a held subscription keeps
            // it from ever being released.
            self.inner.retire_changed(configuration).await;
            Ok(())
        })
    }

    fn quick_select_names<'a>(
        &'a self,
        receiver: &'a ReceiverId,
    ) -> BoxFuture<'a, Result<QuickSelectNameObservation, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let lease = self.inner.lease(receiver).await?;
            lease
                .session
                .quick_select_names()
                .await
                .map_err(ControlError::Receiver)
        })
    }

    fn http_information<'a>(
        &'a self,
        receiver: &'a ReceiverId,
    ) -> BoxFuture<'a, Result<HttpInformationSnapshot, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let lease = self.inner.lease(receiver).await?;
            lease
                .session
                .http_information()
                .await
                .map_err(ControlError::Receiver)
        })
    }

    fn refresh<'a>(
        &'a self,
        receiver: &'a ReceiverId,
    ) -> BoxFuture<'a, Result<Readiness, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let lease = self.inner.lease(receiver).await?;
            lease
                .session
                .synchronize()
                .await
                .map_err(ControlError::Receiver)
        })
    }

    fn dry_run_as<'a>(
        &'a self,
        agent: AgentLabel,
        receiver: &'a ReceiverId,
        intent: ReceiverIntent,
    ) -> BoxFuture<'a, Result<DryRun, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let state = self.inner.agent.as_ref().ok_or_else(no_agent_path)?;
            self.inner.identity(receiver).await?;
            // The Operator testing a tier does not spend the agent's allowance.
            agent::dry_run(&self.inner, state, agent.as_str(), receiver, &intent).await
        })
    }

    fn policy(&self) -> BoxFuture<'_, Result<PolicyView, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let agent = self.inner.agent.as_ref().ok_or_else(no_agent_path)?;
            Ok(agent.policy_view())
        })
    }

    fn reload_policy(&self) -> BoxFuture<'_, Result<PolicyView, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let agent = self.inner.agent.as_ref().ok_or_else(no_agent_path)?;
            Ok(agent.load_policy().await)
        })
    }

    fn audit(&self, query: AuditQuery) -> BoxFuture<'_, Result<AuditPage, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let agent = self.inner.agent.as_ref().ok_or_else(no_agent_path)?;
            agent.audit_page(query).await
        })
    }
}
