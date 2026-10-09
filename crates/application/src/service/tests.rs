//! Service tests run on a paused clock with a fake connector and session, so
//! idle release and waiting are exact and nothing touches a socket.

use super::*;
use crate::control::{
    AgentLabel, ConnectionStatus, IdempotencyKey, OperationSnapshot, OperationSubmission,
    OperatorAdmin, ReceiverReads,
};
use crate::ports::{BoxFuture, OperationErrorKind};
use crate::session_v3::{
    CanonicalReceiverSession, Readiness, SharedReceiverSession, StateSubscription,
};
use denon_avr_domain::{
    ConfiguredReceivers, DiscoveredReceiver, MasterVolume, MuteState, OperationOutcome,
    QuickSelectNameObservation, ReceiverState, RejectionCause, SourceCatalogObservation, ZonePower,
};
use std::collections::BTreeMap;
use std::sync::atomic::AtomicUsize;
use tokio::sync::Notify;

pub(super) type Script = Arc<dyn Fn(&OperationRequest) -> OperationOutcome + Send + Sync>;

/// What a fake session does to its own state when it is asked to write, and the
/// answer it gives instead of the script's when it refuses. Tests of the Agent
/// path use it to move the volume as a real receiver would, and to refuse a
/// write whose precondition no longer holds.
pub(super) type Effect = Arc<
    dyn Fn(&OperationRequest, &watch::Sender<ReceiverState>) -> Option<OperationOutcome>
        + Send
        + Sync,
>;

/// A log that sessions and audit logs share, so a test can see which came first.
pub(super) type OrderLog = Arc<Mutex<Vec<String>>>;

pub(super) fn completed(request: &OperationRequest) -> OperationOutcome {
    OperationOutcome::ObservedRequestedValue {
        operation: request.id,
        dispatch: DispatchCertainty::CompleteWrite,
        observation: "seen".into(),
    }
}

pub(super) struct FakeSession {
    pub(super) states: watch::Sender<ReceiverState>,
    calls: Mutex<Vec<OperationRequest>>,
    synchronizes: AtomicUsize,
    closes: AtomicUsize,
    script: Script,
    /// Signalled when `operate` is entered.
    entered: Arc<Notify>,
    /// When set, `operate` waits for it before answering.
    hold_operate: Option<Arc<Notify>>,
    /// When set, `close` waits for it before returning.
    hold_close: Option<Arc<Notify>>,
    /// When set, run before the script.
    effect: Option<Effect>,
    /// When set, told when `operate` is entered.
    order: Option<OrderLog>,
}

impl FakeSession {
    pub(super) fn calls(&self) -> Vec<OperationRequest> {
        locked(&self.calls).clone()
    }
    fn closes(&self) -> usize {
        self.closes.load(Ordering::SeqCst)
    }
}

impl CanonicalReceiverSession for FakeSession {
    fn state(&self) -> StateSubscription {
        StateSubscription::new(self.states.subscribe())
    }

    fn synchronize(&self) -> BoxFuture<'_, Result<Readiness, OperationError>> {
        Box::pin(async move {
            self.synchronizes.fetch_add(1, Ordering::SeqCst);
            Ok(Readiness {
                ready: true,
                degraded: false,
                detail: "fake".into(),
            })
        })
    }

    fn operate(&self, request: OperationRequest) -> BoxFuture<'_, OperationOutcome> {
        Box::pin(async move {
            locked(&self.calls).push(request.clone());
            if let Some(order) = &self.order {
                locked(order).push("operate".into());
            }
            self.entered.notify_one();
            if let Some(hold) = &self.hold_operate {
                hold.notified().await;
            }
            if let Some(effect) = &self.effect {
                if let Some(outcome) = effect(&request, &self.states) {
                    return outcome;
                }
            }
            (self.script)(&request)
        })
    }

    fn source_catalog(&self) -> BoxFuture<'_, Result<SourceCatalogObservation, OperationError>> {
        Box::pin(async move {
            Ok(SourceCatalogObservation {
                catalog: denon_avr_domain::SourceCatalog {
                    generation: 7,
                    ..Default::default()
                },
                raw_response: "<rx/>".into(),
                response_evidence: denon_avr_domain::CatalogResponseEvidence::Complete,
            })
        })
    }

    fn quick_select_names(
        &self,
    ) -> BoxFuture<'_, Result<QuickSelectNameObservation, OperationError>> {
        Box::pin(async move {
            Ok(QuickSelectNameObservation {
                generation: 7,
                ..Default::default()
            })
        })
    }

    // `http_information` is left at the contract's default so a test can see an
    // unsupported read pass through.

    fn close(&self) -> BoxFuture<'_, Result<(), OperationError>> {
        Box::pin(async move {
            if let Some(hold) = &self.hold_close {
                hold.notified().await;
            }
            self.closes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

pub(super) struct FakeConnector {
    sessions: Mutex<Vec<Arc<FakeSession>>>,
    /// The identity each connection attempt was made with, in order.
    identities: Mutex<Vec<ReceiverIdentity>>,
    attempts: AtomicUsize,
    fail: AtomicBool,
    script: Mutex<Script>,
    entered: Arc<Notify>,
    hold_connect: Mutex<Option<Arc<Notify>>>,
    hold_operate: Mutex<Option<Arc<Notify>>>,
    hold_close: Mutex<Option<Arc<Notify>>>,
    initial: Mutex<Option<ReceiverState>>,
    effect: Mutex<Option<Effect>>,
    order: Mutex<Option<OrderLog>>,
}

impl FakeConnector {
    pub(super) fn new() -> Self {
        Self {
            sessions: Mutex::new(Vec::new()),
            identities: Mutex::new(Vec::new()),
            attempts: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
            script: Mutex::new(Arc::new(completed)),
            entered: Arc::new(Notify::new()),
            hold_connect: Mutex::new(None),
            hold_operate: Mutex::new(None),
            hold_close: Mutex::new(None),
            initial: Mutex::new(None),
            effect: Mutex::new(None),
            order: Mutex::new(None),
        }
    }
    /// Sessions start in this state instead of an empty one.
    pub(super) fn start_in(&self, state: ReceiverState) {
        *locked(&self.initial) = Some(state);
    }
    /// What a session does when it is asked to write.
    pub(super) fn effect(&self, effect: Effect) {
        *locked(&self.effect) = Some(effect);
    }
    pub(super) fn order(&self, order: OrderLog) {
        *locked(&self.order) = Some(order);
    }
    pub(super) fn attempts(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }
    pub(super) fn session(&self, index: usize) -> Arc<FakeSession> {
        Arc::clone(&locked(&self.sessions)[index])
    }
    pub(super) fn sessions(&self) -> usize {
        locked(&self.sessions).len()
    }
    fn hosts(&self) -> Vec<String> {
        locked(&self.identities)
            .iter()
            .map(|identity| identity.host.clone())
            .collect()
    }
    /// Every connection attempt from now on is refused.
    pub(super) fn fail_connections(&self) {
        self.fail.store(true, Ordering::SeqCst);
    }
    pub(super) fn hold_connect(&self) -> Arc<Notify> {
        let gate = Arc::new(Notify::new());
        *locked(&self.hold_connect) = Some(Arc::clone(&gate));
        gate
    }
    pub(super) fn hold_operate(&self) -> Arc<Notify> {
        let gate = Arc::new(Notify::new());
        *locked(&self.hold_operate) = Some(Arc::clone(&gate));
        gate
    }
    fn hold_close(&self) -> Arc<Notify> {
        let gate = Arc::new(Notify::new());
        *locked(&self.hold_close) = Some(Arc::clone(&gate));
        gate
    }
    pub(super) fn script(
        &self,
        script: impl Fn(&OperationRequest) -> OperationOutcome + Send + Sync + 'static,
    ) {
        *locked(&self.script) = Arc::new(script);
    }
}

impl ReceiverConnector for FakeConnector {
    fn connect<'a>(
        &'a self,
        receiver: &'a ReceiverId,
        identity: &'a ReceiverIdentity,
    ) -> BoxFuture<'a, Result<SharedReceiverSession, OperationError>> {
        Box::pin(async move {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            locked(&self.identities).push(identity.clone());
            let hold = locked(&self.hold_connect).clone();
            if let Some(hold) = hold {
                hold.notified().await;
            }
            if self.fail.load(Ordering::SeqCst) {
                return Err(OperationError::new(
                    OperationErrorKind::Connection,
                    "connecting",
                    "refused",
                ));
            }
            let session = Arc::new(FakeSession {
                states: watch::channel(
                    locked(&self.initial)
                        .clone()
                        .unwrap_or_else(|| ReceiverState::new(receiver.clone())),
                )
                .0,
                calls: Mutex::new(Vec::new()),
                synchronizes: AtomicUsize::new(0),
                closes: AtomicUsize::new(0),
                script: Arc::clone(&locked(&self.script)),
                entered: Arc::clone(&self.entered),
                hold_operate: locked(&self.hold_operate).clone(),
                hold_close: locked(&self.hold_close).clone(),
                effect: locked(&self.effect).clone(),
                order: locked(&self.order).clone(),
            });
            locked(&self.sessions).push(Arc::clone(&session));
            Ok(session as SharedReceiverSession)
        })
    }
}

pub(super) struct FakeConfig(pub(super) Mutex<ConfiguredReceivers>);

impl AsyncConfigRepository for FakeConfig {
    fn load(&self) -> BoxFuture<'_, Result<ConfiguredReceivers, OperationError>> {
        Box::pin(async move { Ok(locked(&self.0).clone()) })
    }
    fn save<'a>(
        &'a self,
        config: &'a ConfiguredReceivers,
    ) -> BoxFuture<'a, Result<(), OperationError>> {
        Box::pin(async move {
            *locked(&self.0) = config.clone();
            Ok(())
        })
    }
}

pub(super) struct FakeDiscovery(pub(super) Vec<DiscoveredReceiver>);

impl AsyncReceiverDiscovery for FakeDiscovery {
    fn discover(
        &self,
        _timeout: Duration,
    ) -> BoxFuture<'_, Result<Vec<DiscoveredReceiver>, OperationError>> {
        Box::pin(async move { Ok(self.0.clone()) })
    }
}

pub(super) struct Harness {
    pub(super) service: Arc<ControlService>,
    pub(super) operator: SharedOperatorControl,
    pub(super) connector: Arc<FakeConnector>,
    pub(super) config: Arc<FakeConfig>,
}

const IDLE: Duration = Duration::from_secs(60);

pub(super) fn harness() -> Harness {
    harness_with(ServiceConfig {
        idle_release: IDLE,
        ..ServiceConfig::default()
    })
}

fn harness_with(settings: ServiceConfig) -> Harness {
    let connector = Arc::new(FakeConnector::new());
    let config = Arc::new(FakeConfig(Mutex::new(ConfiguredReceivers {
        current: Some("living-room".into()),
        receivers: BTreeMap::from([(
            "living-room".into(),
            ReceiverIdentity {
                host: "192.0.2.10".into(),
                model: Some("AVR-X3800H".into()),
                friendly_name: None,
            },
        )]),
        ..ConfiguredReceivers::default()
    })));
    let service = Arc::new(ControlService::new(
        connector.clone(),
        config.clone(),
        Arc::new(FakeDiscovery(vec![DiscoveredReceiver {
            address: denon_avr_domain::ReceiverEndpoint {
                host: "192.0.2.10".into(),
                port: 80,
            },
            location: None,
            server: None,
            model: Some("AVR-X3800H".into()),
            search_target: None,
            unique_service_name: None,
        }])),
        settings,
    ));
    Harness {
        operator: service.operator(),
        service,
        connector,
        config,
    }
}

pub(super) fn living_room() -> ReceiverId {
    ReceiverId::new("living-room").unwrap()
}

fn volume(half_steps: i16) -> OperationSubmission {
    OperationSubmission::new(ReceiverIntent::Volume(
        MasterVolume::db_half_steps(half_steps).unwrap(),
    ))
}

/// Let every ready task run, including ones woken by what just ran.
pub(super) async fn settle() {
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }
}

/// Let `by` of virtual time pass. Sleeping, rather than `tokio::time::advance`,
/// lets the paused clock step through every earlier timer in order and run the
/// tasks each one wakes before the sleep itself returns.
pub(super) async fn advance(by: Duration) {
    tokio::time::sleep(by).await;
    settle().await;
}

async fn finished(harness: &Harness, id: OperationId) -> OperationSnapshot {
    let snapshot = harness
        .operator
        .operation(id, Some(Duration::from_secs(5)))
        .await
        .unwrap();
    assert!(snapshot.status.is_terminal(), "{snapshot:?}");
    snapshot
}

#[test]
fn every_session_outcome_maps_to_status_dispatch_and_confirmed() {
    let operation = OperationId(3);
    let cases = [
        (
            OperationOutcome::ObservedRequestedValue {
                operation,
                dispatch: DispatchCertainty::CompleteWrite,
                observation: "seen".into(),
            },
            OperationStatus::Completed,
            DispatchCertainty::CompleteWrite,
            true,
        ),
        (
            OperationOutcome::ObservedRequestedValue {
                operation,
                dispatch: DispatchCertainty::Unknown,
                observation: "seen after an ambiguous write".into(),
            },
            OperationStatus::Completed,
            DispatchCertainty::Unknown,
            true,
        ),
        (
            OperationOutcome::ObservedRequestedValue {
                operation,
                dispatch: DispatchCertainty::PossiblyDispatched,
                observation: "seen".into(),
            },
            OperationStatus::Completed,
            DispatchCertainty::PossiblyDispatched,
            true,
        ),
        (
            OperationOutcome::AlreadyObserved {
                operation,
                observation: "already".into(),
            },
            OperationStatus::AlreadyInState,
            DispatchCertainty::NotDispatched,
            true,
        ),
        (
            OperationOutcome::RejectedBeforeDispatch {
                operation,
                cause: RejectionCause::UnsupportedIntent,
                reason: "unsupported".into(),
            },
            OperationStatus::Rejected,
            DispatchCertainty::NotDispatched,
            false,
        ),
        (
            OperationOutcome::Cancelled { operation },
            OperationStatus::Cancelled,
            DispatchCertainty::NotDispatched,
            false,
        ),
        (
            OperationOutcome::SupersededBeforeDispatch {
                operation,
                by: OperationId(9),
            },
            OperationStatus::Superseded { by: OperationId(9) },
            DispatchCertainty::NotDispatched,
            false,
        ),
        (
            OperationOutcome::Indeterminate {
                operation,
                dispatch: DispatchCertainty::CompleteWrite,
                reason: "not seen in time".into(),
            },
            OperationStatus::Indeterminate,
            DispatchCertainty::CompleteWrite,
            false,
        ),
        (
            OperationOutcome::Indeterminate {
                operation,
                dispatch: DispatchCertainty::Unknown,
                reason: "ambiguous".into(),
            },
            OperationStatus::Indeterminate,
            DispatchCertainty::Unknown,
            false,
        ),
        (
            OperationOutcome::Indeterminate {
                operation,
                dispatch: DispatchCertainty::NotDispatched,
                reason: "stopped".into(),
            },
            OperationStatus::Indeterminate,
            DispatchCertainty::NotDispatched,
            false,
        ),
    ];
    for (outcome, status, dispatch, confirmed) in cases {
        let resolved = resolve(outcome.clone());
        assert_eq!(resolved.status, status, "{outcome:?}");
        assert_eq!(resolved.dispatch, dispatch, "{outcome:?}");
        assert_eq!(resolved.confirmed, confirmed, "{outcome:?}");
    }
}

#[test]
fn confirmed_never_follows_from_a_completed_write_alone() {
    // A write that completed locally but was not observed is not confirmed.
    let resolved = resolve(OperationOutcome::Indeterminate {
        operation: OperationId(1),
        dispatch: DispatchCertainty::CompleteWrite,
        reason: "not seen".into(),
    });
    assert!(!resolved.confirmed);
    assert_eq!(resolved.reason.as_deref(), Some("not seen"));
}

#[tokio::test(start_paused = true)]
async fn an_operation_dispatches_once_and_reports_the_session_outcome() {
    let h = harness();
    let snapshot = h
        .operator
        .submit(&living_room(), volume(-78))
        .await
        .unwrap();
    // An Operator request is evaluated trivially as allowed.
    assert_eq!(snapshot.status, OperationStatus::Allowed);
    assert_eq!(snapshot.dispatch, DispatchCertainty::NotDispatched);
    assert!(!snapshot.confirmed);

    let done = finished(&h, snapshot.id).await;
    assert_eq!(done.status, OperationStatus::Completed);
    assert_eq!(done.dispatch, DispatchCertainty::CompleteWrite);
    assert!(done.confirmed);
    assert_eq!(done.observation.as_deref(), Some("seen"));

    let calls = h.connector.session(0).calls();
    assert_eq!(calls.len(), 1);
    // The session was called with the id the service allocated.
    assert_eq!(calls[0].id, snapshot.id);
    assert_eq!(calls[0].intent, snapshot.intent);
}

#[tokio::test(start_paused = true)]
async fn the_service_allocates_distinct_ascending_operation_ids() {
    let h = harness();
    let first = h
        .operator
        .submit(&living_room(), volume(-78))
        .await
        .unwrap();
    finished(&h, first.id).await;
    let second = h
        .operator
        .submit(&living_room(), volume(-70))
        .await
        .unwrap();
    finished(&h, second.id).await;
    assert!(second.id > first.id);
    let ids: Vec<_> = h
        .connector
        .session(0)
        .calls()
        .iter()
        .map(|call| call.id)
        .collect();
    assert_eq!(ids, vec![first.id, second.id]);
}

#[tokio::test(start_paused = true)]
async fn a_retry_with_the_same_idempotency_key_returns_the_operation_and_dispatches_once() {
    let h = harness();
    let key = IdempotencyKey::new("retry-1").unwrap();
    let first = h
        .operator
        .submit(
            &living_room(),
            volume(-78).with_idempotency_key(key.clone()),
        )
        .await
        .unwrap();
    finished(&h, first.id).await;

    let retry = h
        .operator
        .submit(
            &living_room(),
            volume(-78).with_idempotency_key(key.clone()),
        )
        .await
        .unwrap();
    assert_eq!(retry.id, first.id);
    assert_eq!(retry.status, OperationStatus::Completed);
    assert_eq!(h.connector.session(0).calls().len(), 1);

    let error = h
        .operator
        .submit(&living_room(), volume(-70).with_idempotency_key(key))
        .await
        .unwrap_err();
    assert!(
        matches!(error, ControlError::InvalidRequest(_)),
        "{error:?}"
    );
    assert_eq!(h.connector.session(0).calls().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_retry_while_the_first_is_in_flight_does_not_dispatch_twice() {
    let h = harness();
    let release = h.connector.hold_operate();
    let key = IdempotencyKey::new("retry-2").unwrap();

    let first = h
        .operator
        .submit(
            &living_room(),
            volume(-78).with_idempotency_key(key.clone()),
        )
        .await
        .unwrap();
    h.connector.entered.notified().await;
    let retry = h
        .operator
        .submit(&living_room(), volume(-78).with_idempotency_key(key))
        .await
        .unwrap();
    // The same request without a key is also the same operation.
    let identical = h
        .operator
        .submit(&living_room(), volume(-78))
        .await
        .unwrap();
    assert_eq!(retry.id, first.id);
    assert_eq!(identical.id, first.id);
    assert_eq!(retry.status, OperationStatus::InSession);

    release.notify_one();
    finished(&h, first.id).await;
    assert_eq!(h.connector.session(0).calls().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_request_identical_to_a_finished_one_is_a_new_operation() {
    let h = harness();
    let first = h
        .operator
        .submit(&living_room(), volume(-78))
        .await
        .unwrap();
    finished(&h, first.id).await;
    let again = h
        .operator
        .submit(&living_room(), volume(-78))
        .await
        .unwrap();
    finished(&h, again.id).await;
    assert_ne!(again.id, first.id);
    assert_eq!(h.connector.session(0).calls().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn cancelling_before_the_session_is_called_prevents_any_dispatch() {
    let h = harness();
    let connect = h.connector.hold_connect();
    let snapshot = h
        .operator
        .submit(&living_room(), volume(-78))
        .await
        .unwrap();
    settle().await;
    assert_eq!(h.connector.attempts(), 1);

    let cancelled = h.operator.cancel(snapshot.id).await.unwrap();
    assert_eq!(cancelled.status, OperationStatus::Cancelled);
    assert_eq!(cancelled.dispatch, DispatchCertainty::NotDispatched);

    connect.notify_one();
    settle().await;
    assert!(h.connector.session(0).calls().is_empty());
    let after = finished(&h, snapshot.id).await;
    assert_eq!(after.status, OperationStatus::Cancelled);
    assert!(!after.confirmed);

    let too_late = h.operator.cancel(snapshot.id).await.unwrap_err();
    assert!(matches!(too_late, ControlError::TooLate(_)), "{too_late:?}");
}

#[tokio::test(start_paused = true)]
async fn cancelling_after_the_session_is_called_is_too_late_and_the_call_completes() {
    let h = harness();
    let release = h.connector.hold_operate();
    let snapshot = h
        .operator
        .submit(&living_room(), volume(-78))
        .await
        .unwrap();
    h.connector.entered.notified().await;

    let error = h.operator.cancel(snapshot.id).await.unwrap_err();
    match error {
        ControlError::TooLate(actual) => assert_eq!(actual.status, OperationStatus::InSession),
        other => panic!("expected TooLate, got {other:?}"),
    }

    release.notify_one();
    let done = finished(&h, snapshot.id).await;
    assert_eq!(done.status, OperationStatus::Completed);
    assert_eq!(h.connector.session(0).calls().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_caller_that_goes_away_does_not_abandon_the_session_call() {
    let h = harness();
    let release = h.connector.hold_operate();
    let operator = Arc::clone(&h.operator);
    let id = {
        // The caller's task ends right after submitting.
        tokio::spawn(async move {
            operator
                .submit(&living_room(), volume(-78))
                .await
                .unwrap()
                .id
        })
        .await
        .unwrap()
    };
    h.connector.entered.notified().await;
    release.notify_one();
    let done = finished(&h, id).await;
    assert_eq!(done.status, OperationStatus::Completed);
}

#[tokio::test(start_paused = true)]
async fn concurrent_first_requests_connect_once() {
    let h = harness();
    let connect = h.connector.hold_connect();
    let handle = h.service.handle(Principal::Operator).unwrap();
    let readers: Vec<_> = (0..3)
        .map(|_| {
            let handle = Arc::clone(&handle);
            tokio::spawn(async move { handle.state(&living_room()).await.map(drop) })
        })
        .collect();
    settle().await;
    assert_eq!(h.connector.attempts(), 1);
    connect.notify_one();
    for reader in readers {
        reader.await.unwrap().unwrap();
    }
    assert_eq!(h.connector.attempts(), 1);
    assert_eq!(h.connector.sessions(), 1);
    assert_eq!(
        h.connector.session(0).synchronizes.load(Ordering::SeqCst),
        1,
        "the first state a caller sees is synchronized"
    );
}

#[tokio::test(start_paused = true)]
async fn a_caller_that_gives_up_while_connecting_does_not_leave_the_receiver_connecting() {
    let h = harness();
    let connect = h.connector.hold_connect();
    let handle = h.service.handle(Principal::Operator).unwrap();
    let request = tokio::spawn(async move { handle.state(&living_room()).await.map(drop) });
    settle().await;
    assert_eq!(
        h.operator.receivers().await.unwrap()[0].connection,
        ConnectionStatus::Connecting
    );

    request.abort();
    settle().await;
    assert_eq!(
        h.operator.receivers().await.unwrap()[0].connection,
        ConnectionStatus::Released,
        "nothing is connecting once the only caller has gone"
    );

    // The next request connects as usual. The gate keeps one permit for it.
    connect.notify_one();
    let subscription = h.operator.state(&living_room()).await.unwrap();
    assert_eq!(h.connector.attempts(), 2);
    assert_eq!(
        h.operator.receivers().await.unwrap()[0].connection,
        ConnectionStatus::Connected
    );
    drop(subscription);
}

#[tokio::test(start_paused = true)]
async fn an_idle_receiver_is_released_and_the_next_request_reconnects() {
    let h = harness();
    drop(h.operator.state(&living_room()).await.unwrap());
    assert_eq!(h.connector.attempts(), 1);

    advance(IDLE - Duration::from_secs(1)).await;
    assert_eq!(h.connector.session(0).closes(), 0, "not idle long enough");
    advance(Duration::from_secs(2)).await;
    assert_eq!(h.connector.session(0).closes(), 1);
    let summaries = h.operator.receivers().await.unwrap();
    assert_eq!(summaries[0].connection, ConnectionStatus::Released);

    drop(h.operator.state(&living_room()).await.unwrap());
    assert_eq!(h.connector.attempts(), 2);
    assert_eq!(h.connector.session(0).closes(), 1);
    assert_eq!(h.connector.session(1).closes(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_held_subscription_keeps_the_receiver_connected() {
    let h = harness();
    let subscription = h.operator.state(&living_room()).await.unwrap();
    advance(IDLE * 10).await;
    assert_eq!(h.connector.session(0).closes(), 0);
    assert_eq!(
        h.operator.receivers().await.unwrap()[0].connection,
        ConnectionStatus::Connected
    );

    drop(subscription);
    advance(IDLE - Duration::from_secs(1)).await;
    assert_eq!(
        h.connector.session(0).closes(),
        0,
        "the clock restarts at drop"
    );
    advance(Duration::from_secs(2)).await;
    assert_eq!(h.connector.session(0).closes(), 1);
}

#[tokio::test(start_paused = true)]
async fn an_operation_in_flight_keeps_the_receiver_connected() {
    let h = harness();
    let release = h.connector.hold_operate();
    let snapshot = h
        .operator
        .submit(&living_room(), volume(-78))
        .await
        .unwrap();
    h.connector.entered.notified().await;

    advance(IDLE * 10).await;
    assert_eq!(h.connector.session(0).closes(), 0);

    release.notify_one();
    finished(&h, snapshot.id).await;
    settle().await;
    advance(IDLE + Duration::from_secs(1)).await;
    assert_eq!(h.connector.session(0).closes(), 1);
}

#[tokio::test(start_paused = true)]
async fn activity_restarts_the_idle_clock() {
    let h = harness();
    drop(h.operator.state(&living_room()).await.unwrap());
    advance(IDLE / 2).await;
    drop(h.operator.state(&living_room()).await.unwrap());
    // The first timer fires now and finds that the receiver was used since.
    advance(IDLE / 2 + Duration::from_secs(1)).await;
    assert_eq!(h.connector.session(0).closes(), 0);
    assert_eq!(h.connector.attempts(), 1);
    advance(IDLE / 2).await;
    assert_eq!(h.connector.session(0).closes(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_request_during_release_waits_for_the_close_and_gets_a_new_session() {
    let h = harness();
    let close = h.connector.hold_close();
    drop(h.operator.state(&living_room()).await.unwrap());
    // The idle timer fires and starts closing, which blocks on the gate.
    advance(IDLE + Duration::from_secs(1)).await;
    assert_eq!(h.connector.session(0).closes(), 0);

    let handle = h.service.handle(Principal::Operator).unwrap();
    let request = tokio::spawn(async move { handle.state(&living_room()).await.map(drop) });
    settle().await;
    assert!(!request.is_finished(), "it must wait for the release");
    assert_eq!(
        h.connector.attempts(),
        1,
        "and must not reuse a closing session"
    );

    close.notify_one();
    request.await.unwrap().unwrap();
    assert_eq!(h.connector.session(0).closes(), 1);
    assert_eq!(h.connector.attempts(), 2);
    assert_eq!(h.connector.session(1).closes(), 0);
}

#[tokio::test(start_paused = true)]
async fn an_unknown_receiver_is_not_found_and_nothing_connects() {
    let h = harness();
    let bedroom = ReceiverId::new("bedroom").unwrap();
    assert_eq!(
        h.operator.submit(&bedroom, volume(-78)).await.unwrap_err(),
        ControlError::NotFound("receiver")
    );
    assert!(matches!(
        h.operator.state(&bedroom).await,
        Err(ControlError::NotFound("receiver"))
    ));
    assert!(matches!(
        h.operator.source_catalog(&bedroom).await,
        Err(ControlError::NotFound("receiver"))
    ));
    assert_eq!(h.connector.attempts(), 0);
}

#[tokio::test(start_paused = true)]
async fn an_unreachable_receiver_rejects_the_operation_without_dispatch_and_recovers() {
    let h = harness();
    h.connector.fail.store(true, Ordering::SeqCst);
    let snapshot = h
        .operator
        .submit(&living_room(), volume(-78))
        .await
        .unwrap();
    let done = finished(&h, snapshot.id).await;
    assert_eq!(done.status, OperationStatus::Rejected);
    assert_eq!(done.dispatch, DispatchCertainty::NotDispatched);
    assert!(!done.confirmed);
    assert!(done.reason.unwrap().contains("refused"));
    assert_eq!(h.connector.sessions(), 0);

    h.connector.fail.store(false, Ordering::SeqCst);
    let retry = h
        .operator
        .submit(&living_room(), volume(-70))
        .await
        .unwrap();
    assert_eq!(
        finished(&h, retry.id).await.status,
        OperationStatus::Completed
    );
    assert_eq!(h.connector.attempts(), 2);

    let error = h.operator.state(&living_room()).await.map(drop);
    assert!(error.is_ok());
}

#[tokio::test(start_paused = true)]
async fn an_unreachable_receiver_fails_a_read_with_a_typed_error() {
    let h = harness();
    h.connector.fail.store(true, Ordering::SeqCst);
    let error = match h.operator.state(&living_room()).await {
        Ok(_) => panic!("must not connect"),
        Err(error) => error,
    };
    assert!(matches!(error, ControlError::Unavailable(_)), "{error:?}");
    assert_eq!(
        h.operator.receivers().await.unwrap()[0].connection,
        ConnectionStatus::Released
    );
}

#[tokio::test(start_paused = true)]
async fn an_agent_is_not_handed_a_handle_until_the_policy_path_exists() {
    let h = harness();
    let agent = Principal::Agent(AgentLabel::new("openclaw").unwrap());
    assert!(matches!(
        h.service.handle(agent),
        Err(ControlError::Forbidden)
    ));
    assert!(h.service.handle(Principal::Operator).is_ok());
}

#[tokio::test(start_paused = true)]
async fn the_administration_views_are_the_operators_alone_whatever_the_service_can_answer() {
    let h = harness();
    let intent = ReceiverIntent::Mute(MuteState::On);
    let agent_label = AgentLabel::new("openclaw").unwrap();

    // A service without an Agent path has no policy or log to show the Operator.
    let is_unavailable = |error: ControlError| matches!(error, ControlError::Unavailable(_));
    assert!(is_unavailable(h.operator.policy().await.unwrap_err()));
    assert!(is_unavailable(
        h.operator.reload_policy().await.unwrap_err()
    ));
    assert!(is_unavailable(
        h.operator
            .audit(crate::audit::AuditQuery::new(10))
            .await
            .unwrap_err()
    ));
    assert!(is_unavailable(
        h.operator
            .dry_run_as(agent_label.clone(), &living_room(), intent.clone())
            .await
            .unwrap_err()
    ));

    // An agent is refused them before that is even asked.
    let agent = ServiceHandle {
        inner: Arc::clone(&h.service.inner),
        principal: Principal::Agent(agent_label.clone()),
    };
    assert_eq!(
        agent
            .dry_run_as(agent_label, &living_room(), intent)
            .await
            .unwrap_err(),
        ControlError::Forbidden
    );
    assert_eq!(agent.policy().await.unwrap_err(), ControlError::Forbidden);
    assert_eq!(
        agent.reload_policy().await.unwrap_err(),
        ControlError::Forbidden
    );
    assert_eq!(
        agent
            .audit(crate::audit::AuditQuery::new(10))
            .await
            .unwrap_err(),
        ControlError::Forbidden
    );
    assert_eq!(h.connector.attempts(), 0);
}

#[tokio::test(start_paused = true)]
async fn the_agent_view_carries_no_address_and_ad_hoc_receivers_are_not_listed() {
    let h = harness();
    let ad_hoc = h
        .operator
        .register_ad_hoc(ReceiverIdentity::ad_hoc("192.0.2.50"))
        .await
        .unwrap();
    assert!(ad_hoc.is_ad_hoc());
    // The Operator can use it...
    drop(h.operator.state(&ad_hoc).await.unwrap());
    // ...but it is not a saved receiver, so it is never listed.
    let listed = h.operator.receivers().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, living_room());
    assert_eq!(listed[0].model.as_deref(), Some("AVR-X3800H"));
    assert!(listed[0].capabilities.writable);
    assert!(!format!("{:?}", listed[0]).contains("192.0.2"));

    assert!(matches!(
        h.operator
            .register_ad_hoc(ReceiverIdentity::ad_hoc(" "))
            .await,
        Err(ControlError::InvalidRequest(_))
    ));
}

#[tokio::test(start_paused = true)]
async fn listing_reports_the_connection_state() {
    let h = harness();
    assert_eq!(
        h.operator.receivers().await.unwrap()[0].connection,
        ConnectionStatus::Released
    );
    let subscription = h.operator.state(&living_room()).await.unwrap();
    assert_eq!(
        h.operator.receivers().await.unwrap()[0].connection,
        ConnectionStatus::Connected
    );
    drop(subscription);
}

#[tokio::test(start_paused = true)]
async fn configuration_round_trips_and_is_validated_before_saving() {
    let h = harness();
    let mut configuration = h.operator.configuration().await.unwrap();
    configuration
        .receivers
        .insert("den".into(), ReceiverIdentity::ad_hoc("192.0.2.11"));
    h.operator.save_configuration(&configuration).await.unwrap();
    assert_eq!(locked(&h.config.0).receivers.len(), 2);

    configuration.current = Some("missing".into());
    assert!(matches!(
        h.operator.save_configuration(&configuration).await,
        Err(ControlError::InvalidRequest(_))
    ));
    assert_eq!(locked(&h.config.0).receivers.len(), 2);
    assert_eq!(
        h.operator
            .discover(Duration::from_secs(1))
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test(start_paused = true)]
async fn waiting_for_an_operation_is_bounded_by_the_request_and_the_cap() {
    let h = harness();
    let release = h.connector.hold_operate();
    let snapshot = h
        .operator
        .submit(&living_room(), volume(-78))
        .await
        .unwrap();
    h.connector.entered.notified().await;

    // The wait elapses with the operation still running.
    let running = h
        .operator
        .operation(snapshot.id, Some(Duration::from_secs(1)))
        .await
        .unwrap();
    assert_eq!(running.status, OperationStatus::InSession);
    // No wait is a plain read.
    let plain = h.operator.operation(snapshot.id, None).await.unwrap();
    assert_eq!(plain.status, OperationStatus::InSession);

    // A request for an hour is clamped to the server's cap.
    let operator = Arc::clone(&h.operator);
    let id = snapshot.id;
    let long = tokio::spawn(async move {
        operator
            .operation(id, Some(Duration::from_secs(3600)))
            .await
            .unwrap()
    });
    advance(Duration::from_secs(31)).await;
    assert!(long.is_finished());
    assert_eq!(long.await.unwrap().status, OperationStatus::InSession);

    // And a wait returns as soon as the operation finishes.
    let operator = Arc::clone(&h.operator);
    let waiting = tokio::spawn(async move {
        operator
            .operation(id, Some(Duration::from_secs(20)))
            .await
            .unwrap()
    });
    settle().await;
    assert!(!waiting.is_finished());
    release.notify_one();
    assert_eq!(waiting.await.unwrap().status, OperationStatus::Completed);
}

#[tokio::test(start_paused = true)]
async fn the_operator_sees_every_stage_of_an_operation_as_an_event() {
    let h = harness();
    let mut events = h.operator.operation_events().await.unwrap();
    let snapshot = h
        .operator
        .submit(&living_room(), volume(-78))
        .await
        .unwrap();
    let mut seen = Vec::new();
    while let Some(OperationEvent::Update(update)) = events.next().await {
        assert_eq!(update.id, snapshot.id);
        let terminal = update.status.is_terminal();
        seen.push(update.status);
        if terminal {
            break;
        }
    }
    assert_eq!(
        seen,
        vec![
            OperationStatus::Allowed,
            OperationStatus::InSession,
            OperationStatus::Completed
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn shutdown_waits_for_operations_in_flight_and_closes_sessions() {
    let h = harness();
    let release = h.connector.hold_operate();
    let mut events = h.operator.operation_events().await.unwrap();
    let snapshot = h
        .operator
        .submit(&living_room(), volume(-78))
        .await
        .unwrap();
    h.connector.entered.notified().await;

    let service = Arc::clone(&h.service);
    let shutdown = tokio::spawn(async move { service.shutdown().await });
    settle().await;
    assert!(
        !shutdown.is_finished(),
        "an operation is still in the session"
    );
    assert!(matches!(
        h.operator.submit(&living_room(), volume(-70)).await,
        Err(ControlError::Unavailable(_))
    ));

    release.notify_one();
    shutdown.await.unwrap();
    assert_eq!(h.connector.session(0).closes(), 1);
    assert_eq!(h.connector.session(0).calls().len(), 1);
    assert_eq!(
        h.operator
            .operation(snapshot.id, None)
            .await
            .unwrap()
            .status,
        OperationStatus::Completed
    );
    // Draining the stream ends it.
    while events.next().await.is_some() {}
    assert!(matches!(
        h.operator.state(&living_room()).await,
        Err(ControlError::Unavailable(_))
    ));
}

#[tokio::test(start_paused = true)]
async fn only_the_most_recent_finished_operations_stay_readable() {
    let h = harness_with(ServiceConfig {
        idle_release: IDLE,
        retained_operations: 4,
        ..ServiceConfig::default()
    });
    let mut ids = Vec::new();
    for step in 0..10 {
        let snapshot = h
            .operator
            .submit(&living_room(), volume(-80 + step))
            .await
            .unwrap();
        finished(&h, snapshot.id).await;
        ids.push(snapshot.id);
    }
    assert_eq!(
        h.operator.operation(ids[0], None).await.unwrap_err(),
        ControlError::NotFound("operation")
    );
    assert!(h.operator.operation(ids[9], None).await.is_ok());
    let readable = {
        let mut count = 0;
        for id in &ids {
            if h.operator.operation(*id, None).await.is_ok() {
                count += 1;
            }
        }
        count
    };
    assert!(readable <= 4, "{readable} operations retained");
}

#[tokio::test(start_paused = true)]
async fn a_session_that_panics_leaves_the_operation_indeterminate_not_running() {
    let h = harness();
    h.connector.script(|_| panic!("the session failed"));
    let snapshot = h
        .operator
        .submit(&living_room(), volume(-78))
        .await
        .unwrap();
    let done = finished(&h, snapshot.id).await;
    assert_eq!(done.status, OperationStatus::Indeterminate);
    // The write may have begun, so nothing says it did not.
    assert_eq!(done.dispatch, DispatchCertainty::Unknown);
    assert!(!done.confirmed);
}

#[tokio::test(start_paused = true)]
async fn a_rejection_from_the_session_is_reported_with_its_reason() {
    let h = harness();
    h.connector
        .script(|request| OperationOutcome::RejectedBeforeDispatch {
            operation: request.id,
            cause: RejectionCause::UnsupportedIntent,
            reason: "source is not supported by the X3800H profile".into(),
        });
    let snapshot = h
        .operator
        .submit(
            &living_room(),
            OperationSubmission::new(ReceiverIntent::Mute(MuteState::On)),
        )
        .await
        .unwrap();
    let done = finished(&h, snapshot.id).await;
    assert_eq!(done.status, OperationStatus::Rejected);
    assert_eq!(
        done.reason.as_deref(),
        Some("source is not supported by the X3800H profile")
    );
    assert_eq!(done.dispatch, DispatchCertainty::NotDispatched);
}

#[tokio::test(start_paused = true)]
async fn power_and_zone_intents_pass_through_unchanged() {
    let h = harness();
    let intent = ReceiverIntent::MainZonePower(ZonePower::On);
    let snapshot = h
        .operator
        .submit(&living_room(), OperationSubmission::new(intent.clone()))
        .await
        .unwrap();
    finished(&h, snapshot.id).await;
    assert_eq!(h.connector.session(0).calls()[0].intent, intent);
}

#[tokio::test(start_paused = true)]
async fn inspection_reads_connect_on_demand_and_pass_the_sessions_answer_through() {
    let h = harness();
    let catalog = h.operator.source_catalog(&living_room()).await.unwrap();
    assert_eq!(catalog.catalog.generation, 7);
    assert_eq!(h.connector.attempts(), 1);
    let names = h.operator.quick_select_names(&living_room()).await.unwrap();
    assert_eq!(names.generation, 7);
    // The same session served both reads.
    assert_eq!(h.connector.attempts(), 1);

    // A read the session does not provide stays a typed error.
    match h.operator.http_information(&living_room()).await {
        Err(ControlError::Receiver(error)) => {
            assert_eq!(error.kind, OperationErrorKind::Unsupported)
        }
        other => panic!("expected an unsupported read, got {other:?}"),
    }

    // A read is only activity: it does not hold the receiver connected.
    advance(IDLE + Duration::from_secs(1)).await;
    assert_eq!(h.connector.session(0).closes(), 1);
}

/// The saved configuration with the living room moved to `host`.
fn moved_to(h: &Harness, host: &str) -> ConfiguredReceivers {
    let mut configuration = locked(&h.config.0).clone();
    configuration.receivers.get_mut("living-room").unwrap().host = host.into();
    configuration
}

#[tokio::test(start_paused = true)]
async fn refresh_reads_every_field_again_and_dispatches_nothing() {
    let h = harness();
    // It connects on demand, like any read.
    let readiness = h.operator.refresh(&living_room()).await.unwrap();
    assert!(readiness.ready);
    assert_eq!(h.connector.attempts(), 1);
    let session = h.connector.session(0);
    // One pass to connect and one for the refresh.
    assert_eq!(session.synchronizes.load(Ordering::SeqCst), 2);
    assert!(session.calls().is_empty());

    h.operator.refresh(&living_room()).await.unwrap();
    assert_eq!(h.connector.attempts(), 1);
    assert_eq!(session.synchronizes.load(Ordering::SeqCst), 3);

    let bedroom = ReceiverId::new("bedroom").unwrap();
    assert_eq!(
        h.operator.refresh(&bedroom).await.unwrap_err(),
        ControlError::NotFound("receiver")
    );
}

#[tokio::test(start_paused = true)]
async fn refresh_is_the_operators_alone() {
    let h = harness();
    // An Agent is refused a handle at all until the policy path exists, so the
    // check on the method is exercised through a handle forged for the test.
    let agent = ServiceHandle {
        inner: Arc::clone(&h.service.inner),
        principal: Principal::Agent(AgentLabel::new("openclaw").unwrap()),
    };
    assert_eq!(
        agent.refresh(&living_room()).await.unwrap_err(),
        ControlError::Forbidden
    );
    assert_eq!(h.connector.attempts(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_changed_address_retires_the_session_and_the_next_lease_uses_the_new_one() {
    let h = harness();
    let subscription = h.operator.state(&living_room()).await.unwrap();
    assert_eq!(h.connector.hosts(), ["192.0.2.10"]);

    h.operator
        .save_configuration(&moved_to(&h, "192.0.2.99"))
        .await
        .unwrap();
    assert_eq!(
        h.connector.session(0).closes(),
        1,
        "closed despite the lease"
    );
    assert_eq!(
        h.operator.receivers().await.unwrap()[0].connection,
        ConnectionStatus::Released
    );

    drop(subscription);
    drop(h.operator.state(&living_room()).await.unwrap());
    assert_eq!(h.connector.hosts(), ["192.0.2.10", "192.0.2.99"]);
    assert_eq!(h.connector.session(1).closes(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_subscriber_to_a_retired_session_is_told_it_ended() {
    let h = harness();
    let mut subscription = h.operator.state(&living_room()).await.unwrap();
    h.operator
        .save_configuration(&moved_to(&h, "192.0.2.99"))
        .await
        .unwrap();

    let error = subscription.changed().await.unwrap_err();
    assert_eq!(error.kind, OperationErrorKind::Stopped);

    // Subscribing again reaches the new address.
    drop(subscription);
    let mut again = h.operator.state(&living_room()).await.unwrap();
    assert_eq!(h.connector.hosts(), ["192.0.2.10", "192.0.2.99"]);
    // And that subscription is live: it ends only when its own session does.
    let waiting = tokio::time::timeout(Duration::from_secs(1), again.changed()).await;
    assert!(waiting.is_err(), "no change and no end yet");
}

#[tokio::test(start_paused = true)]
async fn retiring_waits_for_an_operation_in_flight_and_loses_nothing() {
    let h = harness();
    let release = h.connector.hold_operate();
    let snapshot = h
        .operator
        .submit(&living_room(), volume(-78))
        .await
        .unwrap();
    h.connector.entered.notified().await;

    let operator = Arc::clone(&h.operator);
    let moved = moved_to(&h, "192.0.2.99");
    let save = tokio::spawn(async move { operator.save_configuration(&moved).await });
    settle().await;
    assert!(!save.is_finished(), "the retirement must wait");
    assert_eq!(h.connector.session(0).closes(), 0);

    release.notify_one();
    save.await.unwrap().unwrap();
    let done = finished(&h, snapshot.id).await;
    assert_eq!(done.status, OperationStatus::Completed);
    assert!(done.confirmed);
    assert_eq!(h.connector.session(0).closes(), 1);
    assert_eq!(h.connector.session(0).calls().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_new_connection_never_opens_beside_the_old_one() {
    let h = harness();
    let close = h.connector.hold_close();
    drop(h.operator.state(&living_room()).await.unwrap());

    let operator = Arc::clone(&h.operator);
    let moved = moved_to(&h, "192.0.2.99");
    let save = tokio::spawn(async move { operator.save_configuration(&moved).await });
    settle().await;
    assert!(!save.is_finished(), "the old session is still closing");

    let operator = Arc::clone(&h.operator);
    let request = tokio::spawn(async move { operator.state(&living_room()).await.map(drop) });
    settle().await;
    assert!(!request.is_finished(), "it must wait for the close");
    assert_eq!(h.connector.attempts(), 1, "no second connection yet");

    close.notify_one();
    save.await.unwrap().unwrap();
    request.await.unwrap().unwrap();
    assert_eq!(h.connector.session(0).closes(), 1);
    assert_eq!(h.connector.hosts(), ["192.0.2.10", "192.0.2.99"]);
}

#[tokio::test(start_paused = true)]
async fn a_save_that_keeps_the_address_retires_nothing() {
    let h = harness();
    let subscription = h.operator.state(&living_room()).await.unwrap();
    let mut configuration = locked(&h.config.0).clone();
    let entry = configuration.receivers.get_mut("living-room").unwrap();
    entry.friendly_name = Some("Living Room".into());
    entry.model = Some("AVR-X3800H".into());
    h.operator.save_configuration(&configuration).await.unwrap();
    // A second receiver being added is no reason to touch the first.
    configuration
        .receivers
        .insert("den".into(), ReceiverIdentity::ad_hoc("192.0.2.11"));
    h.operator.save_configuration(&configuration).await.unwrap();

    assert_eq!(h.connector.session(0).closes(), 0);
    assert_eq!(h.connector.attempts(), 1);
    drop(subscription);
}

#[tokio::test(start_paused = true)]
async fn removing_an_entry_retires_its_session() {
    let h = harness();
    let mut subscription = h.operator.state(&living_room()).await.unwrap();
    let mut configuration = locked(&h.config.0).clone();
    configuration.receivers.clear();
    configuration.current = None;
    h.operator.save_configuration(&configuration).await.unwrap();

    assert_eq!(h.connector.session(0).closes(), 1);
    assert!(subscription.changed().await.is_err());
    assert!(matches!(
        h.operator.state(&living_room()).await,
        Err(ControlError::NotFound("receiver"))
    ));
}

#[tokio::test(start_paused = true)]
async fn shutdown_ends_the_subscriptions_it_closes() {
    let h = harness();
    let mut subscription = h.operator.state(&living_room()).await.unwrap();
    h.service.shutdown().await;
    assert_eq!(h.connector.session(0).closes(), 1);
    assert!(subscription.changed().await.is_err());
}
