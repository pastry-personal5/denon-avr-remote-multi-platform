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
//!
//! The gate's executor, which makes the one call into a session's `operate`, is
//! here. The `connections` module holds the per-receiver connection pool: slots,
//! leases, connecting, and idle and changed-entry release. The `operations`
//! module holds the gate's table of operations: admission, transitions,
//! cancellation, and how an outcome resolves. The `ports` module implements the
//! port's traits for [`ServiceHandle`].

use crate::audit::AccessRefusal;
use crate::control::{
    ControlError, OperationEvent, OperationStatus, Principal, SharedOperatorControl,
};
use crate::ports::{
    AsyncConfigRepository, AsyncReceiverDiscovery, OperationError, ReceiverConnector,
};
use crate::session_v3::OperationRequest;
use denon_avr_domain::{
    DispatchCertainty, OperationId, ReceiverId, ReceiverIdentity, ReceiverIntent,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio::sync::{broadcast, watch};
use tracing::warn;

mod agent;
#[cfg(test)]
mod agent_tests;
mod connections;
mod operations;
mod ports;
#[cfg(test)]
mod tests;

use agent::AgentState;
pub use agent::{AgentLimits, AgentPath};
use connections::{close_held, Lease, Slot};
use operations::{resolve, Operations, Resolution};

/// How many operation events a slow reader may lag behind before it is told it
/// missed some.
const EVENT_BACKLOG: usize = 256;

/// What a caller is told once the service has shut down.
const SHUT_DOWN: &str = "the control service has shut down";

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

    /// Record a caller refused at an endpoint, in the audit log, at a bounded rate.
    /// The Control API server calls this for every refusal. A service built by
    /// [`ControlService::new`] has no log and records nothing.
    pub async fn record_refusal(&self, refusal: AccessRefusal) {
        if let Some(agent) = &self.inner.agent {
            agent.record_access_refusal(refusal).await;
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

    /// An error as this caller may see it. The Operator sees every error whole.
    /// An agent is never told where a receiver is or what a file path is, which
    /// the connector's and the repository's messages can say, so what reaches it
    /// from a connection or a read is fixed text.
    fn sanitized(&self, error: ControlError) -> ControlError {
        if self.principal == Principal::Operator {
            return error;
        }
        match error {
            ControlError::Unavailable(message) if message != SHUT_DOWN => {
                ControlError::Unavailable(agent::RECEIVER_UNREACHABLE.into())
            }
            ControlError::Receiver(error) => ControlError::Receiver(OperationError::new(
                error.kind,
                error.context,
                "the receiver could not complete the request",
            )),
            other => other,
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
    if let Some((_, state)) = agent {
        if let Err(reason) = state.precheck().await {
            let resolution = agent::refusal(reason);
            inner.finish(id, resolution.clone());
            state
                .record_refused(id, &owner, &receiver, &resolution)
                .await;
            return;
        }
    }
    let lease = match inner.lease_as(&receiver, true).await {
        Ok(lease) => lease,
        Err(error) => {
            match agent {
                // An agent is not told where the receiver is, only that it is out of reach.
                Some((_, state)) => {
                    let resolution = agent::refusal(agent::RECEIVER_UNREACHABLE);
                    finish_agent(&inner, state, id, &owner, &receiver, resolution, None).await;
                }
                None => {
                    let resolution =
                        Resolution::not_dispatched(OperationStatus::Rejected, error.to_string());
                    inner.finish(id, resolution);
                }
            }
            return;
        }
    };

    let mut precondition = None;
    let mut settlement = None;
    if let Some((label, state)) = agent {
        let request = agent::Request {
            id,
            receiver: &receiver,
            intent: &intent,
            label,
        };
        match agent::decide(&inner, state, &lease, &request).await {
            agent::Gate::Stop(resolution) => {
                finish_agent(&inner, state, id, &owner, &receiver, resolution, None).await;
                return;
            }
            agent::Gate::Dispatch {
                precondition: bound,
                settlement: reserved,
            } => {
                precondition = Some(bound);
                settlement = Some(reserved);
            }
        }
    } else if let Some(state) = inner.agent.as_ref() {
        // The Operator on a service with an audit log: recorded and counted,
        // never limited.
        settlement = Some(
            state
                .begin_operator_write(&lease, id, &receiver, &intent)
                .await,
        );
    }

    if !inner.begin_session(id) {
        // Withdrawn while the receiver was connecting or the record was being
        // written. Nothing was dispatched, so nothing stays counted.
        if let (Some(state), Some(settlement)) = (inner.agent.as_ref(), &settlement) {
            state.settle(settlement, false);
            state
                .record_finished(id, &owner, &receiver, &agent::cancelled())
                .await;
        }
        return;
    }
    let mut request = OperationRequest::new(id, intent);
    if let Some(bound) = precondition {
        request = request.with_precondition(bound);
    }
    // The only call that can write to the receiver, made exactly once.
    let outcome = lease.session.operate(request).await;
    // An agent is told a fixed sentence for what the session reports, not the
    // session's text, which can name an address. The log keeps the text.
    let shown = match owner {
        Principal::Agent(_) => agent::fixed_reason(&outcome),
        Principal::Operator => None,
    };
    let resolution = resolve(outcome);
    match (inner.agent.as_ref(), &settlement) {
        (Some(state), Some(settlement)) => {
            // Settle before the operation reads as finished, so whoever asks
            // next sees the ledger as this write left it.
            state.settle(
                settlement,
                resolution.dispatch != DispatchCertainty::NotDispatched,
            );
            finish_agent(&inner, state, id, &owner, &receiver, resolution, shown).await;
        }
        _ => {
            inner.finish(id, resolution);
        }
    }
}

/// End an operation on a service with an audit log, and record how it ended.
async fn finish_agent(
    inner: &Inner,
    state: &AgentState,
    id: OperationId,
    owner: &Principal,
    receiver: &ReceiverId,
    resolution: Resolution,
    shown: Option<&str>,
) {
    // The snapshot carries `shown` when there is one; the log keeps `resolution`.
    let mut visible = resolution.clone();
    if let Some(text) = shown {
        visible.reason = Some(text.into());
    }
    let recorded = if inner.finish(id, visible) {
        resolution
    } else {
        // A cancel won while the gate was deciding. The log says what the client
        // was told, not what the gate would have said.
        inner.ended_as(id).unwrap_or(resolution)
    };
    state.record_finished(id, owner, receiver, &recorded).await;
}
