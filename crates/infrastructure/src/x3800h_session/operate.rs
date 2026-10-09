//! The operate path: intent admission, precondition re-observation, the write,
//! and the confirmation window, plus the state-matching helpers it relies on.

use super::reduce::{
    apply_event, mark_converging, mark_query_failed, mark_settled, query_and_reduce, Feed,
};
use super::{intent_for_field, CONTROL_OBSERVATION_WINDOW, CONTROL_RECHECK_INTERVAL};
use crate::avr_session::{AvrSession, AvrSessionError};
use denon_avr_application::OperationRequest;
use denon_avr_domain::{
    CoreField, DispatchCertainty, MonotonicMillis, OperationOutcome, Precondition,
    PreconditionMismatch, ReceiverIntent, ReceiverState, RejectionCause, SyncCause, SyncCycleId,
    SyncDebt,
};
use denon_avr_protocol::avr::encode_x3800h;
use tokio::sync::oneshot;

/// A refusal before anything is written. `cause` is what callers branch on.
fn reject(
    operation: denon_avr_domain::OperationId,
    cause: RejectionCause,
    reason: impl Into<String>,
) -> OperationOutcome {
    OperationOutcome::RejectedBeforeDispatch {
        operation,
        cause,
        reason: reason.into(),
    }
}

fn describe_mismatch(mismatch: PreconditionMismatch) -> String {
    match mismatch {
        PreconditionMismatch::Epoch => {
            "the receiver connection changed since the request was evaluated".into()
        }
        PreconditionMismatch::Field(field) => {
            format!("{field:?} no longer shows what the request was evaluated against")
        }
    }
}

/// Re-observe every field the precondition names besides the target, which the
/// preflight has just read, and compare them all with what the decision saw.
/// Only reads are issued. A field that cannot be re-observed refuses the write,
/// because the decision can no longer be checked.
async fn verify_precondition(
    avr: &AvrSession,
    precondition: &Precondition,
    target: CoreField,
    operation: denon_avr_domain::OperationId,
    feed: &mut Feed,
) -> Result<(), OperationOutcome> {
    for field in precondition.fields().filter(|field| *field != target) {
        let intent = intent_for_field(field);
        if let Err(error) = query_and_reduce(avr, &intent, feed).await {
            mark_query_failed(feed, field, error.to_string());
            return Err(reject(
                operation,
                RejectionCause::ObservationFailed,
                format!("could not re-observe {field:?} before writing: {error}"),
            ));
        }
    }
    match precondition.mismatch(&feed.states.borrow()) {
        None => Ok(()),
        Some(mismatch) => Err(reject(
            operation,
            RejectionCause::PreconditionMismatch(mismatch),
            describe_mismatch(mismatch),
        )),
    }
}

/// Run one operation. The order of the checks before the write is part of the
/// session contract: admit the intent, re-observe the target, re-observe and
/// compare the precondition when there is one, and only then ask whether the
/// target is already in state. A precondition mismatch therefore takes
/// precedence over `AlreadyObserved`, because the precondition guards the
/// decision and not only the write. Nothing is retried.
pub(super) async fn operate(
    avr: &mut AvrSession,
    feed: &mut Feed,
    request: OperationRequest,
    reply: &oneshot::Sender<OperationOutcome>,
    debt: &mut Option<SyncDebt>,
    cycle: SyncCycleId,
) -> OperationOutcome {
    let OperationRequest {
        id: operation,
        intent,
        precondition,
        ..
    } = request;
    let field = intent.field();
    let dependencies = dependency_fields(field);
    if reply.is_closed() {
        return OperationOutcome::Cancelled { operation };
    }
    if let Err(reason) = admit_intent(&intent) {
        return reject(operation, RejectionCause::UnsupportedIntent, reason);
    }
    for dependency in dependencies {
        mark_converging(feed, *dependency, SyncCause::LocalControl, cycle);
    }
    if let Err(error) = query_and_reduce(avr, &intent, feed).await {
        mark_query_failed(feed, field, error.to_string());
        return reject(
            operation,
            RejectionCause::ObservationFailed,
            format!("preflight failed: {error}"),
        );
    }
    if let Some(precondition) = &precondition {
        if let Err(outcome) = verify_precondition(avr, precondition, field, operation, feed).await {
            return outcome;
        }
    }
    if state_matches(&feed.states.borrow(), &intent) {
        for dependency in dependencies {
            mark_settled(feed, *dependency, cycle);
        }
        return OperationOutcome::AlreadyObserved {
            operation,
            observation: "targeted preflight observation".into(),
        };
    }
    if reply.is_closed() {
        return OperationOutcome::Cancelled { operation };
    }
    let command = match encode_x3800h(&intent) {
        Ok(command) => command,
        Err(error) => return reject(operation, RejectionCause::CommandRefused, error.to_string()),
    };
    if let Err(error) = avr.dispatch(command.as_str()).await {
        return match error {
            AvrSessionError::InvalidCommand(message) => reject(
                operation,
                RejectionCause::CommandRefused,
                format!("command was not dispatched: {message}"),
            ),
            AvrSessionError::SessionStopped => reject(
                operation,
                RejectionCause::SessionStopped,
                "session stopped before command dispatch",
            ),
            error => {
                // A failed write is never replayed. A read-only observation
                // may nevertheless prove that the receiver applied it.
                match query_and_reduce(avr, &intent, feed).await {
                    Ok(()) if state_matches(&feed.states.borrow(), &intent) => {
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
    let write_completed_at = MonotonicMillis(feed.started.elapsed().as_millis() as u64);
    if let Some(debt) = debt.as_mut() {
        debt.defer_final_until(MonotonicMillis(write_completed_at.0 + 5_000));
    }
    let deadline = tokio::time::Instant::now() + CONTROL_OBSERVATION_WINDOW;
    loop {
        match query_and_reduce(avr, &intent, feed).await {
            Ok(()) if state_matches(&feed.states.borrow(), &intent) => {
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
                        Some(event) => { let _ = apply_event(event, feed); }
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
                mark_query_failed(feed, field, error.to_string());
                break OperationOutcome::Indeterminate {
                    operation,
                    dispatch: DispatchCertainty::CompleteWrite,
                    reason: format!("post-dispatch observation failed: {error}"),
                };
            }
        }
    }
}

pub(super) fn dependency_fields(field: CoreField) -> &'static [CoreField] {
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
pub(super) fn admit_intent(intent: &ReceiverIntent) -> Result<(), String> {
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

pub(super) fn state_matches(state: &ReceiverState, intent: &ReceiverIntent) -> bool {
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
