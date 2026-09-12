//! State-first X3800H session adapter.
//!
//! The legacy [`AvrSession`] remains available for the v2 controller while it
//! is being removed.  This type is the Phase 5 production-facing boundary: it
//! owns the only public core-state copy and publishes complete snapshots with
//! a `watch` channel.

use crate::avr_session::{AvrSession, AvrSessionError, AvrSessionEvent};
use crate::AvrSessionConfig;
use denon_avr_application::ports::{OperationError, ReceiverSession};
use denon_avr_application::{
    CanonicalReceiverSession, OperationRequest, Readiness, StateSubscription,
};
use denon_avr_domain::{
    CoreField, CoreFrame, DispatchCertainty, Epoch, FrameSeq, MonotonicMillis, ObservationOrigin,
    OperationOutcome, ReceiverId, ReceiverIntent, ReceiverState, SyncCause, SyncCycleId, SyncDebt,
};
use denon_avr_protocol::avr::{encode_x3800h, parse_x3800h, x3800h_query, X3800hFrame};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::net::lookup_host;
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

const CONTROL_OBSERVATION_WINDOW: std::time::Duration = std::time::Duration::from_secs(5);
const CONTROL_RECHECK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
const MAX_USER_ACTIONS_BEFORE_DEBT: u8 = 8;
const MAX_USER_WORK_WINDOW: std::time::Duration = std::time::Duration::from_millis(500);

fn core_intents() -> Vec<ReceiverIntent> {
    vec![
        ReceiverIntent::SystemPower(denon_avr_domain::SystemPower::On),
        ReceiverIntent::MainZonePower(denon_avr_domain::ZonePower::On),
        ReceiverIntent::Zone2Power(denon_avr_domain::ZonePower::On),
        ReceiverIntent::Source(denon_avr_domain::SourceId::new("CD").expect("fixed source")),
        ReceiverIntent::Volume(denon_avr_domain::MasterVolume::Minimum),
        ReceiverIntent::Mute(denon_avr_domain::MuteState::Off),
        ReceiverIntent::SoundMode(denon_avr_domain::SoundModeIntent::Auto),
    ]
}

fn intent_for_field(field: CoreField) -> ReceiverIntent {
    match field {
        CoreField::SystemPower => ReceiverIntent::SystemPower(denon_avr_domain::SystemPower::On),
        CoreField::MainZonePower => ReceiverIntent::MainZonePower(denon_avr_domain::ZonePower::On),
        CoreField::Zone2Power => ReceiverIntent::Zone2Power(denon_avr_domain::ZonePower::On),
        CoreField::Source => {
            ReceiverIntent::Source(denon_avr_domain::SourceId::new("CD").expect("fixed source"))
        }
        CoreField::Volume => ReceiverIntent::Volume(denon_avr_domain::MasterVolume::Minimum),
        CoreField::Mute => ReceiverIntent::Mute(denon_avr_domain::MuteState::Off),
        CoreField::SoundMode => ReceiverIntent::SoundMode(denon_avr_domain::SoundModeIntent::Auto),
    }
}

enum Command {
    Observe(CoreField, oneshot::Sender<Result<(), OperationError>>),
    Synchronize(oneshot::Sender<Result<Readiness, OperationError>>),
    Operate(OperationRequest, oneshot::Sender<OperationOutcome>),
    Close(oneshot::Sender<Result<(), OperationError>>),
}

/// Cloneable handle to one serialized receiver connection.  Cloning this
/// handle only clones the mailbox sender; it never opens another socket.
pub struct X3800hSession {
    commands: mpsc::Sender<Command>,
    states: watch::Sender<ReceiverState>,
    actor: Arc<Mutex<Option<JoinHandle<()>>>>,
    closed: AtomicBool,
}

impl X3800hSession {
    pub async fn connect(
        receiver: ReceiverId,
        host: &str,
        config: AvrSessionConfig,
    ) -> Result<Arc<Self>, OperationError> {
        let address = tokio::time::timeout(config.connect_timeout, lookup_host((host, 23)))
            .await
            .map_err(|_| {
                OperationError::new(
                    denon_avr_application::ports::OperationErrorKind::Timeout,
                    "resolving receiver",
                    "receiver address resolution timed out",
                )
            })?
            .map_err(|error| {
                OperationError::new(
                    denon_avr_application::ports::OperationErrorKind::Connection,
                    "resolving receiver",
                    error.to_string(),
                )
            })?
            .next()
            .ok_or_else(|| {
                OperationError::new(
                    denon_avr_application::ports::OperationErrorKind::Connection,
                    "resolving receiver",
                    "receiver host has no address",
                )
            })?;
        Self::connect_addr(receiver, address, config).await
    }

    pub async fn connect_addr(
        receiver: ReceiverId,
        address: SocketAddr,
        mut config: AvrSessionConfig,
    ) -> Result<Arc<Self>, OperationError> {
        info!(receiver = receiver.as_str(), %address, "connecting to X3800H receiver");
        // Canonical monitoring is long-lived. A temporary connection refusal
        // is degraded evidence, never a reason to silently stop monitoring.
        config.reconnect_indefinitely = true;
        let avr = AvrSession::connect_addr(address, config)
            .await
            .map_err(|error| {
                let mapped = OperationError::from(error);
                OperationError::new(
                    mapped.kind,
                    mapped.context,
                    initial_connection_guidance(mapped.message),
                )
            })?;
        let mut initial = ReceiverState::new(receiver);
        initial.establish_epoch(Epoch(1));
        let (states, _) = watch::channel(initial);
        let (commands, rx) = mpsc::channel(32);
        let actor = Arc::new(Mutex::new(None));
        let handle = Arc::new(Self {
            commands,
            states,
            actor: Arc::clone(&actor),
            closed: AtomicBool::new(false),
        });
        let task = tokio::spawn(run_actor(avr, rx, handle.states.clone()));
        *actor.lock().await = Some(task);
        Ok(handle)
    }

    pub fn current_state(&self) -> ReceiverState {
        self.states.borrow().clone()
    }
}

fn initial_connection_guidance(message: impl Into<String>) -> String {
    format!(
        "{}; verify Network Control is enabled and no other Telnet client is connected",
        message.into()
    )
}

impl CanonicalReceiverSession for X3800hSession {
    fn state(&self) -> StateSubscription {
        StateSubscription::new(self.states.subscribe())
    }

    fn observe(
        &self,
        field: CoreField,
    ) -> denon_avr_application::ports::BoxFuture<'_, Result<(), OperationError>> {
        Box::pin(async move {
            let (reply, result) = oneshot::channel();
            self.commands
                .send(Command::Observe(field, reply))
                .await
                .map_err(|_| stopped())?;
            result.await.map_err(|_| stopped())?
        })
    }

    fn synchronize(
        &self,
    ) -> denon_avr_application::ports::BoxFuture<'_, Result<Readiness, OperationError>> {
        Box::pin(async move {
            let (reply, result) = oneshot::channel();
            self.commands
                .send(Command::Synchronize(reply))
                .await
                .map_err(|_| stopped())?;
            result.await.map_err(|_| stopped())?
        })
    }

    fn operate(
        &self,
        request: OperationRequest,
    ) -> denon_avr_application::ports::BoxFuture<'_, OperationOutcome> {
        Box::pin(async move {
            let (reply, result) = oneshot::channel();
            let operation = request.id;
            if self
                .commands
                .send(Command::Operate(request, reply))
                .await
                .is_err()
            {
                return OperationOutcome::Indeterminate {
                    operation,
                    dispatch: DispatchCertainty::NotDispatched,
                    reason: "receiver session stopped before operation admission".into(),
                };
            }
            result.await.unwrap_or(OperationOutcome::Indeterminate {
                operation,
                dispatch: DispatchCertainty::NotDispatched,
                reason: "receiver session stopped before operation completion".into(),
            })
        })
    }

    fn close(&self) -> denon_avr_application::ports::BoxFuture<'_, Result<(), OperationError>> {
        Box::pin(async move {
            if self.closed.swap(true, Ordering::AcqRel) {
                return Ok(());
            }
            let (reply, result) = oneshot::channel();
            if self.commands.send(Command::Close(reply)).await.is_err() {
                return Err(stopped());
            }
            match tokio::time::timeout(std::time::Duration::from_millis(250), result).await {
                Ok(result) => result.map_err(|_| stopped())?,
                Err(_) => {
                    // The actor can be waiting for an indefinitely retrying
                    // transport. Abort the owned task; AvrSession's Drop
                    // implementation then aborts its socket owner as well.
                    if let Some(task) = self.actor.lock().await.take() {
                        task.abort();
                        let _ = task.await;
                    }
                    Ok(())
                }
            }
        })
    }
}

async fn run_actor(
    mut avr: AvrSession,
    mut commands: mpsc::Receiver<Command>,
    states: watch::Sender<ReceiverState>,
) {
    debug!("X3800H canonical session actor started");
    let started = Instant::now();
    let mut epoch = Epoch(1);
    let mut frame_seq = FrameSeq(0);
    let mut cycle = SyncCycleId(0);
    let mut debt: Option<SyncDebt> = None;
    let mut user_work_started = Instant::now();
    let mut user_action_count = 0_u8;
    // Socket establishment enters Synchronizing immediately. Consumers may
    // still request an explicit synchronize; that request simply performs a
    // fresh deterministic pass against the same canonical state.
    cycle.0 = 1;
    let readiness = synchronize(&avr, &states, &mut epoch, &mut frame_seq, cycle, started).await;
    clear_settled_debt(&mut debt, &readiness, started);
    let mut sweep = tokio::time::interval(std::time::Duration::from_secs(5));
    let mut debt_tick = tokio::time::interval(std::time::Duration::from_millis(50));
    // Consume interval's immediate first tick; the first sweep is due five
    // seconds after actor startup, not during initial synchronization.
    sweep.tick().await;
    loop {
        if fairness_budget_due(&debt, started, user_work_started, user_action_count) {
            service_debt(
                &avr,
                &states,
                &mut debt,
                &mut epoch,
                &mut frame_seq,
                cycle,
                started,
            )
            .await;
            user_work_started = Instant::now();
            user_action_count = 0;
            continue;
        }
        tokio::select! {
            _ = debt_tick.tick() => {
                service_debt(&avr, &states, &mut debt, &mut epoch, &mut frame_seq, cycle, started).await;
            }
            _ = sweep.tick() => {
                expire_state(&states, started);
                cycle.0 = cycle.0.saturating_add(1);
                let readiness = synchronize(&avr, &states, &mut epoch, &mut frame_seq, cycle, started).await;
                clear_settled_debt(&mut debt, &readiness, started);
            }
            event = avr.next_event() => match event {
                Some(event) => {
                    if let Some(field) = apply_event(event, &states, &mut epoch, &mut frame_seq, started) {
                        merge_debt(&mut debt, &states, field, SyncCause::ReceiverEvent, cycle, started);
                    }
                },
                None => break,
            },
            command = commands.recv() => match command {
                Some(Command::Observe(field, reply)) => {
                    user_action_count = user_action_count.saturating_add(1);
                    cycle.0 = cycle.0.saturating_add(1);
                    mark_converging(&states, field, SyncCause::ManualRefresh, cycle, started);
                    let intent = intent_for_field(field);
                    let result = query_and_reduce(&avr, &intent, &states, &mut epoch, &mut frame_seq, started).await;
                    match &result {
                        Ok(()) => mark_settled(&states, field, cycle, started),
                        Err(error) => mark_query_failed(&states, field, error.to_string()),
                    }
                    let _ = reply.send(result);
                }
                Some(Command::Synchronize(reply)) => {
                    user_action_count = user_action_count.saturating_add(1);
                    cycle.0 = cycle.0.saturating_add(1);
                    let result = synchronize(&avr, &states, &mut epoch, &mut frame_seq, cycle, started).await;
                    clear_settled_debt(&mut debt, &result, started);
                    let _ = reply.send(result);
                }
                Some(Command::Operate(request, reply)) => {
                    user_action_count = user_action_count.saturating_add(1);
                    cycle.0 = cycle.0.saturating_add(1);
                    if !reply.is_closed() {
                        for field in dependency_fields(request.intent.field()) {
                            merge_debt(
                                &mut debt,
                                &states,
                                *field,
                                SyncCause::LocalControl,
                                cycle,
                                started,
                            );
                        }
                    }
                    let outcome = operate(
                        &mut avr,
                        request.id,
                        request.intent,
                        &reply,
                        &states,
                        &mut debt,
                        &mut epoch,
                        &mut frame_seq,
                        cycle,
                        started,
                    )
                    .await;
                    let _ = reply.send(outcome);
                }
                Some(Command::Close(reply)) => {
                    let result = avr.close().await;
                    let _ = reply.send(result);
                    break;
                }
                None => { let _ = avr.close().await; break; }
            }
        }
    }
    debug!("X3800H canonical session actor stopped");
}

async fn service_debt(
    avr: &AvrSession,
    states: &watch::Sender<ReceiverState>,
    debt: &mut Option<SyncDebt>,
    epoch: &mut Epoch,
    frame_seq: &mut FrameSeq,
    cycle: SyncCycleId,
    started: Instant,
) {
    let Some(current) = debt.as_ref() else { return };
    let now = MonotonicMillis(started.elapsed().as_millis() as u64);
    let final_pass = current.final_due(now);
    if !final_pass && !current.quick_due(now) {
        return;
    }
    let fields = current.fields.iter().copied().collect::<Vec<_>>();
    if fields.is_empty() {
        if final_pass {
            *debt = None;
        }
        return;
    }
    let receiver_epoch = current.epoch;
    let receiver = current.receiver.clone();
    let mut all_succeeded = true;
    for field in fields {
        let intent = intent_for_field(field);
        match query_and_reduce(avr, &intent, states, epoch, frame_seq, started).await {
            Ok(()) => {
                mark_settled(states, field, cycle, started);
                if final_pass {
                    if let Some(value) = debt.as_mut() {
                        value.settle_field(field);
                    }
                }
            }
            Err(error) => {
                all_succeeded = false;
                mark_query_failed(states, field, error.to_string());
            }
        }
    }
    let Some(value) = debt.as_mut() else { return };
    if !value.is_for(&receiver, receiver_epoch) {
        return;
    }
    if all_succeeded && (value.final_reconcile_at.is_none() || (final_pass && value.settled())) {
        *debt = None;
    } else if !final_pass {
        let next = value
            .final_reconcile_at
            .unwrap_or(MonotonicMillis(now.0.saturating_add(5_000)));
        value.defer_quick_until(next);
    }
}

fn fairness_budget_due(
    debt: &Option<SyncDebt>,
    started: Instant,
    user_work_started: Instant,
    user_action_count: u8,
) -> bool {
    let now = MonotonicMillis(started.elapsed().as_millis() as u64);
    let reconciliation_due = debt
        .as_ref()
        .is_some_and(|value| value.quick_due(now) || value.final_due(now));
    reconciliation_due
        && (user_action_count >= MAX_USER_ACTIONS_BEFORE_DEBT
            || user_work_started.elapsed() >= MAX_USER_WORK_WINDOW)
}

async fn synchronize(
    avr: &AvrSession,
    states: &watch::Sender<ReceiverState>,
    epoch: &mut Epoch,
    frame_seq: &mut FrameSeq,
    cycle: SyncCycleId,
    started: Instant,
) -> Result<Readiness, OperationError> {
    let mut degraded = false;
    for intent in &core_intents() {
        mark_converging(
            states,
            intent.field(),
            SyncCause::ManualRefresh,
            cycle,
            started,
        );
        match query_and_reduce(avr, intent, states, epoch, frame_seq, started).await {
            Ok(()) => mark_settled(states, intent.field(), cycle, started),
            Err(error) => {
                mark_query_failed(states, intent.field(), error.to_string());
                degraded = true;
            }
        }
    }
    Ok(Readiness {
        ready: !degraded,
        degraded,
        detail: if degraded {
            "one or more core fields could not be observed".into()
        } else {
            "core receiver state synchronized".into()
        },
    })
}

#[allow(clippy::too_many_arguments)]
async fn operate(
    avr: &mut AvrSession,
    operation: denon_avr_domain::OperationId,
    intent: ReceiverIntent,
    reply: &oneshot::Sender<OperationOutcome>,
    states: &watch::Sender<ReceiverState>,
    debt: &mut Option<SyncDebt>,
    epoch: &mut Epoch,
    frame_seq: &mut FrameSeq,
    cycle: SyncCycleId,
    started: Instant,
) -> OperationOutcome {
    let field = intent.field();
    let dependencies = dependency_fields(field);
    if reply.is_closed() {
        return OperationOutcome::Cancelled { operation };
    }
    if let Err(reason) = admit_intent(&intent) {
        return OperationOutcome::RejectedBeforeDispatch { operation, reason };
    }
    for dependency in dependencies {
        mark_converging(states, *dependency, SyncCause::LocalControl, cycle, started);
    }
    match query_and_reduce(avr, &intent, states, epoch, frame_seq, started).await {
        Ok(()) if state_matches(&states.borrow(), &intent) => {
            for dependency in dependencies {
                mark_settled(states, *dependency, cycle, started);
            }
            return OperationOutcome::AlreadyObserved {
                operation,
                observation: "targeted preflight observation".into(),
            };
        }
        Ok(()) => {}
        Err(error) => {
            mark_query_failed(states, field, error.to_string());
            return OperationOutcome::RejectedBeforeDispatch {
                operation,
                reason: format!("preflight failed: {error}"),
            };
        }
    }
    if reply.is_closed() {
        return OperationOutcome::Cancelled { operation };
    }
    let command = match encode_x3800h(&intent) {
        Ok(command) => command,
        Err(error) => {
            return OperationOutcome::RejectedBeforeDispatch {
                operation,
                reason: error.to_string(),
            }
        }
    };
    if let Err(error) = avr.dispatch(command.as_str()).await {
        return match error {
            AvrSessionError::InvalidCommand(message) => OperationOutcome::RejectedBeforeDispatch {
                operation,
                reason: format!("command was not dispatched: {message}"),
            },
            AvrSessionError::SessionStopped => OperationOutcome::RejectedBeforeDispatch {
                operation,
                reason: "session stopped before command dispatch".into(),
            },
            error => {
                // A failed write is never replayed. A read-only observation
                // may nevertheless prove that the receiver applied it.
                match query_and_reduce(avr, &intent, states, epoch, frame_seq, started).await {
                    Ok(()) if state_matches(&states.borrow(), &intent) => {
                        OperationOutcome::ObservedRequestedValue {
                            operation,
                            dispatch: DispatchCertainty::Unknown,
                            observation: "receiver observation after ambiguous write".into(),
                        }
                    }
                    Ok(()) => OperationOutcome::Indeterminate {
                        operation,
                        dispatch: DispatchCertainty::Unknown,
                        reason: format!("write began but delivery is ambiguous: {error}"),
                    },
                    Err(observation_error) => OperationOutcome::Indeterminate {
                        operation,
                        dispatch: DispatchCertainty::Unknown,
                        reason: format!(
                            "write began but delivery is ambiguous ({error}); confirmation failed: {observation_error}"
                        ),
                    },
                }
            }
        };
    }
    let write_completed_at = MonotonicMillis(started.elapsed().as_millis() as u64);
    if let Some(debt) = debt.as_mut() {
        debt.defer_final_until(MonotonicMillis(write_completed_at.0 + 5_000));
    }
    let deadline = tokio::time::Instant::now() + CONTROL_OBSERVATION_WINDOW;
    loop {
        match query_and_reduce(avr, &intent, states, epoch, frame_seq, started).await {
            Ok(()) if state_matches(&states.borrow(), &intent) => {
                // The target was observed, so feedback can complete
                // immediately. Dependencies remain converging until the
                // shared periodic pass settles the complete debt cycle.
                break OperationOutcome::ObservedRequestedValue {
                    operation,
                    dispatch: DispatchCertainty::CompleteWrite,
                    observation: "post-dispatch receiver observation".into(),
                };
            }
            Ok(()) if tokio::time::Instant::now() < deadline => {
                tokio::select! {
                    _ = tokio::time::sleep(CONTROL_RECHECK_INTERVAL) => {}
                    event = avr.next_event() => match event {
                        Some(event) => { let _ = apply_event(event, states, epoch, frame_seq, started); }
                        None => break OperationOutcome::Indeterminate {
                            operation,
                            dispatch: DispatchCertainty::CompleteWrite,
                            reason: "receiver session stopped during confirmation window".into(),
                        },
                    }
                }
            }
            Ok(()) => {
                break OperationOutcome::Indeterminate {
                    operation,
                    dispatch: DispatchCertainty::CompleteWrite,
                    reason: "requested value was not observed within the confirmation window"
                        .into(),
                };
            }
            Err(error) => {
                mark_query_failed(states, field, error.to_string());
                break OperationOutcome::Indeterminate {
                    operation,
                    dispatch: DispatchCertainty::CompleteWrite,
                    reason: format!("post-dispatch observation failed: {error}"),
                };
            }
        }
    }
}

fn dependency_fields(field: CoreField) -> &'static [CoreField] {
    match field {
        CoreField::SystemPower => &[
            CoreField::SystemPower,
            CoreField::MainZonePower,
            CoreField::Source,
            CoreField::Volume,
            CoreField::Mute,
            CoreField::SoundMode,
            CoreField::Zone2Power,
        ],
        CoreField::MainZonePower => &[
            CoreField::MainZonePower,
            CoreField::Source,
            CoreField::Volume,
            CoreField::Mute,
            CoreField::SoundMode,
        ],
        CoreField::Source => &[CoreField::Source, CoreField::SoundMode],
        CoreField::Volume => &[CoreField::Volume],
        CoreField::Mute => &[CoreField::Mute],
        CoreField::SoundMode => &[CoreField::SoundMode],
        CoreField::Zone2Power => &[CoreField::Zone2Power],
    }
}

/// Admit only chart-backed X3800H source identifiers until runtime catalog
/// evidence is available. Display labels and arbitrary protocol tokens never
/// reach the encoder.
fn admit_intent(intent: &ReceiverIntent) -> Result<(), String> {
    match intent {
        ReceiverIntent::Source(source)
            if denon_avr_domain::supports_x3800h_source(source.as_str()) =>
        {
            Ok(())
        }
        ReceiverIntent::Source(_) => Err("source is not supported by the X3800H profile".into()),
        ReceiverIntent::SoundMode(denon_avr_domain::SoundModeIntent::Select(mode))
            if denon_avr_domain::supports_x3800h_sound_mode(mode) =>
        {
            Ok(())
        }
        ReceiverIntent::SoundMode(denon_avr_domain::SoundModeIntent::Select(_)) => {
            Err("sound mode is not supported by the X3800H profile".into())
        }
        _ => Ok(()),
    }
}

async fn query_and_reduce(
    avr: &AvrSession,
    intent: &ReceiverIntent,
    states: &watch::Sender<ReceiverState>,
    epoch: &mut Epoch,
    frame_seq: &mut FrameSeq,
    started: Instant,
) -> Result<(), OperationError> {
    synchronize_generation(avr, states, epoch);
    let line = avr
        .request(x3800h_query(intent).as_str())
        .await
        .map_err(OperationError::from)?;
    synchronize_generation(avr, states, epoch);
    reduce_line(states, *epoch, frame_seq, started, &line, true);
    Ok(())
}

/// The low-level socket actor may reconnect while an outstanding query is
/// resolved. Bind every reduced response to its actual socket generation even
/// when lifecycle notifications are still queued behind that response.
fn synchronize_generation(
    avr: &AvrSession,
    states: &watch::Sender<ReceiverState>,
    epoch: &mut Epoch,
) {
    let observed = Epoch(avr.connection_generation().saturating_add(1));
    if observed == *epoch {
        return;
    }
    let mut state = states.borrow().clone();
    if state.epoch.is_some() {
        state.mark_disconnected();
    }
    state.establish_epoch(observed);
    states.send_replace(state);
    *epoch = observed;
}

fn apply_event(
    event: AvrSessionEvent,
    states: &watch::Sender<ReceiverState>,
    epoch: &mut Epoch,
    frame_seq: &mut FrameSeq,
    started: Instant,
) -> Option<CoreField> {
    match event {
        AvrSessionEvent::Line(line) => {
            return reduce_line(states, *epoch, frame_seq, started, &line, false)
        }
        AvrSessionEvent::Disconnected {
            generation,
            message,
        } => {
            warn!(generation, %message, "receiver transport disconnected");
            // A reconnect may finish while this lifecycle notification is in
            // the mailbox. Never let old-connection failure evidence make a
            // newer epoch stale.
            if generation.saturating_add(1) < epoch.0 {
                return None;
            }
            let mut state = states.borrow().clone();
            state.mark_disconnected();
            states.send_replace(state);
            let _ = message;
        }
        AvrSessionEvent::Connected => {
            info!("receiver transport connected");
            // `connect_addr` establishes epoch one before the actor starts.
            // The initial lifecycle notification is evidence of that same
            // connection, not a second connection.
        }
        AvrSessionEvent::Reconnected => {
            info!("receiver transport reconnected");
            epoch.0 = epoch.0.saturating_add(1);
            let mut state = states.borrow().clone();
            // Lifecycle notifications can be reordered relative to the
            // actor's command mailbox. Always retire the prior epoch before
            // installing the newly established socket epoch.
            state.mark_disconnected();
            state.establish_epoch(*epoch);
            states.send_replace(state);
        }
    }
    None
}

fn reduce_line(
    states: &watch::Sender<ReceiverState>,
    epoch: Epoch,
    frame_seq: &mut FrameSeq,
    started: Instant,
    line: &str,
    in_query: bool,
) -> Option<CoreField> {
    // Sequence every received line before parsing so diagnostics and typed
    // observations share one monotonic stream within the connection epoch.
    frame_seq.0 = frame_seq.0.saturating_add(1);
    let Ok(parsed) = parse_x3800h(line) else {
        warn!(frame = %line, "malformed X3800H frame retained as diagnostic");
        let mut state = states.borrow().clone();
        state.record_diagnostic(format!("malformed: {line}"));
        states.send_replace(state);
        return None;
    };
    // `MVMAX …` is a supported AVR metadata frame. It has no Main Zone state
    // equivalent, so ignore it without manufacturing diagnostic churn.
    if matches!(&parsed, X3800hFrame::VolumeLimit(_)) {
        return None;
    }
    let Some(frame) = core_frame(parsed.clone()) else {
        debug!(frame = %line, "unknown X3800H frame retained as diagnostic");
        let mut state = states.borrow().clone();
        state.record_diagnostic(line);
        states.send_replace(state);
        return None;
    };
    let field = frame.field();
    let now = MonotonicMillis(started.elapsed().as_millis() as u64);
    let mut state = states.borrow().clone();
    let receiver = state.receiver.clone();
    state.reduce(
        &receiver,
        epoch,
        *frame_seq,
        now,
        MonotonicMillis(now.0 + 10_000),
        if in_query {
            ObservationOrigin::ReceiverFrameInQueryWindow { query: frame_seq.0 }
        } else {
            ObservationOrigin::ReceiverFrame
        },
        frame,
    );
    if !in_query {
        state.mark_converging(
            field,
            SyncCycleId(frame_seq.0),
            SyncCause::ReceiverEvent,
            MonotonicMillis(now.0 + 250),
        );
    }
    states.send_replace(state);
    Some(field)
}

fn merge_debt(
    debt: &mut Option<SyncDebt>,
    states: &watch::Sender<ReceiverState>,
    field: CoreField,
    cause: SyncCause,
    cycle: SyncCycleId,
    started: Instant,
) {
    let state = states.borrow();
    let Some(epoch) = state.epoch else { return };
    let receiver = state.receiver.clone();
    let now = MonotonicMillis(started.elapsed().as_millis() as u64);
    match debt {
        Some(existing) if existing.is_for(&receiver, epoch) => {
            existing.merge([field], cause);
            existing.note_activity(now);
        }
        _ => *debt = Some(SyncDebt::new(receiver, epoch, cycle, field, cause, now)),
    }
}

fn clear_settled_debt(
    debt: &mut Option<SyncDebt>,
    readiness: &Result<Readiness, OperationError>,
    started: Instant,
) {
    if !readiness.as_ref().is_ok_and(|value| value.ready) {
        return;
    }
    let now = MonotonicMillis(started.elapsed().as_millis() as u64);
    if debt
        .as_ref()
        .is_none_or(|value| value.final_due(now) || value.final_reconcile_at.is_none())
    {
        *debt = None;
    }
}

fn core_frame(frame: X3800hFrame) -> Option<CoreFrame> {
    crate::x3800h_reducer::core_frame(frame)
}

fn mark_converging(
    states: &watch::Sender<ReceiverState>,
    field: CoreField,
    cause: SyncCause,
    cycle: SyncCycleId,
    started: Instant,
) {
    let mut state = states.borrow().clone();
    let now = MonotonicMillis(started.elapsed().as_millis() as u64);
    state.mark_converging(field, cycle, cause, MonotonicMillis(now.0 + 250));
    states.send_replace(state);
}

fn mark_settled(
    states: &watch::Sender<ReceiverState>,
    field: CoreField,
    cycle: SyncCycleId,
    started: Instant,
) {
    let mut state = states.borrow().clone();
    state.mark_settled(
        field,
        cycle,
        MonotonicMillis(started.elapsed().as_millis() as u64),
    );
    states.send_replace(state);
}

fn mark_query_failed(states: &watch::Sender<ReceiverState>, field: CoreField, message: String) {
    warn!(?field, %message, "canonical receiver observation failed");
    let mut state = states.borrow().clone();
    state.mark_query_failed(field, message);
    states.send_replace(state);
}

fn expire_state(states: &watch::Sender<ReceiverState>, started: Instant) {
    let now = MonotonicMillis(started.elapsed().as_millis() as u64);
    let mut state = states.borrow().clone();
    if state.expire_fields(now) {
        states.send_replace(state);
    }
}

fn state_matches(state: &ReceiverState, intent: &ReceiverIntent) -> bool {
    match intent {
        ReceiverIntent::SystemPower(value) => {
            current_matches(&state.system_power, |item| item == value)
        }
        ReceiverIntent::MainZonePower(value) => {
            current_matches(&state.main_zone.power, |item| item == value)
        }
        ReceiverIntent::Zone2Power(value) => {
            current_matches(&state.zone2_power, |item| item == value)
        }
        ReceiverIntent::Source(value) => {
            current_matches(&state.main_zone.source, |item| item == value)
        }
        ReceiverIntent::Volume(value) => {
            current_matches(&state.main_zone.volume, |item| item == value)
        }
        ReceiverIntent::Mute(value) => current_matches(&state.main_zone.mute, |item| item == value),
        ReceiverIntent::SoundMode(mode) => current_matches(&state.main_zone.sound_mode, |item| {
            sound_mode_matches(&item.id, mode)
        }),
    }
}

fn current_matches<T>(
    field: &denon_avr_domain::ReceiverFieldState<T>,
    predicate: impl FnOnce(&T) -> bool,
) -> bool {
    matches!(
        field.validity,
        denon_avr_domain::ReceiverFieldValidity::Current { .. }
    ) && field
        .last_good
        .as_ref()
        .is_some_and(|item| predicate(&item.value))
}

fn sound_mode_matches(id: &str, intent: &denon_avr_domain::SoundModeIntent) -> bool {
    let expected = match intent {
        denon_avr_domain::SoundModeIntent::Auto => "AUTO",
        denon_avr_domain::SoundModeIntent::Direct => "DIRECT",
        denon_avr_domain::SoundModeIntent::PureDirect => "PURE DIRECT",
        denon_avr_domain::SoundModeIntent::Stereo => "STEREO",
        denon_avr_domain::SoundModeIntent::RecallMovie => {
            return denon_avr_domain::x3800h_sound_mode_in_category(
                id,
                denon_avr_domain::SoundModeCategory::Movie,
            )
        }
        denon_avr_domain::SoundModeIntent::RecallMusic => {
            return denon_avr_domain::x3800h_sound_mode_in_category(
                id,
                denon_avr_domain::SoundModeCategory::Music,
            )
        }
        denon_avr_domain::SoundModeIntent::RecallGame => {
            return denon_avr_domain::x3800h_sound_mode_in_category(
                id,
                denon_avr_domain::SoundModeCategory::Game,
            )
        }
        denon_avr_domain::SoundModeIntent::Select(value) => value,
    };
    id.eq_ignore_ascii_case(expected)
}

fn stopped() -> OperationError {
    OperationError::new(
        denon_avr_application::ports::OperationErrorKind::Stopped,
        "receiver session",
        "session actor stopped",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use denon_avr_application::CanonicalReceiverSession;
    use denon_avr_domain::{MasterVolume, OperationId};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    #[test]
    fn unsupported_source_and_arbitrary_sound_mode_writes_are_rejected() {
        assert!(admit_intent(&ReceiverIntent::Source(
            denon_avr_domain::SourceId::new("NOT-A-SOURCE").unwrap()
        ))
        .is_err());
        assert!(admit_intent(&ReceiverIntent::Source(
            denon_avr_domain::SourceId::new("CD").unwrap()
        ))
        .is_ok());
        assert!(admit_intent(&ReceiverIntent::SoundMode(
            denon_avr_domain::SoundModeIntent::Select("UNVERIFIED".into())
        ))
        .is_err());
        assert!(admit_intent(&ReceiverIntent::SoundMode(
            denon_avr_domain::SoundModeIntent::Select("DOLBY SURROUND".into())
        ))
        .is_ok());
    }

    #[test]
    fn initial_connection_errors_include_receiver_access_guidance() {
        let message = initial_connection_guidance("connection refused");
        assert!(message.contains("connection refused"));
        assert!(message.contains("Network Control"));
        assert!(message.contains("Telnet client"));
    }

    #[test]
    fn fairness_budget_forces_due_reconciliation_by_count_or_time() {
        let receiver = ReceiverId::new("fairness").unwrap();
        let debt = Some(SyncDebt::new(
            receiver,
            Epoch(1),
            SyncCycleId(1),
            CoreField::Volume,
            SyncCause::ReceiverEvent,
            MonotonicMillis(0),
        ));
        let started = Instant::now() - std::time::Duration::from_secs(1);
        let recent = Instant::now();
        assert!(!fairness_budget_due(&debt, started, recent, 7));
        assert!(fairness_budget_due(&debt, started, recent, 8));
        assert!(fairness_budget_due(
            &debt,
            started,
            recent - MAX_USER_WORK_WINDOW,
            0
        ));
    }

    #[tokio::test]
    async fn cancelled_operation_is_rejected_before_transport_or_debt_creation() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let _ = listener.accept().await;
            std::future::pending::<()>().await;
        });
        let mut avr = AvrSession::connect_addr(address, AvrSessionConfig::default())
            .await
            .unwrap();
        let receiver = ReceiverId::new("cancelled").unwrap();
        let mut initial = ReceiverState::new(receiver);
        initial.establish_epoch(Epoch(1));
        let (states, _) = watch::channel(initial);
        let (reply, result) = oneshot::channel();
        drop(result);
        let mut debt = None;
        let outcome = operate(
            &mut avr,
            OperationId(99),
            ReceiverIntent::Mute(denon_avr_domain::MuteState::On),
            &reply,
            &states,
            &mut debt,
            &mut Epoch(1),
            &mut FrameSeq(0),
            SyncCycleId(1),
            Instant::now(),
        )
        .await;
        assert!(matches!(
            outcome,
            OperationOutcome::Cancelled {
                operation: OperationId(99)
            }
        ));
        assert!(debt.is_none());
        <AvrSession as ReceiverSession>::close(&mut avr)
            .await
            .unwrap();
        server.abort();
    }

    #[test]
    fn detailed_sound_mode_observation_matches_exact_and_category_intents() {
        let id = ReceiverId::new("living-room").unwrap();
        let mut state = ReceiverState::new(id.clone());
        state.establish_epoch(Epoch(1));
        state.reduce(
            &id,
            Epoch(1),
            FrameSeq(1),
            MonotonicMillis(0),
            MonotonicMillis(100),
            ObservationOrigin::ReceiverFrame,
            CoreFrame::SoundMode(denon_avr_domain::SoundModeStatus {
                id: "STEREO".into(),
                raw: "STEREO".into(),
            }),
        );
        assert!(state_matches(
            &state,
            &ReceiverIntent::SoundMode(denon_avr_domain::SoundModeIntent::Stereo)
        ));
        assert!(!state_matches(
            &state,
            &ReceiverIntent::SoundMode(denon_avr_domain::SoundModeIntent::Direct)
        ));
        assert!(state_matches(
            &state,
            &ReceiverIntent::SoundMode(denon_avr_domain::SoundModeIntent::RecallMusic)
        ));
        assert!(!state_matches(
            &state,
            &ReceiverIntent::SoundMode(denon_avr_domain::SoundModeIntent::RecallMovie)
        ));
    }

    #[test]
    fn malformed_and_unknown_frames_are_diagnostic_only() {
        let id = ReceiverId::new("diagnostic").unwrap();
        let mut state = ReceiverState::new(id.clone());
        state.establish_epoch(Epoch(1));
        let (states, _) = watch::channel(state);
        let mut sequence = FrameSeq(0);
        reduce_line(
            &states,
            Epoch(1),
            &mut sequence,
            Instant::now(),
            "ZZUNKNOWN",
            false,
        );
        reduce_line(
            &states,
            Epoch(1),
            &mut sequence,
            Instant::now(),
            "MV985",
            false,
        );
        assert_eq!(states.borrow().diagnostics.len(), 2);
        assert_eq!(sequence, FrameSeq(2));
        assert!(states.borrow().main_zone.volume.last_good.is_none());
    }

    #[test]
    fn reconnect_lifecycle_installs_new_epoch_before_accepting_frames() {
        let id = ReceiverId::new("living-room").unwrap();
        let mut state = ReceiverState::new(id);
        state.establish_epoch(Epoch(1));
        let (states, _) = watch::channel(state);
        let mut epoch = Epoch(1);
        let mut sequence = FrameSeq(0);
        apply_event(
            AvrSessionEvent::Reconnected,
            &states,
            &mut epoch,
            &mut sequence,
            Instant::now(),
        );
        assert_eq!(epoch, Epoch(2));
        assert_eq!(states.borrow().epoch, Some(Epoch(2)));
    }

    #[tokio::test]
    async fn loopback_session_publishes_one_canonical_state_and_confirms_targeted_control() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            let mut line = Vec::new();
            let mut volume = "MV80";
            loop {
                line.clear();
                if stream.read_until(b'\r', &mut line).await.unwrap() == 0 {
                    break;
                }
                let command = std::str::from_utf8(&line).unwrap();
                let response = match command {
                    "PW?\r" => "PWON\r",
                    "ZM?\r" => "ZMON\r",
                    "Z2?\r" => "Z2OFF\r",
                    "SI?\r" => "SICD\r",
                    "MV?\r" => {
                        if volume == "MV80" {
                            "MV80\r"
                        } else {
                            "MV41\r"
                        }
                    }
                    "MU?\r" => "MUOFF\r",
                    "MS?\r" => "MSSTEREO\r",
                    "MV41\r" => {
                        volume = "MV41";
                        continue;
                    }
                    unexpected => panic!("unexpected command {unexpected:?}"),
                };
                stream
                    .get_mut()
                    .write_all(response.as_bytes())
                    .await
                    .unwrap();
            }
        });
        let receiver = ReceiverId::new("loopback-x3800h").unwrap();
        let session = X3800hSession::connect_addr(
            receiver,
            address,
            AvrSessionConfig {
                transmission_interval: std::time::Duration::from_millis(1),
                ..AvrSessionConfig::default()
            },
        )
        .await
        .unwrap();
        let readiness = session.synchronize().await.unwrap();
        assert!(readiness.ready, "{readiness:?}");
        session.observe(CoreField::Volume).await.unwrap();
        assert!(matches!(
            session.current_state().main_zone.volume.synchronization,
            denon_avr_domain::FieldSynchronization::Settled { .. }
        ));
        let subscription = session.state();
        assert_eq!(
            subscription
                .latest()
                .main_zone
                .power
                .last_good
                .unwrap()
                .value,
            denon_avr_domain::ZonePower::On
        );
        let outcome = session
            .operate(OperationRequest {
                id: OperationId(1),
                intent: ReceiverIntent::Volume(MasterVolume::db_half_steps(-78).unwrap()),
            })
            .await;
        assert!(
            matches!(
                outcome,
                OperationOutcome::ObservedRequestedValue {
                    operation: OperationId(1),
                    ..
                }
            ),
            "{outcome:?}"
        );
        assert_eq!(
            subscription
                .latest()
                .main_zone
                .volume
                .last_good
                .unwrap()
                .value,
            MasterVolume::db_half_steps(-78).unwrap()
        );
        session.close().await.unwrap();
        session.close().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn dropped_operation_requester_does_not_replay_or_rollback_receiver_state() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (dispatched_tx, dispatched_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            let mut line = Vec::new();
            let mut mutating_writes = 0_u8;
            let mut dispatched_tx = Some(dispatched_tx);
            loop {
                line.clear();
                if stream.read_until(b'\r', &mut line).await.unwrap() == 0 {
                    break;
                }
                let command = std::str::from_utf8(&line).unwrap();
                if command == "MV205\r" {
                    mutating_writes += 1;
                    if let Some(sender) = dispatched_tx.take() {
                        let _ = sender.send(());
                    }
                    continue;
                }
                let response = match command {
                    "PW?\r" => "PWON\r",
                    "ZM?\r" => "ZMON\r",
                    "Z2?\r" => "Z2OFF\r",
                    "SI?\r" => "SICD\r",
                    "MV?\r" if mutating_writes == 0 => "MV80\r",
                    "MV?\r" => "MV205\r",
                    "MU?\r" => "MUOFF\r",
                    "MS?\r" => "MSSTEREO\r",
                    unexpected => panic!("unexpected command {unexpected:?}"),
                };
                stream
                    .get_mut()
                    .write_all(response.as_bytes())
                    .await
                    .unwrap();
            }
            assert_eq!(mutating_writes, 1);
        });
        let receiver = ReceiverId::new("dropped-requester").unwrap();
        let session = X3800hSession::connect_addr(
            receiver,
            address,
            AvrSessionConfig {
                transmission_interval: std::time::Duration::from_millis(1),
                ..AvrSessionConfig::default()
            },
        )
        .await
        .unwrap();
        let operation = tokio::spawn({
            let session = Arc::clone(&session);
            async move {
                session
                    .operate(OperationRequest {
                        id: OperationId(44),
                        intent: ReceiverIntent::Volume(MasterVolume::db_half_steps(-119).unwrap()),
                    })
                    .await
            }
        });
        dispatched_rx.await.unwrap();
        operation.abort();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(
            session
                .current_state()
                .main_zone
                .volume
                .last_good
                .as_ref()
                .unwrap()
                .value,
            MasterVolume::db_half_steps(-119).unwrap()
        );
        session.close().await.unwrap();
        server.await.unwrap();
    }
}
