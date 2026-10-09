//! Dispatch, the precondition, the ledger and budget, and cancel and settle.

use super::*;
use crate::control::{IdempotencyKey, OperationControl, OperationSubmission, ReceiverReads};
use std::sync::atomic::AtomicUsize;

// ---- Dispatch ----

pub(super) fn counted(h: &AgentHarness) -> usize {
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

pub(super) fn dispatching_record(
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
        principal: Some(principal),
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
        .filter(|record| {
            record.principal == Some(Principal::Operator) && record.operation.is_some()
        })
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
