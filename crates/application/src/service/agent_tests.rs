//! Tests of the Agent path through the gate: evaluation, refusal, the caps, and
//! the policy and ledger it depends on. They run on a paused clock with the
//! fakes of `tests.rs`, a fake audit log, a fake policy source, and a clock the
//! test sets.

use super::tests::{
    advance, living_room, settle, Effect as SessionEffect, FakeConfig, FakeConnector,
    FakeDiscovery, OrderLog,
};
use super::*;
use crate::audit::{AuditEntry, AuditError, AuditLog, AuditPage, AuditQuery, Durability};
use crate::clock::Clock;
use crate::control::{AgentLabel, ApprovalHealth, AuditHealth, DryRunDecision, PolicyHealth};
use crate::policy_source::{LoadedPolicy, PolicyDigest, PolicyLoadError, PolicySource};
use crate::{AuditDecision, AuditEvent, AuditRecord};
use denon_avr_domain::{
    CoreField, Epoch, FrameSeq, MasterVolume, MonotonicMillis, MuteState, ObservationOrigin,
    OperationOutcome, ReceiverObservation, ReceiverState, RejectionCause, WallTime,
};
use denon_avr_policy::{
    Effect as Verdict, IntentKind, IntentValue, Level, PolicyConfig, Rule, Span,
};
use std::collections::BTreeMap;

// ---- Fakes ----

struct FakeClock(Mutex<WallTime>);

impl FakeClock {
    fn at(millis: u64) -> Arc<Self> {
        Arc::new(Self(Mutex::new(WallTime(millis))))
    }

    fn advance(&self, by: Duration) {
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
enum AuditMode {
    Working,
    FailAppend,
    HangAppend,
    FailRead,
    /// Neither written nor read.
    FailAll,
    /// Only the `Dispatching` record cannot be written.
    FailDispatching,
}

struct FakeAudit {
    records: Mutex<Vec<AuditRecord>>,
    mode: Mutex<AuditMode>,
    order: Mutex<Option<OrderLog>>,
    /// When set, an append waits for a permit before it does anything.
    gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
    /// How durably each record was asked to be written, in order.
    durability: Mutex<Vec<(&'static str, Durability)>>,
}

impl FakeAudit {
    fn new(mode: AuditMode) -> Arc<Self> {
        Arc::new(Self {
            records: Mutex::new(Vec::new()),
            mode: Mutex::new(mode),
            order: Mutex::new(None),
            gate: Mutex::new(None),
            durability: Mutex::new(Vec::new()),
        })
    }

    /// Appends wait until the returned semaphore is given permits.
    fn hold_appends(&self) -> Arc<tokio::sync::Semaphore> {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *locked(&self.gate) = Some(Arc::clone(&gate));
        gate
    }

    fn set_mode(&self, mode: AuditMode) {
        *locked(&self.mode) = mode;
    }

    fn share_order(&self, order: OrderLog) {
        *locked(&self.order) = Some(order);
    }

    fn records(&self) -> Vec<AuditRecord> {
        locked(&self.records).clone()
    }

    fn events(&self) -> Vec<AuditEvent> {
        self.records()
            .into_iter()
            .map(|record| record.event)
            .collect()
    }
}

fn event_name(event: &AuditEvent) -> &'static str {
    match event {
        AuditEvent::Decided { .. } => "audit:decided",
        AuditEvent::Dispatching { .. } => "audit:dispatching",
        AuditEvent::Finished { .. } => "audit:finished",
        AuditEvent::PolicyLoaded { .. } => "audit:policy_loaded",
        AuditEvent::PolicyLoadFailed { .. } => "audit:policy_load_failed",
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

struct FakePolicy {
    result: Mutex<Result<LoadedPolicy, PolicyLoadError>>,
    loads: AtomicUsize,
}

impl FakePolicy {
    fn new(result: Result<LoadedPolicy, PolicyLoadError>) -> Arc<Self> {
        Arc::new(Self {
            result: Mutex::new(result),
            loads: AtomicUsize::new(0),
        })
    }

    fn set(&self, result: Result<LoadedPolicy, PolicyLoadError>) {
        *locked(&self.result) = result;
    }
}

impl PolicySource for FakePolicy {
    fn load(&self) -> BoxFuture<'_, Result<LoadedPolicy, PolicyLoadError>> {
        Box::pin(async move {
            self.loads.fetch_add(1, Ordering::SeqCst);
            locked(&self.result).clone()
        })
    }
}

// ---- The owner's policy and a receiver state ----

fn level(db: f64) -> Level {
    Level::from_db(db).unwrap()
}

fn span(db: f64) -> Span {
    Span::from_db(db).unwrap()
}

fn owners_rules() -> Vec<Rule> {
    vec![
        Rule::new("volume-hard-limit", Verdict::Deny)
            .intent(IntentKind::Volume)
            .target_above(level(-20.0)),
        Rule::new("volume-ceiling", Verdict::RequireApproval)
            .intent(IntentKind::Volume)
            .target_above(level(-30.0)),
        Rule::new("volume-step", Verdict::RequireApproval)
            .intent(IntentKind::Volume)
            .increase_over_baseline(span(6.0)),
        Rule::new("volume-budget", Verdict::RequireApproval)
            .intent(IntentKind::Volume)
            .budget(span(10.0), 10),
        Rule::new("loud-or-unknown-baseline", Verdict::RequireApproval)
            .intent_value(IntentKind::MainZonePower, IntentValue::On)
            .intent_value(IntentKind::Mute, IntentValue::Off)
            .baseline_volume_above(level(-30.0)),
        Rule::new("system-power", Verdict::RequireApproval).intent(IntentKind::SystemPower),
        Rule::new("zone2-power", Verdict::RequireApproval).intent(IntentKind::Zone2Power),
        Rule::new("low-risk", Verdict::Allow)
            .intent_value(IntentKind::MainZonePower, IntentValue::Off)
            .intent_value(IntentKind::Mute, IntentValue::On)
            .intent(IntentKind::Source)
            .intent(IntentKind::SoundMode),
    ]
}

fn policy_of(rules: Vec<Rule>, digest: u8) -> LoadedPolicy {
    LoadedPolicy {
        config: PolicyConfig::new(rules, Duration::from_secs(300)).unwrap(),
        digest: PolicyDigest::from_bytes([digest; 32]),
        text: format!("# policy {digest}\n"),
    }
}

fn owners_policy() -> LoadedPolicy {
    policy_of(owners_rules(), 1)
}

fn observation<T>(value: T) -> ReceiverObservation<T> {
    ReceiverObservation {
        receiver: living_room(),
        epoch: Epoch(1),
        frame_seq: FrameSeq(1),
        observed_at: MonotonicMillis(0),
        origin: ObservationOrigin::ReceiverFrame,
        value,
    }
}

fn db_half_steps(db: f64) -> i16 {
    (db * 2.0).round() as i16
}

fn master(db: f64) -> MasterVolume {
    MasterVolume::db_half_steps(db_half_steps(db)).unwrap()
}

/// A receiver with an established connection, a usable volume, and mute on.
fn receiver_at(db: f64) -> ReceiverState {
    let mut state = ReceiverState::new(living_room());
    state.establish_epoch(Epoch(1));
    state
        .main_zone
        .volume
        .observe(observation(master(db)), MonotonicMillis(1_000));
    state
        .main_zone
        .mute
        .observe(observation(MuteState::On), MonotonicMillis(1_000));
    state
}

/// A receiver that refuses a write whose precondition no longer holds, and
/// otherwise applies a volume change to its state as the real one would.
fn receiver_effect() -> SessionEffect {
    Arc::new(|request, states| {
        if let Some(precondition) = &request.precondition {
            if let Some(mismatch) = precondition.mismatch(&states.borrow()) {
                return Some(OperationOutcome::RejectedBeforeDispatch {
                    operation: request.id,
                    cause: RejectionCause::PreconditionMismatch(mismatch),
                    reason: "the receiver changed since the decision".into(),
                });
            }
        }
        if let ReceiverIntent::Volume(volume) = &request.intent {
            states.send_modify(|state| {
                state
                    .main_zone
                    .volume
                    .observe(observation(*volume), MonotonicMillis(2_000));
            });
        }
        None
    })
}

/// The receiver's volume changes by itself, as if someone turned the dial.
fn turn_dial(h: &AgentHarness, db: f64) {
    h.connector.session(0).states.send_modify(|state| {
        state
            .main_zone
            .volume
            .observe(observation(master(db)), MonotonicMillis(3_000));
    });
}

fn mute_on() -> OperationSubmission {
    OperationSubmission::new(ReceiverIntent::Mute(MuteState::On))
}

fn set_volume(db: f64) -> OperationSubmission {
    OperationSubmission::new(ReceiverIntent::Volume(master(db)))
}

// ---- The harness ----

struct Setup {
    policy: Result<LoadedPolicy, PolicyLoadError>,
    audit: AuditMode,
    limits: AgentLimits,
    volume_db: f64,
    /// A receiver state to use instead of one at `volume_db`.
    initial: Option<ReceiverState>,
    /// Records already in the audit log when the service starts.
    history: Option<Vec<AuditRecord>>,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            policy: Ok(owners_policy()),
            audit: AuditMode::Working,
            limits: AgentLimits::default(),
            volume_db: -35.0,
            initial: None,
            history: None,
        }
    }
}

struct AgentHarness {
    order: OrderLog,
    service: Arc<ControlService>,
    operator: SharedOperatorControl,
    agent: Arc<ServiceHandle>,
    connector: Arc<FakeConnector>,
    audit: Arc<FakeAudit>,
    policy: Arc<FakePolicy>,
    clock: Arc<FakeClock>,
}

fn label(name: &str) -> AgentLabel {
    AgentLabel::new(name).unwrap()
}

async fn start(setup: Setup) -> AgentHarness {
    let connector = Arc::new(FakeConnector::new());
    connector.start_in(
        setup
            .initial
            .unwrap_or_else(|| receiver_at(setup.volume_db)),
    );
    connector.effect(receiver_effect());
    let order: OrderLog = Arc::default();
    connector.order(Arc::clone(&order));
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
    let audit = FakeAudit::new(setup.audit);
    audit.share_order(Arc::clone(&order));
    if let Some(records) = setup.history {
        locked(&audit.records).extend(records);
    }
    let policy = FakePolicy::new(setup.policy);
    let clock = FakeClock::at(10_000_000);
    let service = Arc::new(
        ControlService::start(
            connector.clone(),
            config,
            Arc::new(FakeDiscovery(Vec::new())),
            ServiceConfig::default(),
            AgentPath {
                policy: policy.clone(),
                audit: audit.clone(),
                clock: clock.clone(),
                limits: setup.limits,
            },
        )
        .await,
    );
    AgentHarness {
        order,
        operator: service.operator(),
        agent: service.handle(Principal::Agent(label("openclaw"))).unwrap(),
        service,
        connector,
        audit,
        policy,
        clock,
    }
}

async fn start_default() -> AgentHarness {
    start(Setup::default()).await
}

impl AgentHarness {
    fn other(&self, name: &str) -> Arc<ServiceHandle> {
        self.service.handle(Principal::Agent(label(name))).unwrap()
    }
}

/// Submit as `handle` and wait for the operation to end.
async fn ask(handle: &ServiceHandle, submission: OperationSubmission) -> OperationSnapshot {
    let first = handle.submit(&living_room(), submission).await.unwrap();
    let snapshot = handle
        .operation(first.id, Some(Duration::from_secs(5)))
        .await
        .unwrap();
    assert!(snapshot.status.is_terminal(), "{snapshot:?}");
    snapshot
}

fn operate_calls(h: &AgentHarness) -> usize {
    (0..h.connector.sessions())
        .map(|index| h.connector.session(index).calls().len())
        .sum()
}

// ---- Tests ----

#[tokio::test(start_paused = true)]
async fn an_agent_handle_needs_a_service_built_by_start() {
    let h = start_default().await;
    assert!(h.service.handle(Principal::Agent(label("a"))).is_ok());
    assert!(h.service.handle(Principal::Operator).is_ok());

    let plain = super::tests::harness();
    assert!(matches!(
        plain.service.handle(Principal::Agent(label("a"))),
        Err(ControlError::Forbidden)
    ));
}

#[tokio::test(start_paused = true)]
async fn a_deny_ends_denied_and_a_require_approval_ends_approval_unavailable() {
    let h = start_default().await;

    let denied = ask(&h.agent, set_volume(-15.0)).await;
    assert_eq!(denied.status, OperationStatus::Denied);
    assert_eq!(denied.dispatch, DispatchCertainty::NotDispatched);
    assert!(!denied.confirmed);

    let held = ask(&h.agent, set_volume(-25.0)).await;
    assert_eq!(held.status, OperationStatus::ApprovalUnavailable);
    assert_eq!(held.dispatch, DispatchCertainty::NotDispatched);

    assert_eq!(operate_calls(&h), 0, "nothing reached the receiver");

    // The reason names the limit that fired, and no rule id: ids are for the
    // audit log and the Operator.
    let reason = denied.reason.unwrap();
    assert!(reason.contains("above the limit"), "{reason}");
    let reason = held.reason.unwrap();
    assert!(reason.contains("above the limit"), "{reason}");
    for rule in [
        "volume-hard-limit",
        "volume-ceiling",
        "volume-step",
        "volume-budget",
    ] {
        assert!(!reason.contains(rule), "{reason}");
    }

    // Each refusal is decided and finished in the log, with the rules that fired.
    let events = h.audit.events();
    let decided: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            AuditEvent::Decided {
                decision,
                rules,
                baseline,
                policy,
                ..
            } => Some((*decision, rules.clone(), baseline.clone(), *policy)),
            _ => None,
        })
        .collect();
    assert_eq!(decided.len(), 2);
    assert_eq!(decided[0].0, AuditDecision::Deny);
    assert_eq!(decided[0].1[0], "volume-hard-limit");
    assert_eq!(decided[1].0, AuditDecision::RequireApproval);
    assert_eq!(decided[1].1, vec!["volume-ceiling", "volume-step"]);
    assert_eq!(decided[1].3, Some(PolicyDigest::from_bytes([1; 32])));
    let finished = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                AuditEvent::Finished {
                    dispatch: DispatchCertainty::NotDispatched,
                    ..
                }
            )
        })
        .count();
    assert_eq!(finished, 2);
}

#[tokio::test(start_paused = true)]
async fn an_agents_request_starts_submitted_and_an_operators_starts_allowed() {
    let h = start_default().await;
    let release = h.connector.hold_connect();
    let agent = h
        .agent
        .submit(&living_room(), set_volume(-50.0))
        .await
        .unwrap();
    assert_eq!(agent.status, OperationStatus::Submitted);
    let operator = h
        .operator
        .submit(&living_room(), set_volume(-51.0))
        .await
        .unwrap();
    assert_eq!(operator.status, OperationStatus::Allowed);
    release.notify_one();
    settle().await;
}

#[tokio::test(start_paused = true)]
async fn a_decision_records_what_it_read() {
    let h = start_default().await;
    ask(&h.agent, set_volume(-25.0)).await;
    ask(&h.agent, set_volume(-15.0)).await;
    let decided: Vec<_> = h
        .audit
        .events()
        .into_iter()
        .filter_map(|event| match event {
            AuditEvent::Decided {
                intent, baseline, ..
            } => Some((intent, baseline)),
            _ => None,
        })
        .collect();
    assert_eq!(decided[0].0, "volume -25.0 dB");
    assert_eq!(decided[0].1, vec![denon_avr_domain::CoreField::Volume]);
    // A deny is final and read nothing it needs to keep.
    assert_eq!(decided[1].0, "volume -15.0 dB");
    assert!(decided[1].1.is_empty());
}

#[tokio::test(start_paused = true)]
async fn an_unreachable_receiver_is_reported_to_an_agent_without_detail() {
    let h = start_default().await;
    h.connector.fail_connections();
    let agent = ask(&h.agent, set_volume(-50.0)).await;
    assert_eq!(agent.status, OperationStatus::Rejected);
    assert_eq!(agent.dispatch, DispatchCertainty::NotDispatched);
    assert_eq!(
        agent.reason.as_deref(),
        Some("the receiver could not be reached")
    );
    // The Operator still gets the cause.
    let operator = ask_as_operator(&h, set_volume(-50.0)).await;
    assert_eq!(operator.status, OperationStatus::Rejected);
    assert!(operator.reason.unwrap().contains("refused"));
}

#[tokio::test(start_paused = true)]
async fn an_unavailable_policy_rejects_every_agent_write_and_leaves_reads_and_operator() {
    let h = start(Setup {
        policy: Err(PolicyLoadError::Invalid(
            "rule \"secret-rule-name\": nonsense".into(),
        )),
        ..Setup::default()
    })
    .await;

    let health = h.agent.health().await.unwrap();
    assert_eq!(health.policy, PolicyHealth::Unavailable);
    assert_eq!(health.audit, AuditHealth::Ok);
    assert!(health.ledger_ready);
    assert_eq!(health.approval, ApprovalHealth::Unavailable);

    // Every agent write is rejected, whatever it asks, with fixed text.
    for submission in [
        set_volume(-50.0),
        OperationSubmission::new(ReceiverIntent::Mute(MuteState::On)),
    ] {
        let snapshot = ask(&h.agent, submission).await;
        assert_eq!(snapshot.status, OperationStatus::Rejected);
        assert_eq!(snapshot.dispatch, DispatchCertainty::NotDispatched);
        assert_eq!(snapshot.reason.as_deref(), Some("policy unavailable"));
    }
    assert_eq!(
        h.connector.attempts(),
        0,
        "the receiver was not even opened"
    );
    assert_eq!(operate_calls(&h), 0);

    // A dry run says so too.
    let dry = h
        .agent
        .dry_run(&living_room(), set_volume(-50.0).intent)
        .await
        .unwrap();
    assert_eq!(
        dry.decision,
        DryRunDecision::Unavailable {
            reason: "policy unavailable".into()
        }
    );
    assert_eq!(dry.policy, None);

    // Reads are unaffected, and so are the Operator's controls.
    assert_eq!(h.agent.receivers().await.unwrap().len(), 1);
    drop(h.agent.state(&living_room()).await.unwrap());
    let operator = ask_as_operator(&h, set_volume(-50.0)).await;
    assert_eq!(operator.status, OperationStatus::Completed);
    assert_eq!(operate_calls(&h), 1);

    // The Operator sees why, and the failure is in the log.
    let view = h.operator.policy().await.unwrap();
    assert_eq!(view.digest, None);
    assert!(view.error.unwrap().contains("secret-rule-name"));
    assert!(matches!(
        h.audit.events().first(),
        Some(AuditEvent::PolicyLoadFailed { .. })
    ));
}

async fn ask_as_operator(h: &AgentHarness, submission: OperationSubmission) -> OperationSnapshot {
    let first = h.operator.submit(&living_room(), submission).await.unwrap();
    h.operator
        .operation(first.id, Some(Duration::from_secs(5)))
        .await
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn a_failed_reload_disables_agent_writes_until_a_good_one() {
    let h = start_default().await;
    assert_eq!(
        ask(&h.agent, set_volume(-25.0)).await.status,
        OperationStatus::ApprovalUnavailable
    );
    assert_eq!(h.agent.health().await.unwrap().policy, PolicyHealth::Active);

    // A bad file replaces the good policy, it does not leave it in force.
    h.policy.set(Err(PolicyLoadError::Invalid("bad".into())));
    let view = h.operator.reload_policy().await.unwrap();
    assert_eq!(view.digest, None);
    assert_eq!(view.text, None);
    assert!(view.error.unwrap().contains("invalid"));
    assert_eq!(
        h.agent.health().await.unwrap().policy,
        PolicyHealth::Unavailable
    );
    let rejected = ask(&h.agent, set_volume(-25.0)).await;
    assert_eq!(rejected.status, OperationStatus::Rejected);
    assert_eq!(rejected.reason.as_deref(), Some("policy unavailable"));

    // A good one restores it.
    h.policy.set(Ok(policy_of(owners_rules(), 2)));
    let view = h.operator.reload_policy().await.unwrap();
    assert_eq!(view.digest, Some(PolicyDigest::from_bytes([2; 32])));
    assert_eq!(view.text.as_deref(), Some("# policy 2\n"));
    assert_eq!(view.error, None);
    assert!(view.loaded_at.is_some());
    assert_eq!(h.operator.policy().await.unwrap(), view);
    assert_eq!(
        ask(&h.agent, set_volume(-25.0)).await.status,
        OperationStatus::ApprovalUnavailable
    );

    // Each load is in the log, in order.
    let loads: Vec<&'static str> = h
        .audit
        .events()
        .iter()
        .filter(|event| {
            matches!(
                event,
                AuditEvent::PolicyLoaded { .. } | AuditEvent::PolicyLoadFailed { .. }
            )
        })
        .map(event_name)
        .collect();
    assert_eq!(
        loads,
        [
            "audit:policy_loaded",
            "audit:policy_load_failed",
            "audit:policy_loaded"
        ]
    );

    // Only the Operator reloads.
    assert_eq!(
        h.agent_forged().reload_policy().await.unwrap_err(),
        ControlError::Forbidden
    );
}

impl AgentHarness {
    /// A handle for an agent, as `refresh_is_the_operators_alone` forges one.
    fn agent_forged(&self) -> ServiceHandle {
        ServiceHandle {
            inner: Arc::clone(&self.service.inner),
            principal: Principal::Agent(label("openclaw")),
        }
    }
}

#[tokio::test(start_paused = true)]
async fn an_unreadable_audit_log_keeps_the_ledger_unready_and_writes_rejected() {
    let h = start(Setup {
        audit: AuditMode::FailRead,
        ..Setup::default()
    })
    .await;
    let health = h.agent.health().await.unwrap();
    assert!(!health.ledger_ready);
    assert_eq!(health.policy, PolicyHealth::Active);

    let snapshot = ask(&h.agent, set_volume(-25.0)).await;
    assert_eq!(snapshot.status, OperationStatus::Rejected);
    assert_eq!(snapshot.dispatch, DispatchCertainty::NotDispatched);
    let reason = snapshot.reason.unwrap();
    assert!(reason.contains("budget history"), "{reason}");
    assert!(!reason.contains("/private"), "{reason}");
    assert_eq!(
        h.connector.attempts(),
        0,
        "refused before the receiver was opened"
    );

    // The Operator is not held up by it.
    assert_eq!(
        ask_as_operator(&h, set_volume(-50.0)).await.status,
        OperationStatus::Completed
    );

    // Once the log can be read, the next agent write rebuilds the ledger first.
    h.audit.set_mode(AuditMode::Working);
    let snapshot = ask(&h.agent, set_volume(-25.0)).await;
    assert_eq!(snapshot.status, OperationStatus::ApprovalUnavailable);
    assert!(h.agent.health().await.unwrap().ledger_ready);
}

#[tokio::test(start_paused = true)]
async fn a_dry_run_returns_the_decision_and_creates_no_operation_or_record() {
    let h = start_default().await;
    let records_before = h.audit.records().len();

    let deny = h
        .agent
        .dry_run(&living_room(), set_volume(-15.0).intent)
        .await
        .unwrap();
    let DryRunDecision::Deny { reasons, rules } = &deny.decision else {
        panic!("{deny:?}");
    };
    assert!(reasons[0].contains("above the limit"));
    assert_eq!(rules[0], "volume-hard-limit");
    assert_eq!(deny.policy, Some(PolicyDigest::from_bytes([1; 32])));

    let held = h
        .agent
        .dry_run(&living_room(), set_volume(-25.0).intent)
        .await
        .unwrap();
    assert!(matches!(
        held.decision,
        DryRunDecision::RequireApproval { .. }
    ));

    let allowed = h
        .agent
        .dry_run(&living_room(), set_volume(-32.0).intent)
        .await
        .unwrap();
    assert_eq!(allowed.decision, DryRunDecision::Allow);

    // Nothing was made or written.
    assert_eq!(h.audit.records().len(), records_before);
    assert_eq!(
        h.operator
            .operation(OperationId(1), None)
            .await
            .unwrap_err(),
        ControlError::NotFound("operation")
    );
    assert_eq!(operate_calls(&h), 0);

    // The Operator's own request skips policy.
    let operator = h
        .operator
        .dry_run(&living_room(), set_volume(-15.0).intent)
        .await
        .unwrap();
    assert_eq!(operator.decision, DryRunDecision::Allow);
}

#[tokio::test(start_paused = true)]
async fn dry_run_as_applies_the_named_agents_rules() {
    let mut rules = owners_rules();
    rules.push(Rule::new("claude-code-read-only", Verdict::Deny).agents(&["claude-code"]));
    let h = start(Setup {
        policy: Ok(policy_of(rules, 3)),
        ..Setup::default()
    })
    .await;
    let quiet = set_volume(-50.0).intent;

    let tier = h
        .operator
        .dry_run_as(label("claude-code"), &living_room(), quiet.clone())
        .await
        .unwrap();
    let DryRunDecision::Deny { rules, .. } = tier.decision else {
        panic!("claude-code is read-only");
    };
    assert_eq!(rules, vec!["claude-code-read-only"]);

    let other = h
        .operator
        .dry_run_as(label("openclaw"), &living_room(), quiet.clone())
        .await
        .unwrap();
    assert_eq!(other.decision, DryRunDecision::Allow);

    // A label that differs only in case is another agent, and is not held.
    let spelt = h
        .operator
        .dry_run_as(label("Claude-Code"), &living_room(), quiet)
        .await
        .unwrap();
    assert_eq!(spelt.decision, DryRunDecision::Allow);

    // The Operator's own test of a tier does not spend an agent's allowance,
    // however often it is made: forty is more than the cap of thirty.
    for _ in 0..40 {
        h.operator
            .dry_run_as(label("openclaw"), &living_room(), set_volume(-50.0).intent)
            .await
            .unwrap();
    }
    assert!(h
        .agent
        .dry_run(&living_room(), set_volume(-50.0).intent)
        .await
        .is_ok());
    assert_eq!(operate_calls(&h), 0);
}

#[tokio::test(start_paused = true)]
async fn the_write_cap_refuses_with_rate_limited_and_creates_nothing() {
    let h = start_default().await;
    // Thirty requests, each finished before the next, are thirty writes. Muting
    // is allowed and leaves the volume budget alone.
    for _ in 0..30 {
        let snapshot = ask(&h.agent, mute_on()).await;
        assert_eq!(snapshot.status, OperationStatus::Completed);
    }
    let records = h.audit.records().len();

    let refused = h.agent.submit(&living_room(), mute_on()).await.unwrap_err();
    let ControlError::RateLimited {
        retry_after: Some(retry_after),
    } = refused
    else {
        panic!("{refused:?}");
    };
    assert!(
        retry_after > Duration::ZERO && retry_after <= Duration::from_secs(60),
        "{retry_after:?}"
    );
    // It made no operation and wrote no record.
    assert_eq!(h.audit.records().len(), records);
    assert_eq!(
        h.operator
            .operation(OperationId(31), None)
            .await
            .unwrap_err(),
        ControlError::NotFound("operation")
    );

    // Another label has its own allowance.
    let other = h.other("second");
    assert_eq!(
        ask(&other, mute_on()).await.status,
        OperationStatus::Completed
    );

    // After a minute the oldest write has left the window.
    advance(Duration::from_secs(61)).await;
    assert!(h.agent.submit(&living_room(), mute_on()).await.is_ok());
}

#[tokio::test(start_paused = true)]
async fn the_unfinished_operation_cap_refuses_with_rate_limited_and_creates_nothing() {
    let h = start(Setup {
        limits: AgentLimits {
            unfinished_operations: 2,
            ..AgentLimits::default()
        },
        ..Setup::default()
    })
    .await;
    // Operations that cannot start stay unfinished.
    let release = h.connector.hold_connect();
    let first = h
        .agent
        .submit(&living_room(), set_volume(-50.0))
        .await
        .unwrap();
    let second = h
        .agent
        .submit(&living_room(), set_volume(-49.0))
        .await
        .unwrap();
    settle().await;

    let refused = h
        .agent
        .submit(&living_room(), set_volume(-48.0))
        .await
        .unwrap_err();
    assert_eq!(refused, ControlError::RateLimited { retry_after: None });
    assert_eq!(
        h.operator
            .operation(OperationId(3), None)
            .await
            .unwrap_err(),
        ControlError::NotFound("operation")
    );

    // Asking again for what is already running is the same request, not a new one.
    let again = h
        .agent
        .submit(&living_room(), set_volume(-50.0))
        .await
        .unwrap();
    assert_eq!(again.id, first.id);

    // Another label is not counted against it.
    let other = h.other("second");
    assert!(other
        .submit(&living_room(), set_volume(-47.0))
        .await
        .is_ok());

    // Finishing frees the room.
    release.notify_waiters();
    release.notify_one();
    settle().await;
    for id in [first.id, second.id] {
        let done = h
            .agent
            .operation(id, Some(Duration::from_secs(5)))
            .await
            .unwrap();
        assert!(done.status.is_terminal(), "{done:?}");
    }
    assert!(h
        .agent
        .submit(&living_room(), set_volume(-48.0))
        .await
        .is_ok());
}

#[tokio::test(start_paused = true)]
async fn a_dry_run_counts_against_the_write_cap() {
    let h = start(Setup {
        limits: AgentLimits {
            writes_per_minute: 2,
            ..AgentLimits::default()
        },
        ..Setup::default()
    })
    .await;
    for _ in 0..2 {
        h.agent
            .dry_run(&living_room(), set_volume(-50.0).intent)
            .await
            .unwrap();
    }
    assert!(matches!(
        h.agent
            .dry_run(&living_room(), set_volume(-50.0).intent)
            .await
            .unwrap_err(),
        ControlError::RateLimited { .. }
    ));
    assert!(matches!(
        h.agent
            .submit(&living_room(), set_volume(-50.0))
            .await
            .unwrap_err(),
        ControlError::RateLimited { .. }
    ));
    advance(Duration::from_secs(61)).await;
    assert!(h
        .agent
        .dry_run(&living_room(), set_volume(-50.0).intent)
        .await
        .is_ok());
}

#[tokio::test(start_paused = true)]
async fn an_idempotent_retry_is_not_a_new_write() {
    let h = start(Setup {
        limits: AgentLimits {
            writes_per_minute: 1,
            ..AgentLimits::default()
        },
        ..Setup::default()
    })
    .await;
    let key = IdempotencyKey::new("retry-1").unwrap();
    let submission = set_volume(-50.0).with_idempotency_key(key);

    let first = h
        .agent
        .submit(&living_room(), submission.clone())
        .await
        .unwrap();
    // The allowance is spent, yet the retry returns the operation it names.
    let again = h
        .agent
        .submit(&living_room(), submission.clone())
        .await
        .unwrap();
    assert_eq!(again.id, first.id);
    h.agent
        .operation(first.id, Some(Duration::from_secs(5)))
        .await
        .unwrap();
    let after = h.agent.submit(&living_room(), submission).await.unwrap();
    assert_eq!(after.id, first.id, "still the same operation once it ended");

    // A new request is a new write.
    assert!(matches!(
        h.agent
            .submit(&living_room(), set_volume(-49.0))
            .await
            .unwrap_err(),
        ControlError::RateLimited { .. }
    ));
}

#[tokio::test(start_paused = true)]
async fn a_service_built_by_new_audits_nothing_and_serves_no_agent() {
    let plain = super::tests::harness();
    let health = plain.operator.health().await.unwrap();
    assert_eq!(health.policy, PolicyHealth::NotConfigured);
    assert_eq!(health.audit, AuditHealth::NotConfigured);
    assert!(!health.ledger_ready);
    assert_eq!(health.approval, ApprovalHealth::Unavailable);

    // The Operator's controls work as before, and their dry run skips policy.
    let snapshot = plain
        .operator
        .submit(&living_room(), set_volume(-50.0))
        .await
        .unwrap();
    let done = plain
        .operator
        .operation(snapshot.id, Some(Duration::from_secs(5)))
        .await
        .unwrap();
    assert_eq!(done.status, OperationStatus::Completed);
    let dry = plain
        .operator
        .dry_run(&living_room(), set_volume(-15.0).intent)
        .await
        .unwrap();
    assert_eq!(dry.decision, DryRunDecision::Allow);
    assert_eq!(dry.policy, None);

    // There is no policy or audit log to show, and no way to give an agent a handle.
    assert!(matches!(
        plain.operator.policy().await,
        Err(ControlError::Unavailable(_))
    ));
    assert!(matches!(
        plain.operator.audit(AuditQuery::new(10)).await,
        Err(ControlError::Unavailable(_))
    ));
    assert!(matches!(
        plain
            .operator
            .dry_run_as(label("a"), &living_room(), set_volume(-50.0).intent)
            .await,
        Err(ControlError::Unavailable(_))
    ));
}

#[tokio::test(start_paused = true)]
async fn the_operator_reads_the_audit_log_through_the_port() {
    let h = start_default().await;
    ask(&h.agent, set_volume(-15.0)).await;
    let page = h.operator.audit(AuditQuery::new(2)).await.unwrap();
    assert_eq!(page.entries.len(), 2);
    assert!(matches!(
        page.entries[0].record.event,
        AuditEvent::Finished { .. }
    ));
    assert_eq!(
        h.agent_forged()
            .audit(AuditQuery::new(2))
            .await
            .unwrap_err(),
        ControlError::Forbidden
    );
}

#[tokio::test(start_paused = true)]
async fn an_audit_log_that_cannot_be_read_is_reported_without_its_text() {
    let h = start(Setup {
        audit: AuditMode::FailRead,
        ..Setup::default()
    })
    .await;
    let error = h.operator.audit(AuditQuery::new(5)).await.unwrap_err();
    assert_eq!(
        error,
        ControlError::Unavailable("the audit log cannot be read".into())
    );
    assert!(!error.to_string().contains("/private"));
}

#[tokio::test(start_paused = true)]
async fn a_healthy_start_reports_every_part_working() {
    let h = start_default().await;
    let health = h.operator.health().await.unwrap();
    assert_eq!(
        health,
        ServiceHealth {
            policy: PolicyHealth::Active,
            audit: AuditHealth::Ok,
            ledger_ready: true,
            approval: ApprovalHealth::Unavailable,
        }
    );
    // Start recorded the policy it loaded.
    assert_eq!(
        h.audit.events(),
        vec![AuditEvent::PolicyLoaded {
            digest: PolicyDigest::from_bytes([1; 32])
        }]
    );
    let record = &h.audit.records()[0];
    assert_eq!(record.principal, Principal::Operator);
    assert_eq!(record.run, h.clock.now());
}

// ---- Dispatch ----

fn counted(h: &AgentHarness) -> usize {
    h.service
        .inner
        .agent
        .as_ref()
        .unwrap()
        .counted(&living_room())
}

fn order(h: &AgentHarness) -> Vec<String> {
    locked(&h.order).clone()
}

fn dispatching_record(
    run: u64,
    id: u64,
    at: WallTime,
    principal: Principal,
    before: i16,
    target: i16,
) -> AuditRecord {
    AuditRecord {
        schema: crate::audit::AUDIT_SCHEMA,
        run: WallTime(run),
        at,
        operation: Some(OperationId(id)),
        principal,
        receiver: Some(living_room()),
        event: AuditEvent::Dispatching {
            intent: "volume".into(),
            before: Some(before),
            target: Some(target),
            precondition_fields: vec![CoreField::Volume],
        },
    }
}

#[tokio::test(start_paused = true)]
async fn an_allow_dispatches_once_with_a_precondition_naming_its_baseline() {
    let h = start_default().await;
    let snapshot = ask(&h.agent, set_volume(-32.0)).await;
    assert_eq!(snapshot.status, OperationStatus::Completed);
    assert_eq!(snapshot.dispatch, DispatchCertainty::CompleteWrite);
    assert!(snapshot.confirmed);

    let calls = h.connector.session(0).calls();
    assert_eq!(calls.len(), 1, "dispatched exactly once");
    let precondition = calls[0].precondition.as_ref().expect("a precondition");
    assert_eq!(precondition.epoch(), Epoch(1));
    assert_eq!(
        precondition.fields().collect::<Vec<_>>(),
        vec![CoreField::Volume]
    );

    // Decided, then Dispatching, then the write, then Finished.
    assert_eq!(
        order(&h),
        [
            "audit:policy_loaded",
            "audit:decided",
            "audit:dispatching",
            "operate",
            "audit:finished"
        ]
    );
    let events = h.audit.events();
    assert_eq!(
        events[2],
        AuditEvent::Dispatching {
            intent: "volume -32.0 dB".into(),
            before: Some(-70),
            target: Some(-64),
            precondition_fields: vec![CoreField::Volume],
        }
    );
    assert!(matches!(
        &events[3],
        AuditEvent::Finished {
            status,
            dispatch: DispatchCertainty::CompleteWrite,
            confirmed: true,
            ..
        } if status == "completed"
    ));
    assert_eq!(counted(&h), 1, "the write counts toward the budget");
}

#[tokio::test(start_paused = true)]
async fn an_allow_that_read_nothing_still_carries_the_epoch() {
    let h = start_default().await;
    let snapshot = ask(&h.agent, mute_on()).await;
    assert_eq!(snapshot.status, OperationStatus::Completed);
    let calls = h.connector.session(0).calls();
    let precondition = calls[0].precondition.as_ref().unwrap();
    assert_eq!(precondition.epoch(), Epoch(1));
    assert_eq!(precondition.fields().count(), 0);
    assert!(matches!(
        &h.audit.events()[2],
        AuditEvent::Dispatching {
            before: None,
            target: None,
            ..
        }
    ));
    assert_eq!(counted(&h), 0, "only volume changes are counted");
}

#[tokio::test(start_paused = true)]
async fn an_unmute_precondition_carries_the_volume_though_the_loud_rule_did_not_match() {
    let h = start_default().await;
    let release = h.connector.hold_operate();
    let unmute = OperationSubmission::new(ReceiverIntent::Mute(MuteState::Off));
    let submitted = h.agent.submit(&living_room(), unmute).await.unwrap();
    settle().await;

    // The request is at the receiver. It was allowed because the volume, -35 dB,
    // is under the loud limit, so the precondition names the volume.
    assert_eq!(operate_calls(&h), 1);
    let fields: Vec<_> = h.connector.session(0).calls()[0]
        .precondition
        .as_ref()
        .unwrap()
        .fields()
        .collect();
    assert_eq!(fields, vec![CoreField::Volume]);

    // Someone turns the volume up before the write lands.
    turn_dial(&h, -25.0);
    release.notify_one();
    let done = h
        .agent
        .operation(submitted.id, Some(Duration::from_secs(5)))
        .await
        .unwrap();
    assert_eq!(done.status, OperationStatus::Rejected);
    assert_eq!(done.dispatch, DispatchCertainty::NotDispatched);
    assert_eq!(operate_calls(&h), 1, "never retried");
    assert_eq!(counted(&h), 0);
}

#[tokio::test(start_paused = true)]
async fn no_established_epoch_rejects_without_dispatch() {
    let mut state = ReceiverState::new(living_room());
    state
        .main_zone
        .volume
        .observe(observation(master(-35.0)), MonotonicMillis(1_000));
    assert_eq!(state.epoch, None);
    let h = start(Setup {
        initial: Some(state),
        ..Setup::default()
    })
    .await;

    let snapshot = ask(&h.agent, set_volume(-32.0)).await;
    assert_eq!(snapshot.status, OperationStatus::Rejected);
    assert_eq!(snapshot.dispatch, DispatchCertainty::NotDispatched);
    assert_eq!(operate_calls(&h), 0);
    assert_eq!(counted(&h), 0);
    // The policy did allow it, and the log says it was not sent.
    let events = h.audit.events();
    assert!(events.iter().any(|event| matches!(
        event,
        AuditEvent::Decided {
            decision: AuditDecision::Allow,
            ..
        }
    )));
    assert!(!events
        .iter()
        .any(|event| matches!(event, AuditEvent::Dispatching { .. })));
}

#[tokio::test(start_paused = true)]
async fn dispatching_is_appended_before_operate_and_a_failed_append_rejects() {
    let h = start_default().await;
    h.audit.set_mode(AuditMode::FailDispatching);

    let snapshot = ask(&h.agent, set_volume(-32.0)).await;
    assert_eq!(snapshot.status, OperationStatus::Rejected);
    assert_eq!(snapshot.dispatch, DispatchCertainty::NotDispatched);
    assert_eq!(snapshot.reason.as_deref(), Some("audit log unavailable"));
    assert_eq!(operate_calls(&h), 0, "nothing was sent without its record");
    assert_eq!(counted(&h), 0, "the reservation was released");
    assert!(!order(&h).contains(&"operate".to_string()));

    // The log working again is noticed by the next write.
    h.audit.set_mode(AuditMode::Working);
    assert_eq!(
        ask(&h.agent, set_volume(-32.0)).await.status,
        OperationStatus::Completed
    );
    assert_eq!(h.agent.health().await.unwrap().audit, AuditHealth::Ok);
}

#[tokio::test(start_paused = true)]
async fn an_audit_append_that_never_returns_is_a_failure_after_five_seconds() {
    let h = start_default().await;
    h.audit.set_mode(AuditMode::HangAppend);
    let submitted = h
        .agent
        .submit(&living_room(), set_volume(-32.0))
        .await
        .unwrap();
    settle().await;
    let waiting = h.agent.operation(submitted.id, None).await.unwrap();
    assert!(!waiting.status.is_terminal(), "{waiting:?}");

    // Shutdown waits for the operation, and so for the append to give up.
    let service = Arc::clone(&h.service);
    let shutdown = tokio::spawn(async move { service.shutdown().await });
    advance(Duration::from_secs(4)).await;
    assert!(!shutdown.is_finished());
    assert!(!h
        .agent
        .operation(submitted.id, None)
        .await
        .unwrap()
        .status
        .is_terminal());

    advance(Duration::from_secs(2)).await;
    let done = h.agent.operation(submitted.id, None).await.unwrap();
    assert_eq!(done.status, OperationStatus::Rejected);
    assert_eq!(done.reason.as_deref(), Some("audit log unavailable"));
    assert_eq!(operate_calls(&h), 0);
    assert_eq!(counted(&h), 0);
    shutdown.await.unwrap();
    assert_eq!(h.agent.health().await.unwrap().audit, AuditHealth::Failing);
}

#[tokio::test(start_paused = true)]
async fn an_operator_continues_when_audit_fails_and_health_says_failing() {
    let h = start_default().await;
    h.audit.set_mode(AuditMode::FailAppend);

    let operator = ask_as_operator(&h, set_volume(-50.0)).await;
    assert_eq!(operator.status, OperationStatus::Completed);
    assert_eq!(operate_calls(&h), 1);
    assert_eq!(
        h.operator.health().await.unwrap().audit,
        AuditHealth::Failing
    );

    // An agent is not served while the log cannot take its record.
    let agent = ask(&h.agent, set_volume(-60.0)).await;
    assert_eq!(agent.status, OperationStatus::Rejected);
    assert_eq!(agent.reason.as_deref(), Some("audit log unavailable"));
    assert_eq!(operate_calls(&h), 1);

    h.audit.set_mode(AuditMode::Working);
    assert_eq!(
        ask(&h.agent, set_volume(-60.0)).await.status,
        OperationStatus::Completed
    );
    assert_eq!(h.operator.health().await.unwrap().audit, AuditHealth::Ok);
}

#[tokio::test(start_paused = true)]
async fn two_agents_each_within_the_budget_cannot_together_exceed_it() {
    let h = start(Setup {
        volume_db: -50.0,
        ..Setup::default()
    })
    .await;
    // The first agent raises the volume 6 dB, inside the step and the budget,
    // and its write is at the receiver but has not finished.
    let release = h.connector.hold_operate();
    let first = h
        .agent
        .submit(&living_room(), set_volume(-44.0))
        .await
        .unwrap();
    settle().await;
    assert_eq!(operate_calls(&h), 1);
    // The receiver has applied it: the level the second agent will read is -44.
    turn_dial(&h, -44.0);

    // Alone, a rise from -44 to -38 is 6 dB and within every limit. Beside the
    // first agent's rise from -50 it is 12 dB above the lowest level, over the
    // 10 dB budget, and only the first agent's reservation says so.
    let second = h.other("second");
    let held = ask(&second, set_volume(-38.0)).await;
    assert_eq!(held.status, OperationStatus::ApprovalUnavailable);
    assert!(held.reason.unwrap().contains("above the lowest level"));
    let rules: Vec<String> = h
        .audit
        .events()
        .into_iter()
        .filter_map(|event| match event {
            AuditEvent::Decided { rules, .. } => Some(rules),
            _ => None,
        })
        .next_back()
        .unwrap();
    assert_eq!(rules, vec!["volume-budget"]);
    assert_eq!(
        operate_calls(&h),
        1,
        "the second never reached the receiver"
    );

    release.notify_one();
    h.agent
        .operation(first.id, Some(Duration::from_secs(5)))
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_cancel_before_the_session_releases_the_reservation() {
    let h = start_default().await;
    let gate = h.audit.hold_appends();
    let submitted = h
        .agent
        .submit(&living_room(), set_volume(-32.0))
        .await
        .unwrap();
    settle().await;
    // Allowed, reserved, and waiting for its record to be written.
    assert_eq!(counted(&h), 1);
    let waiting = h.agent.operation(submitted.id, None).await.unwrap();
    assert_eq!(waiting.status, OperationStatus::Allowed);

    let cancelled = h.agent.cancel(submitted.id).await.unwrap();
    assert_eq!(cancelled.status, OperationStatus::Cancelled);
    gate.add_permits(10);
    settle().await;

    let done = h.agent.operation(submitted.id, None).await.unwrap();
    assert_eq!(done.status, OperationStatus::Cancelled);
    assert_eq!(done.dispatch, DispatchCertainty::NotDispatched);
    assert_eq!(operate_calls(&h), 0);
    assert_eq!(
        counted(&h),
        0,
        "nothing is left counted for a write never made"
    );
    assert!(!h
        .audit
        .events()
        .iter()
        .any(|event| matches!(event, AuditEvent::Dispatching { .. })));
}

#[tokio::test(start_paused = true)]
async fn a_restart_rebuilds_the_ledger_from_audit() {
    // A first run raises the volume 6 dB, and its records are kept.
    let first = start(Setup {
        volume_db: -50.0,
        ..Setup::default()
    })
    .await;
    assert_eq!(
        ask(&first.agent, set_volume(-44.0)).await.status,
        OperationStatus::Completed
    );
    let kept = first.audit.records();

    // A second run starts over the same log. The receiver is at -44 dB, and a
    // rise to -38 is 12 above the -50 the first run left from.
    let second = start(Setup {
        volume_db: -44.0,
        history: Some(kept),
        ..Setup::default()
    })
    .await;
    assert!(second.agent.health().await.unwrap().ledger_ready);
    let held = ask(&second.agent, set_volume(-38.0)).await;
    assert_eq!(held.status, OperationStatus::ApprovalUnavailable);

    // A write the process died before recording the end of still counts.
    let now = WallTime(10_000_000);
    let crashed = dispatching_record(
        5,
        1,
        now.saturating_sub(Duration::from_secs(120)),
        Principal::Agent(label("openclaw")),
        -100,
        -88,
    );
    let third = start(Setup {
        volume_db: -44.0,
        history: Some(vec![crashed]),
        ..Setup::default()
    })
    .await;
    assert_eq!(
        ask(&third.agent, set_volume(-38.0)).await.status,
        OperationStatus::ApprovalUnavailable
    );

    // And so does the Operator's: the owner went from -60 to -45 two minutes ago,
    // so even half a decibel up is 15.5 dB above the lowest level of the window.
    let operator = dispatching_record(
        5,
        1,
        now.saturating_sub(Duration::from_secs(120)),
        Principal::Operator,
        -120,
        -90,
    );
    let fourth = start(Setup {
        volume_db: -45.0,
        history: Some(vec![operator]),
        ..Setup::default()
    })
    .await;
    assert_eq!(
        ask(&fourth.agent, set_volume(-44.5)).await.status,
        OperationStatus::ApprovalUnavailable
    );
    // Lowering it is never over the budget.
    assert_eq!(
        ask(&fourth.agent, set_volume(-50.0)).await.status,
        OperationStatus::Completed
    );
}

#[tokio::test(start_paused = true)]
async fn every_session_outcome_settles_the_ledger_by_dispatch() {
    use denon_avr_domain::OperationOutcome as Outcome;
    type Script = Box<dyn Fn(&crate::session_v3::OperationRequest) -> Outcome + Send + Sync>;
    let cases: Vec<(&str, Script, OperationStatus, usize)> = vec![
        (
            "observed",
            Box::new(|r| Outcome::ObservedRequestedValue {
                operation: r.id,
                dispatch: DispatchCertainty::CompleteWrite,
                observation: "seen".into(),
            }),
            OperationStatus::Completed,
            1,
        ),
        (
            "already in state",
            Box::new(|r| Outcome::AlreadyObserved {
                operation: r.id,
                observation: "seen".into(),
            }),
            OperationStatus::AlreadyInState,
            0,
        ),
        (
            "rejected before dispatch",
            Box::new(|r| Outcome::RejectedBeforeDispatch {
                operation: r.id,
                cause: RejectionCause::UnsupportedIntent,
                reason: "no".into(),
            }),
            OperationStatus::Rejected,
            0,
        ),
        (
            "cancelled",
            Box::new(|r| Outcome::Cancelled { operation: r.id }),
            OperationStatus::Cancelled,
            0,
        ),
        (
            "superseded",
            Box::new(|r| Outcome::SupersededBeforeDispatch {
                operation: r.id,
                by: OperationId(99),
            }),
            OperationStatus::Superseded {
                by: OperationId(99),
            },
            0,
        ),
        (
            "indeterminate, unknown",
            Box::new(|r| Outcome::Indeterminate {
                operation: r.id,
                dispatch: DispatchCertainty::Unknown,
                reason: "lost".into(),
            }),
            OperationStatus::Indeterminate,
            1,
        ),
        (
            "indeterminate, possibly dispatched",
            Box::new(|r| Outcome::Indeterminate {
                operation: r.id,
                dispatch: DispatchCertainty::PossiblyDispatched,
                reason: "lost".into(),
            }),
            OperationStatus::Indeterminate,
            1,
        ),
    ];
    for (name, script, status, counts) in cases {
        let h = start_default().await;
        h.connector.script(move |request| script(request));
        let snapshot = ask(&h.agent, set_volume(-32.0)).await;
        assert_eq!(snapshot.status, status, "{name}");
        assert_eq!(counted(&h), counts, "{name}");
        // What the log says it dispatched is what the client was told.
        let finished = h
            .audit
            .events()
            .into_iter()
            .find_map(|event| match event {
                AuditEvent::Finished { dispatch, .. } => Some(dispatch),
                _ => None,
            })
            .unwrap();
        assert_eq!(finished, snapshot.dispatch, "{name}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_superseded_agent_operation_releases_its_reservation() {
    use denon_avr_domain::OperationOutcome as Outcome;
    let h = start(Setup {
        volume_db: -50.0,
        ..Setup::default()
    })
    .await;
    // The first call is superseded; later ones complete.
    let calls = AtomicUsize::new(0);
    h.connector.script(move |request| {
        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
            Outcome::SupersededBeforeDispatch {
                operation: request.id,
                by: OperationId(99),
            }
        } else {
            super::tests::completed(request)
        }
    });
    let snapshot = ask(&h.agent, set_volume(-44.0)).await;
    assert_eq!(
        snapshot.status,
        OperationStatus::Superseded {
            by: OperationId(99)
        }
    );
    assert_eq!(counted(&h), 0);
    // The receiver did not move, so asking again is judged as if the first never was.
    assert_eq!(
        ask(&h.agent, set_volume(-44.0)).await.status,
        OperationStatus::Completed
    );
}

#[tokio::test(start_paused = true)]
async fn an_operator_volume_write_enters_the_ledger() {
    let h = start(Setup {
        volume_db: -50.0,
        ..Setup::default()
    })
    .await;
    let operator = ask_as_operator(&h, set_volume(-44.0)).await;
    assert_eq!(operator.status, OperationStatus::Completed);
    assert_eq!(counted(&h), 1);

    // The agent's rise to -38 is 6 dB from the level it sees, and 12 above the
    // -50 the Operator rose from.
    let held = ask(&h.agent, set_volume(-38.0)).await;
    assert_eq!(held.status, OperationStatus::ApprovalUnavailable);

    // The Operator is recorded, never limited: -10 dB is over the agent's hard
    // limit, and the Operator's own write goes through.
    let loud = ask_as_operator(&h, set_volume(-10.0)).await;
    assert_eq!(loud.status, OperationStatus::Completed);
    let mute = ask_as_operator(
        &h,
        OperationSubmission::new(ReceiverIntent::Mute(MuteState::Off)),
    )
    .await;
    assert_eq!(mute.status, OperationStatus::Completed);
    assert_eq!(counted(&h), 2, "two volume writes; a mute is not counted");

    let records = h.audit.records();
    let operator_events: Vec<&AuditEvent> = records
        .iter()
        .filter(|record| record.principal == Principal::Operator && record.operation.is_some())
        .map(|record| &record.event)
        .collect();
    assert!(matches!(
        operator_events[0],
        AuditEvent::Decided {
            decision: AuditDecision::Allow,
            policy: None,
            ..
        }
    ));
    assert_eq!(
        operator_events[1],
        &AuditEvent::Dispatching {
            intent: "volume -44.0 dB".into(),
            before: Some(-100),
            target: Some(-88),
            precondition_fields: vec![],
        }
    );
    assert!(matches!(
        operator_events[2],
        AuditEvent::Finished {
            dispatch: DispatchCertainty::CompleteWrite,
            ..
        }
    ));
}

#[tokio::test(start_paused = true)]
async fn what_the_operator_did_while_the_log_was_unreadable_survives_the_late_rebuild() {
    let h = start(Setup {
        volume_db: -50.0,
        audit: AuditMode::FailAll,
        ..Setup::default()
    })
    .await;
    assert!(!h.agent.health().await.unwrap().ledger_ready);
    // The Operator is served, and the write counts although nothing could be written.
    assert_eq!(
        ask_as_operator(&h, set_volume(-44.0)).await.status,
        OperationStatus::Completed
    );

    h.audit.set_mode(AuditMode::Working);
    let held = ask(&h.agent, set_volume(-38.0)).await;
    assert_eq!(held.status, OperationStatus::ApprovalUnavailable);
    assert!(h.agent.health().await.unwrap().ledger_ready);
}

#[tokio::test(start_paused = true)]
async fn an_idempotent_retry_dispatches_once() {
    let h = start_default().await;
    let key = IdempotencyKey::new("retry-1").unwrap();
    let submission = set_volume(-32.0).with_idempotency_key(key);

    let first = h
        .agent
        .submit(&living_room(), submission.clone())
        .await
        .unwrap();
    let retry = h
        .agent
        .submit(&living_room(), submission.clone())
        .await
        .unwrap();
    assert_eq!(retry.id, first.id);
    h.agent
        .operation(first.id, Some(Duration::from_secs(5)))
        .await
        .unwrap();
    let after = h.agent.submit(&living_room(), submission).await.unwrap();
    assert_eq!(after.id, first.id);

    assert_eq!(operate_calls(&h), 1);
    let dispatching = h
        .audit
        .events()
        .iter()
        .filter(|event| matches!(event, AuditEvent::Dispatching { .. }))
        .count();
    assert_eq!(dispatching, 1);
}

#[tokio::test(start_paused = true)]
async fn a_cancel_while_the_receiver_connects_dispatches_nothing() {
    let h = start_default().await;
    let release = h.connector.hold_connect();
    let submitted = h
        .agent
        .submit(&living_room(), set_volume(-32.0))
        .await
        .unwrap();
    settle().await;
    assert_eq!(
        h.agent.operation(submitted.id, None).await.unwrap().status,
        OperationStatus::Submitted
    );

    let cancelled = h.agent.cancel(submitted.id).await.unwrap();
    assert_eq!(cancelled.status, OperationStatus::Cancelled);
    release.notify_one();
    settle().await;

    let done = h.agent.operation(submitted.id, None).await.unwrap();
    assert_eq!(done.status, OperationStatus::Cancelled);
    assert_eq!(operate_calls(&h), 0);
    assert_eq!(counted(&h), 0);
    assert!(!h
        .audit
        .events()
        .iter()
        .any(|event| matches!(event, AuditEvent::Dispatching { .. })));
}

#[tokio::test(start_paused = true)]
async fn only_the_record_made_before_a_write_is_synced_to_disk() {
    let h = start(Setup {
        volume_db: -50.0,
        ..Setup::default()
    })
    .await;
    ask(&h.agent, set_volume(-44.0)).await;
    ask_as_operator(&h, set_volume(-60.0)).await;
    ask(&h.agent, set_volume(-15.0)).await;

    let durability = locked(&h.audit.durability).clone();
    assert!(durability.len() >= 9, "{durability:?}");
    for (kind, how) in durability {
        let expected = if kind == "audit:dispatching" {
            Durability::Synced
        } else {
            Durability::Flushed
        };
        assert_eq!(how, expected, "{kind}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_cancel_while_the_dispatching_record_is_written_sends_nothing() {
    let h = start_default().await;
    let gate = h.audit.hold_appends();
    let submitted = h
        .agent
        .submit(&living_room(), set_volume(-32.0))
        .await
        .unwrap();
    settle().await;
    // The decision is recorded; the record of the write is being written.
    gate.add_permits(1);
    settle().await;
    assert_eq!(
        h.agent.operation(submitted.id, None).await.unwrap().status,
        OperationStatus::Allowed
    );
    assert_eq!(counted(&h), 1);

    assert_eq!(
        h.agent.cancel(submitted.id).await.unwrap().status,
        OperationStatus::Cancelled
    );
    gate.add_permits(10);
    settle().await;

    let done = h.agent.operation(submitted.id, None).await.unwrap();
    assert_eq!(done.status, OperationStatus::Cancelled);
    assert_eq!(operate_calls(&h), 0);
    assert_eq!(counted(&h), 0, "released, though the write was recorded");
    // The log shows a write that was recorded and then withdrawn, so a restart
    // does not count it.
    let events = h.audit.events();
    assert!(events
        .iter()
        .any(|event| matches!(event, AuditEvent::Dispatching { .. })));
    assert!(matches!(
        events.last().unwrap(),
        AuditEvent::Finished {
            dispatch: DispatchCertainty::NotDispatched,
            status,
            ..
        } if status == "cancelled"
    ));
}

#[tokio::test(start_paused = true)]
async fn the_budget_forgets_changes_older_than_its_window() {
    let h = start(Setup {
        volume_db: -50.0,
        ..Setup::default()
    })
    .await;
    assert_eq!(
        ask(&h.agent, set_volume(-44.0)).await.status,
        OperationStatus::Completed
    );

    // Within the ten minutes, a further rise to -38 is 12 above the -50 it left.
    h.clock.advance(Duration::from_secs(9 * 60));
    let held = ask(&h.other("second"), set_volume(-38.0)).await;
    assert_eq!(held.status, OperationStatus::ApprovalUnavailable);

    // Once the first change is a minute outside the window, the lowest level in
    // it is the -44 the receiver shows now, and the rise is 6.
    h.clock.advance(Duration::from_secs(2 * 60));
    let allowed = ask(&h.other("third"), set_volume(-38.0)).await;
    assert_eq!(allowed.status, OperationStatus::Completed);
}

#[tokio::test(start_paused = true)]
async fn a_running_service_forgets_what_is_older_than_a_day() {
    let h = start(Setup {
        volume_db: -50.0,
        ..Setup::default()
    })
    .await;
    let stored = |h: &AgentHarness| h.service.inner.agent.as_ref().unwrap().stored();

    // A write counts, and is stored.
    assert_eq!(
        ask(&h.agent, set_volume(-44.0)).await.status,
        OperationStatus::Completed
    );
    assert_eq!(stored(&h), 1);

    // A day and an hour later, the next write finds it expired and drops it. The
    // count by time alone would hide it, which is why the raw count is read.
    h.clock.advance(Duration::from_secs(25 * 60 * 60));
    assert_eq!(
        ask(&h.agent, set_volume(-60.0)).await.status,
        OperationStatus::Completed
    );
    assert_eq!(stored(&h), 1, "only the new write is kept");
    assert_eq!(counted(&h), 1);
}
