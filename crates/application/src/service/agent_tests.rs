//! Tests of the Agent path through the gate: evaluation, refusal, the caps, and
//! the policy and ledger it depends on. They run on a paused clock with the
//! fakes of `tests.rs`, a fake audit log, a fake policy source, and a clock the
//! test sets.

use super::tests::{
    advance, living_room, settle, FakeConfig, FakeConnector, FakeDiscovery, OrderLog,
};
use super::*;
use crate::audit::{AuditEntry, AuditError, AuditLog, AuditPage, AuditQuery, Durability};
use crate::clock::Clock;
use crate::control::{AgentLabel, ApprovalHealth, AuditHealth, DryRunDecision, PolicyHealth};
use crate::policy_source::{LoadedPolicy, PolicyDigest, PolicyLoadError, PolicySource};
use crate::{AuditDecision, AuditEvent, AuditRecord};
use denon_avr_domain::{
    Epoch, FrameSeq, MasterVolume, MonotonicMillis, MuteState, ObservationOrigin,
    ReceiverObservation, ReceiverState, WallTime,
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

    // Moves the wall clock for the budget window; the dispatch tests use it.
    #[allow(dead_code)]
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

// The append failures are what the dispatch tests inject.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum AuditMode {
    Working,
    FailAppend,
    HangAppend,
    FailRead,
}

struct FakeAudit {
    records: Mutex<Vec<AuditRecord>>,
    mode: Mutex<AuditMode>,
    order: Mutex<Option<OrderLog>>,
}

impl FakeAudit {
    fn new(mode: AuditMode) -> Arc<Self> {
        Arc::new(Self {
            records: Mutex::new(Vec::new()),
            mode: Mutex::new(mode),
            order: Mutex::new(None),
        })
    }

    fn set_mode(&self, mode: AuditMode) {
        *locked(&self.mode) = mode;
    }

    #[allow(dead_code)]
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
    fn append(&self, record: AuditRecord, _: Durability) -> BoxFuture<'_, Result<(), AuditError>> {
        Box::pin(async move {
            let mode = *locked(&self.mode);
            match mode {
                AuditMode::FailAppend => Err(AuditError::new("/private/audit/dir is full")),
                AuditMode::HangAppend => std::future::pending().await,
                AuditMode::Working | AuditMode::FailRead => {
                    if let Some(order) = locked(&self.order).as_ref() {
                        locked(order).push(event_name(&record.event).into());
                    }
                    locked(&self.records).push(record);
                    Ok(())
                }
            }
        })
    }

    fn since(&self, from: WallTime) -> BoxFuture<'_, Result<Vec<AuditRecord>, AuditError>> {
        Box::pin(async move {
            if *locked(&self.mode) == AuditMode::FailRead {
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
            if *locked(&self.mode) == AuditMode::FailRead {
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

fn set_volume(db: f64) -> OperationSubmission {
    OperationSubmission::new(ReceiverIntent::Volume(master(db)))
}

// ---- The harness ----

struct Setup {
    policy: Result<LoadedPolicy, PolicyLoadError>,
    audit: AuditMode,
    limits: AgentLimits,
    volume_db: f64,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            policy: Ok(owners_policy()),
            audit: AuditMode::Working,
            limits: AgentLimits::default(),
            volume_db: -35.0,
        }
    }
}

struct AgentHarness {
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
    connector.start_in(receiver_at(setup.volume_db));
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
async fn an_allow_is_rejected_until_dispatch_is_enabled() {
    let h = start_default().await;
    let snapshot = ask(&h.agent, set_volume(-32.0)).await;
    assert_eq!(snapshot.status, OperationStatus::Rejected);
    assert_eq!(snapshot.dispatch, DispatchCertainty::NotDispatched);
    assert_eq!(
        snapshot.reason.as_deref(),
        Some("agent dispatch is not enabled")
    );
    assert_eq!(operate_calls(&h), 0);
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
    // Thirty distinct requests, each finished before the next, are thirty writes.
    for step in 0..30 {
        let db = -79.5 + f64::from(step) * 0.5;
        let snapshot = ask(&h.agent, set_volume(db)).await;
        assert_eq!(snapshot.status, OperationStatus::Rejected, "{db}");
    }
    let records = h.audit.records().len();

    let refused = h
        .agent
        .submit(&living_room(), set_volume(-60.0))
        .await
        .unwrap_err();
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
        ask(&other, set_volume(-60.0)).await.status,
        OperationStatus::Rejected
    );

    // After a minute the oldest write has left the window.
    advance(Duration::from_secs(61)).await;
    assert!(h
        .agent
        .submit(&living_room(), set_volume(-60.0))
        .await
        .is_ok());
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
