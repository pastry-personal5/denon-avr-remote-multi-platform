//! The owner's policy, receiver states and the harness the Agent path tests
//! start a service with.

use super::*;
use crate::control::{OperationControl, OperationSnapshot, OperationSubmission};
use denon_avr_domain::ConfiguredReceivers;

// ---- The owner's policy and a receiver state ----

pub(super) fn level(db: f64) -> Level {
    Level::from_db(db).unwrap()
}

fn span(db: f64) -> Span {
    Span::from_db(db).unwrap()
}

pub(super) fn owners_rules() -> Vec<Rule> {
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

pub(super) fn policy_of(rules: Vec<Rule>, digest: u8) -> LoadedPolicy {
    LoadedPolicy {
        config: PolicyConfig::new(rules, Duration::from_secs(300)).unwrap(),
        digest: PolicyDigest::from_bytes([digest; 32]),
        text: format!("# policy {digest}\n"),
    }
}

fn owners_policy() -> LoadedPolicy {
    policy_of(owners_rules(), 1)
}

pub(super) fn observation<T>(value: T) -> ReceiverObservation<T> {
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

pub(super) fn master(db: f64) -> MasterVolume {
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
pub(super) fn turn_dial(h: &AgentHarness, db: f64) {
    h.connector.session(0).states.send_modify(|state| {
        state
            .main_zone
            .volume
            .observe(observation(master(db)), MonotonicMillis(3_000));
    });
}

pub(super) fn mute_on() -> OperationSubmission {
    OperationSubmission::new(ReceiverIntent::Mute(MuteState::On))
}

pub(super) fn set_volume(db: f64) -> OperationSubmission {
    OperationSubmission::new(ReceiverIntent::Volume(master(db)))
}

// ---- The harness ----

pub(super) struct Setup {
    pub(super) policy: Result<LoadedPolicy, PolicyLoadError>,
    pub(super) audit: AuditMode,
    pub(super) limits: AgentLimits,
    pub(super) volume_db: f64,
    /// A receiver state to use instead of one at `volume_db`.
    pub(super) initial: Option<ReceiverState>,
    /// Records already in the audit log when the service starts.
    pub(super) history: Option<Vec<AuditRecord>>,
    /// The token store the service is given, when it has one.
    pub(super) tokens: Option<SharedTokenStore>,
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
            tokens: None,
        }
    }
}

pub(super) struct AgentHarness {
    pub(super) order: OrderLog,
    pub(super) service: Arc<ControlService>,
    pub(super) operator: SharedOperatorControl,
    pub(super) agent: Arc<ServiceHandle>,
    pub(super) connector: Arc<FakeConnector>,
    pub(super) audit: Arc<FakeAudit>,
    pub(super) policy: Arc<FakePolicy>,
    pub(super) clock: Arc<FakeClock>,
}

pub(super) fn label(name: &str) -> AgentLabel {
    AgentLabel::new(name).unwrap()
}

pub(super) async fn start(setup: Setup) -> AgentHarness {
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
                tokens: setup.tokens,
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

pub(super) async fn start_default() -> AgentHarness {
    start(Setup::default()).await
}

impl AgentHarness {
    pub(super) fn other(&self, name: &str) -> Arc<ServiceHandle> {
        self.service.handle(Principal::Agent(label(name))).unwrap()
    }
}

/// Submit as `handle` and wait for the operation to end.
pub(super) async fn ask(
    handle: &ServiceHandle,
    submission: OperationSubmission,
) -> OperationSnapshot {
    let first = handle.submit(&living_room(), submission).await.unwrap();
    let snapshot = handle
        .operation(first.id, Some(Duration::from_secs(5)))
        .await
        .unwrap();
    assert!(snapshot.status.is_terminal(), "{snapshot:?}");
    snapshot
}

pub(super) fn operate_calls(h: &AgentHarness) -> usize {
    (0..h.connector.sessions())
        .map(|index| h.connector.session(index).calls().len())
        .sum()
}

pub(super) async fn ask_as_operator(
    h: &AgentHarness,
    submission: OperationSubmission,
) -> OperationSnapshot {
    let first = h.operator.submit(&living_room(), submission).await.unwrap();
    h.operator
        .operation(first.id, Some(Duration::from_secs(5)))
        .await
        .unwrap()
}

impl AgentHarness {
    /// A handle for an agent, as `refresh_is_the_operators_alone` forges one.
    pub(super) fn agent_forged(&self) -> ServiceHandle {
        ServiceHandle {
            inner: Arc::clone(&self.service.inner),
            principal: Principal::Agent(label("openclaw")),
        }
    }
}
