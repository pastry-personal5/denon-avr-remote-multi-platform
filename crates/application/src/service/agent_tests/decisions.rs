//! Decisions, policy availability, dry runs, the caps, and health.

use super::*;
use crate::control::{
    IdempotencyKey, OperationControl, OperationSubmission, OperatorAdmin, ReceiverReads,
    ServiceHealth,
};

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
    assert!(rules.is_empty(), "an agent is not shown rule ids");
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
    assert_eq!(record.principal, Some(Principal::Operator));
    assert_eq!(record.run, h.clock.now());
}
