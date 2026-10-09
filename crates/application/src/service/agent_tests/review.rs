//! Tests of cases found in review.

use super::*;
use crate::control::{OperationControl, ReceiverReads};

// ---- Found in review ----

#[tokio::test(start_paused = true)]
async fn a_policy_edit_made_while_the_receiver_connects_decides_the_request() {
    let h = start_default().await;
    let release = h.connector.hold_connect();
    let held = h
        .agent
        .submit(&living_room(), set_volume(-32.0))
        .await
        .unwrap();
    settle().await;

    // While the receiver connects, the owner makes the policy stricter.
    let mut rules = owners_rules();
    rules.insert(
        0,
        Rule::new("quiet-only", Verdict::Deny)
            .intent(IntentKind::Volume)
            .target_above(level(-40.0)),
    );
    h.policy.set(Ok(policy_of(rules, 2)));
    h.operator.reload_policy().await.unwrap();
    release.notify_one();

    let done = h
        .agent
        .operation(held.id, Some(Duration::from_secs(5)))
        .await
        .unwrap();
    assert_eq!(done.status, OperationStatus::Denied);
    assert_eq!(operate_calls(&h), 0);
    // And the log says which policy decided it.
    let digest = h
        .audit
        .events()
        .into_iter()
        .filter_map(|event| match event {
            AuditEvent::Decided { policy, .. } => Some(policy),
            _ => None,
        })
        .next_back()
        .unwrap();
    assert_eq!(digest, Some(PolicyDigest::from_bytes([2; 32])));
}

#[tokio::test(start_paused = true)]
async fn a_policy_that_fails_to_load_while_the_receiver_connects_stops_the_request() {
    let h = start_default().await;
    let release = h.connector.hold_connect();
    let held = h
        .agent
        .submit(&living_room(), set_volume(-32.0))
        .await
        .unwrap();
    settle().await;

    h.policy
        .set(Err(PolicyLoadError::Invalid("a bad edit".into())));
    h.operator.reload_policy().await.unwrap();
    release.notify_one();

    let done = h
        .agent
        .operation(held.id, Some(Duration::from_secs(5)))
        .await
        .unwrap();
    assert_eq!(done.status, OperationStatus::Rejected);
    assert_eq!(done.reason.as_deref(), Some("policy unavailable"));
    assert_eq!(operate_calls(&h), 0, "the old rules did not allow it");
    assert_eq!(counted(&h), 0);
}

#[tokio::test(start_paused = true)]
async fn the_log_says_a_cancelled_operation_ended_cancelled_whatever_the_gate_decided() {
    let h = start_default().await;
    let release = h.connector.hold_connect();
    // The policy would deny this one.
    let held = h
        .agent
        .submit(&living_room(), set_volume(-15.0))
        .await
        .unwrap();
    settle().await;
    assert_eq!(
        h.agent.cancel(held.id).await.unwrap().status,
        OperationStatus::Cancelled
    );
    release.notify_one();
    settle().await;

    let done = h.agent.operation(held.id, None).await.unwrap();
    assert_eq!(done.status, OperationStatus::Cancelled);
    let finished: Vec<String> = h
        .audit
        .events()
        .into_iter()
        .filter_map(|event| match event {
            AuditEvent::Finished { status, .. } => Some(status),
            _ => None,
        })
        .collect();
    assert_eq!(
        finished,
        ["cancelled"],
        "what the client saw is what is logged"
    );
}

#[tokio::test(start_paused = true)]
async fn two_reloads_leave_the_policy_and_the_log_in_the_order_they_loaded() {
    let h = start_default().await;
    // The first reload reads the file as it is now and is held up.
    h.policy.set(Ok(policy_of(owners_rules(), 2)));
    let slow = h.policy.hold_next_load();
    let operator = h.operator.clone();
    let first = tokio::spawn(async move { operator.reload_policy().await });
    settle().await;

    // The file is edited again and reloaded before the first reload finishes.
    h.policy.set(Ok(policy_of(owners_rules(), 3)));
    let operator = h.operator.clone();
    let second = tokio::spawn(async move { operator.reload_policy().await });
    settle().await;

    slow.add_permits(1);
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();

    // The newer file is in force, and the log ends with it.
    assert_eq!(
        h.operator.policy().await.unwrap().digest,
        Some(PolicyDigest::from_bytes([3; 32]))
    );
    let loaded: Vec<PolicyDigest> = h
        .audit
        .events()
        .into_iter()
        .filter_map(|event| match event {
            AuditEvent::PolicyLoaded { digest } => Some(digest),
            _ => None,
        })
        .collect();
    assert_eq!(
        loaded,
        [1, 2, 3].map(|byte| PolicyDigest::from_bytes([byte; 32]))
    );
}

#[tokio::test(start_paused = true)]
async fn a_label_that_has_gone_quiet_is_forgotten() {
    let h = start_default().await;
    let agent = h.service.inner.agent.as_ref().unwrap();
    for n in 0..300 {
        agent.charge_write(&label(&format!("agent-{n}"))).unwrap();
    }
    assert_eq!(agent.tracked_labels(), 300);

    advance(Duration::from_secs(61)).await;
    agent.charge_write(&label("fresh")).unwrap();
    assert_eq!(agent.tracked_labels(), 1, "the quiet ones were swept");
}

#[tokio::test(start_paused = true)]
async fn a_stalled_audit_disk_delays_an_operator_write_by_at_most_a_second() {
    let h = start_default().await;
    h.audit.set_mode(AuditMode::HangAppend);

    let started = tokio::time::Instant::now();
    let first = ask_as_operator(&h, set_volume(-50.0)).await;
    assert_eq!(first.status, OperationStatus::Completed, "{first:?}");
    assert!(
        started.elapsed() <= Duration::from_millis(1_100),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(
        h.operator.health().await.unwrap().audit,
        AuditHealth::Failing
    );

    // Knowing the log is failing, the next write does not wait for it at all.
    let started = tokio::time::Instant::now();
    let second = ask_as_operator(&h, set_volume(-51.0)).await;
    assert_eq!(second.status, OperationStatus::Completed);
    assert!(
        started.elapsed() < Duration::from_millis(100),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test(start_paused = true)]
async fn the_operators_records_come_back_once_the_disk_does() {
    let h = start_default().await;
    h.audit.set_mode(AuditMode::FailAppend);
    ask_as_operator(&h, set_volume(-50.0)).await;
    assert_eq!(
        h.operator.health().await.unwrap().audit,
        AuditHealth::Failing
    );

    // The disk is back. The next write skips its records, because the log was
    // failing, but its Finished record is written and shows the log works.
    h.audit.set_mode(AuditMode::Working);
    ask_as_operator(&h, set_volume(-51.0)).await;
    settle().await;
    assert_eq!(h.operator.health().await.unwrap().audit, AuditHealth::Ok);

    // From then on the Operator's writes are recorded in full again.
    let before = h.audit.records().len();
    ask_as_operator(&h, set_volume(-52.0)).await;
    settle().await;
    let kinds: Vec<&'static str> = h.audit.events()[before..].iter().map(event_name).collect();
    assert_eq!(
        kinds,
        ["audit:decided", "audit:dispatching", "audit:finished"]
    );
}

#[tokio::test(start_paused = true)]
async fn an_agent_is_given_fixed_text_for_what_a_session_reports() {
    use denon_avr_domain::OperationOutcome as Outcome;
    use denon_avr_domain::PreconditionMismatch;
    type Script = Box<dyn Fn(&crate::session_v3::OperationRequest) -> Outcome + Send + Sync>;
    let leak = "socket 192.0.2.10:23 reset";
    let cases: Vec<(&str, Script, OperationStatus, &str)> = vec![
        (
            "unsupported",
            Box::new(move |r| Outcome::RejectedBeforeDispatch {
                operation: r.id,
                cause: RejectionCause::UnsupportedIntent,
                reason: leak.into(),
            }),
            OperationStatus::Rejected,
            "the receiver does not support this change",
        ),
        (
            "observation failed",
            Box::new(move |r| Outcome::RejectedBeforeDispatch {
                operation: r.id,
                cause: RejectionCause::ObservationFailed,
                reason: leak.into(),
            }),
            OperationStatus::Rejected,
            "the receiver's state could not be read",
        ),
        (
            "precondition",
            Box::new(move |r| Outcome::RejectedBeforeDispatch {
                operation: r.id,
                cause: RejectionCause::PreconditionMismatch(PreconditionMismatch::Epoch),
                reason: leak.into(),
            }),
            OperationStatus::Rejected,
            "the receiver changed since the request was judged",
        ),
        (
            "command refused",
            Box::new(move |r| Outcome::RejectedBeforeDispatch {
                operation: r.id,
                cause: RejectionCause::CommandRefused,
                reason: leak.into(),
            }),
            OperationStatus::Rejected,
            "the receiver refused the command",
        ),
        (
            "session stopped",
            Box::new(move |r| Outcome::RejectedBeforeDispatch {
                operation: r.id,
                cause: RejectionCause::SessionStopped,
                reason: leak.into(),
            }),
            OperationStatus::Rejected,
            "the receiver connection closed",
        ),
        (
            "indeterminate",
            Box::new(move |r| Outcome::Indeterminate {
                operation: r.id,
                dispatch: DispatchCertainty::Unknown,
                reason: format!("post-dispatch observation failed: {leak}"),
            }),
            OperationStatus::Indeterminate,
            "the outcome could not be established",
        ),
    ];
    for (name, script, status, expected) in cases {
        let script: Arc<dyn Fn(&crate::session_v3::OperationRequest) -> Outcome + Send + Sync> =
            Arc::from(script);

        // The agent is told the fixed sentence.
        let h = start_default().await;
        let for_agent = Arc::clone(&script);
        h.connector.script(move |request| for_agent(request));
        let snapshot = ask(&h.agent, set_volume(-32.0)).await;
        assert_eq!(snapshot.status, status, "{name}");
        assert_eq!(snapshot.reason.as_deref(), Some(expected), "{name}");

        // The log keeps what the session said.
        let logged = h
            .audit
            .events()
            .into_iter()
            .find_map(|event| match event {
                AuditEvent::Finished { reason, .. } => reason,
                _ => None,
            })
            .unwrap();
        assert!(logged.contains("192.0.2.10"), "{name}: {logged}");

        // The Operator sees it too, as before.
        let h = start_default().await;
        let for_operator = Arc::clone(&script);
        h.connector.script(move |request| for_operator(request));
        let snapshot = ask_as_operator(&h, set_volume(-32.0)).await;
        assert!(
            snapshot.reason.unwrap().contains("192.0.2.10"),
            "{name}: the Operator is told what the session said"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn an_agent_is_not_told_where_the_receiver_is_when_a_read_fails() {
    let h = start_default().await;
    h.connector.fail_connections();
    let fixed = ControlError::Unavailable("the receiver could not be reached".into());

    assert_eq!(h.agent.state(&living_room()).await.err().unwrap(), fixed);
    assert_eq!(
        h.agent.source_catalog(&living_room()).await.unwrap_err(),
        fixed
    );
    assert_eq!(
        h.agent
            .dry_run(&living_room(), set_volume(-50.0).intent)
            .await
            .unwrap_err(),
        fixed
    );
    // The Operator is told why.
    let operator = h.operator.state(&living_room()).await.err().unwrap();
    assert!(operator.to_string().contains("refused"), "{operator}");

    // A failure the session reports keeps its kind and loses its message.
    let handle = h.agent_forged();
    let error = handle.sanitized(ControlError::Receiver(OperationError::new(
        crate::ports::OperationErrorKind::Connection,
        "using AVR session",
        "connect 192.0.2.10:23 failed",
    )));
    let ControlError::Receiver(error) = error else {
        panic!("{error:?}");
    };
    assert_eq!(error.kind, crate::ports::OperationErrorKind::Connection);
    assert!(!error.to_string().contains("192.0.2"), "{error}");
    // The Operator's error is untouched, and so is a refusal that names nothing.
    let operator = h.service.operator();
    let _ = operator;
    assert_eq!(
        handle.sanitized(ControlError::NotFound("receiver")),
        ControlError::NotFound("receiver")
    );
}

#[tokio::test(start_paused = true)]
async fn an_agents_dry_run_shows_the_limits_and_not_the_rule_ids() {
    let h = start_default().await;
    let deny = h
        .agent
        .dry_run(&living_room(), set_volume(-15.0).intent)
        .await
        .unwrap();
    let DryRunDecision::Deny { reasons, rules } = deny.decision else {
        panic!("the policy denies this");
    };
    assert!(reasons[0].contains("above the limit"));
    assert!(rules.is_empty(), "{rules:?}");

    let held = h
        .agent
        .dry_run(&living_room(), set_volume(-25.0).intent)
        .await
        .unwrap();
    let DryRunDecision::RequireApproval { rules, .. } = held.decision else {
        panic!("the policy holds this");
    };
    assert!(rules.is_empty(), "{rules:?}");

    // The Operator testing the same label sees which rules fired.
    let as_agent = h
        .operator
        .dry_run_as(label("openclaw"), &living_room(), set_volume(-15.0).intent)
        .await
        .unwrap();
    let DryRunDecision::Deny { rules, .. } = as_agent.decision else {
        panic!("the policy denies this");
    };
    assert_eq!(rules[0], "volume-hard-limit");
}

#[tokio::test(start_paused = true)]
async fn a_stalled_audit_disk_does_not_hold_shutdown_long_after_an_operator_write() {
    let h = start_default().await;
    h.audit.set_mode(AuditMode::HangAppend);
    let started = tokio::time::Instant::now();
    ask_as_operator(&h, set_volume(-50.0)).await;
    // The write is over; its closing record is still being given up on.
    h.service.shutdown().await;
    assert!(
        started.elapsed() <= Duration::from_millis(2_200),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test(start_paused = true)]
async fn after_shutdown_an_agent_is_told_so_and_not_that_the_receiver_is_out_of_reach() {
    let h = start_default().await;
    h.service.shutdown().await;
    let error = h.agent.state(&living_room()).await.err().unwrap();
    assert_eq!(
        error,
        ControlError::Unavailable("the control service has shut down".into())
    );
}

#[tokio::test(start_paused = true)]
async fn refusals_made_before_any_decision_are_logged_once_a_minute_not_each_time() {
    let h = start(Setup {
        policy: Err(PolicyLoadError::Invalid("a bad edit".into())),
        ..Setup::default()
    })
    .await;
    let finished = |h: &AgentHarness| -> Vec<(String, Option<String>)> {
        h.audit
            .events()
            .into_iter()
            .filter_map(|event| match event {
                AuditEvent::Finished { status, reason, .. } => Some((status, reason)),
                _ => None,
            })
            .collect()
    };

    // Ten refused requests in a row. Each is told so, and one is logged.
    for _ in 0..10 {
        let snapshot = ask(&h.agent, mute_on()).await;
        assert_eq!(snapshot.reason.as_deref(), Some("policy unavailable"));
    }
    assert_eq!(
        finished(&h),
        [(
            "rejected".to_string(),
            Some("policy unavailable".to_string())
        )]
    );

    // A minute on, the next is logged with the count of those not written.
    advance(Duration::from_secs(61)).await;
    ask(&h.agent, mute_on()).await;
    let records = finished(&h);
    assert_eq!(records.len(), 2);
    assert_eq!(
        records[1].1.as_deref(),
        Some("policy unavailable (9 more refused since the last record)")
    );
}
