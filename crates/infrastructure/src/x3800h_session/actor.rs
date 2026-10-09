//! The session actor: the loop that owns the receiver connection, services
//! reconciliation debt, and runs the periodic synchronization sweep.

use super::operate::{dependency_fields, operate};
use super::reduce::{
    apply_event, clear_settled_debt, mark_converging, mark_query_failed, mark_settled, merge_debt,
    query_and_reduce, Feed,
};
use super::{
    core_intents, intent_for_field, Command, MAX_USER_ACTIONS_BEFORE_DEBT, MAX_USER_WORK_WINDOW,
};
use crate::avr_session::{AvrSession, AvrSessionEvent};
use denon_avr_application::ports::OperationError;
use denon_avr_application::Readiness;
use denon_avr_domain::{
    Epoch, FrameSeq, MonotonicMillis, ReceiverState, SyncCause, SyncCycleId, SyncDebt,
};
use std::time::Instant;
use tokio::sync::{mpsc, watch};
use tracing::debug;

pub(super) async fn run_actor(
    mut avr: AvrSession,
    mut commands: mpsc::Receiver<Command>,
    states: watch::Sender<ReceiverState>,
) {
    debug!("X3800H canonical session actor started");
    let started = Instant::now();
    let mut feed = Feed {
        states,
        epoch: Epoch(1),
        frame_seq: FrameSeq(0),
        started,
    };
    let mut cycle = SyncCycleId(0);
    let mut debt: Option<SyncDebt> = None;
    let mut user_work_started = Instant::now();
    let mut user_action_count = 0_u8;
    // Socket establishment enters Synchronizing immediately. Consumers may
    // still request an explicit synchronize; that request simply performs a
    // fresh deterministic pass against the same canonical state.
    cycle.0 = 1;
    let readiness = synchronize(&avr, &mut feed, cycle).await;
    clear_settled_debt(&mut debt, &readiness, feed.started);
    let mut sweep = tokio::time::interval(std::time::Duration::from_secs(5));
    let mut debt_tick = tokio::time::interval(std::time::Duration::from_millis(50));
    // Consume interval's immediate first tick; the first sweep is due five
    // seconds after actor startup, not during initial synchronization.
    sweep.tick().await;
    loop {
        if fairness_budget_due(&debt, feed.started, user_work_started, user_action_count) {
            service_debt(&avr, &mut feed, &mut debt, cycle).await;
            user_work_started = Instant::now();
            user_action_count = 0;
            continue;
        }
        tokio::select! {
            _ = debt_tick.tick() => {
                service_debt(&avr, &mut feed, &mut debt, cycle).await;
            }
            _ = sweep.tick() => {
                expire_state(&feed);
                cycle.0 = cycle.0.saturating_add(1);
                let readiness = synchronize(&avr, &mut feed, cycle).await;
                clear_settled_debt(&mut debt, &readiness, feed.started);
            }
            event = avr.next_event() => match event {
                Some(event) => {
                    let reconnected = matches!(event, AvrSessionEvent::Reconnected);
                    if let Some(field) = apply_event(event, &mut feed) {
                        merge_debt(&mut debt, &feed, field, SyncCause::ReceiverEvent, cycle);
                    }
                    if reconnected {
                        // A new connection starts with no evidence: every
                        // field was marked stale when the old one went. Read
                        // them all now, which only queries, instead of
                        // leaving every client to wait for the next sweep.
                        cycle.0 = cycle.0.saturating_add(1);
                        let readiness = synchronize(&avr, &mut feed, cycle).await;
                        clear_settled_debt(&mut debt, &readiness, feed.started);
                    }
                },
                None => break,
            },
            command = commands.recv() => match command {
                Some(Command::Observe(field, reply)) => {
                    user_action_count = user_action_count.saturating_add(1);
                    cycle.0 = cycle.0.saturating_add(1);
                    mark_converging(&feed, field, SyncCause::ManualRefresh, cycle);
                    let intent = intent_for_field(field);
                    let result = query_and_reduce(&avr, &intent, &mut feed).await;
                    match &result {
                        Ok(()) => mark_settled(&feed, field, cycle),
                        Err(error) => mark_query_failed(&feed, field, error.to_string()),
                    }
                    let _ = reply.send(result);
                }
                Some(Command::Synchronize(reply)) => {
                    user_action_count = user_action_count.saturating_add(1);
                    cycle.0 = cycle.0.saturating_add(1);
                    let result = synchronize(&avr, &mut feed, cycle).await;
                    clear_settled_debt(&mut debt, &result, feed.started);
                    let _ = reply.send(result);
                }
                Some(Command::Operate(request, reply)) => {
                    user_action_count = user_action_count.saturating_add(1);
                    cycle.0 = cycle.0.saturating_add(1);
                    if !reply.is_closed() {
                        for field in dependency_fields(request.intent.field()) {
                            merge_debt(&mut debt, &feed, *field, SyncCause::LocalControl, cycle);
                        }
                    }
                    let outcome = operate(
                        &mut avr,
                        &mut feed,
                        request,
                        &reply,
                        &mut debt,
                        cycle,
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
    feed: &mut Feed,
    debt: &mut Option<SyncDebt>,
    cycle: SyncCycleId,
) {
    let Some(current) = debt.as_ref() else { return };
    let now = MonotonicMillis(feed.started.elapsed().as_millis() as u64);
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
        match query_and_reduce(avr, &intent, feed).await {
            Ok(()) => {
                mark_settled(feed, field, cycle);
                if final_pass {
                    if let Some(value) = debt.as_mut() {
                        value.settle_field(field);
                    }
                }
            }
            Err(error) => {
                all_succeeded = false;
                mark_query_failed(feed, field, error.to_string());
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

pub(super) fn fairness_budget_due(
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
    feed: &mut Feed,
    cycle: SyncCycleId,
) -> Result<Readiness, OperationError> {
    let mut degraded = false;
    for intent in &core_intents() {
        mark_converging(feed, intent.field(), SyncCause::ManualRefresh, cycle);
        match query_and_reduce(avr, intent, feed).await {
            Ok(()) => mark_settled(feed, intent.field(), cycle),
            Err(error) => {
                mark_query_failed(feed, intent.field(), error.to_string());
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

fn expire_state(feed: &Feed) {
    let now = MonotonicMillis(feed.started.elapsed().as_millis() as u64);
    let mut state = feed.states.borrow().clone();
    if state.expire_fields(now) {
        feed.states.send_replace(state);
    }
}
