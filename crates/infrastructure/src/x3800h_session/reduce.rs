//! The reducer: queries and receiver events folded into the one canonical
//! state, with the debt and synchronization markers that track them.

use crate::avr_session::{AvrSession, AvrSessionEvent};
use denon_avr_application::ports::OperationError;
use denon_avr_application::Readiness;
use denon_avr_domain::{
    CoreField, CoreFrame, Epoch, FrameSeq, MonotonicMillis, ObservationOrigin, ReceiverIntent,
    ReceiverState, SyncCause, SyncCycleId, SyncDebt,
};
use denon_avr_protocol::avr::{parse_x3800h, x3800h_query, X3800hFrame};
use std::time::Instant;
use tokio::sync::watch;
use tracing::{debug, info, warn};

/// The published canonical state and what stamps every observation reduced
/// into it: the connection epoch, the frame sequence, and the clock origin.
pub(super) struct Feed {
    pub(super) states: watch::Sender<ReceiverState>,
    pub(super) epoch: Epoch,
    pub(super) frame_seq: FrameSeq,
    pub(super) started: Instant,
}

pub(super) async fn query_and_reduce(
    avr: &AvrSession,
    intent: &ReceiverIntent,
    feed: &mut Feed,
) -> Result<(), OperationError> {
    synchronize_generation(avr, feed);
    let line = avr
        .request(x3800h_query(intent).as_str())
        .await
        .map_err(OperationError::from)?;
    synchronize_generation(avr, feed);
    reduce_line(
        &feed.states,
        feed.epoch,
        &mut feed.frame_seq,
        feed.started,
        &line,
        true,
    );
    Ok(())
}

/// The low-level socket actor may reconnect while an outstanding query is
/// resolved. Bind every reduced response to its actual socket generation even
/// when lifecycle notifications are still queued behind that response.
fn synchronize_generation(avr: &AvrSession, feed: &mut Feed) {
    let observed = Epoch(avr.connection_generation().saturating_add(1));
    if observed == feed.epoch {
        return;
    }
    let mut state = feed.states.borrow().clone();
    if state.epoch.is_some() {
        state.mark_disconnected();
    }
    state.establish_epoch(observed);
    feed.states.send_replace(state);
    feed.epoch = observed;
}

pub(super) fn apply_event(event: AvrSessionEvent, feed: &mut Feed) -> Option<CoreField> {
    match event {
        AvrSessionEvent::Line(line) => {
            return reduce_line(
                &feed.states,
                feed.epoch,
                &mut feed.frame_seq,
                feed.started,
                &line,
                false,
            )
        }
        AvrSessionEvent::Disconnected {
            generation,
            message,
        } => {
            warn!(generation, %message, "receiver transport disconnected");
            // A reconnect may finish while this lifecycle notification is in
            // the mailbox. Never let old-connection failure evidence make a
            // newer epoch stale.
            if generation.saturating_add(1) < feed.epoch.0 {
                return None;
            }
            let mut state = feed.states.borrow().clone();
            state.mark_disconnected();
            feed.states.send_replace(state);
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
            feed.epoch.0 = feed.epoch.0.saturating_add(1);
            let mut state = feed.states.borrow().clone();
            // Lifecycle notifications can be reordered relative to the
            // actor's command mailbox. Always retire the prior epoch before
            // installing the newly established socket epoch.
            state.mark_disconnected();
            state.establish_epoch(feed.epoch);
            feed.states.send_replace(state);
        }
    }
    None
}

pub(super) fn reduce_line(
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

pub(super) fn merge_debt(
    debt: &mut Option<SyncDebt>,
    feed: &Feed,
    field: CoreField,
    cause: SyncCause,
    cycle: SyncCycleId,
) {
    let state = feed.states.borrow();
    let Some(epoch) = state.epoch else { return };
    let receiver = state.receiver.clone();
    let now = MonotonicMillis(feed.started.elapsed().as_millis() as u64);
    match debt {
        Some(existing) if existing.is_for(&receiver, epoch) => {
            existing.merge([field], cause);
            existing.note_activity(now);
        }
        _ => *debt = Some(SyncDebt::new(receiver, epoch, cycle, field, cause, now)),
    }
}

pub(super) fn clear_settled_debt(
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

pub(super) fn mark_converging(feed: &Feed, field: CoreField, cause: SyncCause, cycle: SyncCycleId) {
    let mut state = feed.states.borrow().clone();
    let now = MonotonicMillis(feed.started.elapsed().as_millis() as u64);
    state.mark_converging(field, cycle, cause, MonotonicMillis(now.0 + 250));
    feed.states.send_replace(state);
}

pub(super) fn mark_settled(feed: &Feed, field: CoreField, cycle: SyncCycleId) {
    let mut state = feed.states.borrow().clone();
    state.mark_settled(
        field,
        cycle,
        MonotonicMillis(feed.started.elapsed().as_millis() as u64),
    );
    feed.states.send_replace(state);
}

pub(super) fn mark_query_failed(feed: &Feed, field: CoreField, message: String) {
    warn!(?field, %message, "canonical receiver observation failed");
    let mut state = feed.states.borrow().clone();
    state.mark_query_failed(field, message);
    feed.states.send_replace(state);
}
