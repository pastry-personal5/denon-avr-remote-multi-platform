//! Tokens and refused callers.

use super::*;
use crate::audit::{AccessRefusal, EndpointKind, RefusalReason};
use crate::control::{OperatorAdmin, ReceiverReads};
use crate::ports::BoxFuture;
use crate::tokens::{
    Credential, IssuedToken, TokenError, TokenId, TokenRecord, TokenSecret, TokenStore,
};

// ---- Tokens and refused callers ----

/// A token store in memory that can be made to fail.
struct FakeTokens {
    records: Mutex<Vec<TokenRecord>>,
    secrets: Mutex<Vec<String>>,
    failing: AtomicBool,
    revoked: watch::Sender<u64>,
}

impl FakeTokens {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            records: Mutex::new(Vec::new()),
            secrets: Mutex::new(Vec::new()),
            failing: AtomicBool::new(false),
            revoked: watch::channel(0).0,
        })
    }

    fn shared(self: &Arc<Self>) -> SharedTokenStore {
        self.clone()
    }
}

impl TokenStore for FakeTokens {
    fn issue(
        &self,
        label: AgentLabel,
        now: WallTime,
    ) -> BoxFuture<'_, Result<IssuedToken, TokenError>> {
        Box::pin(async move {
            if self.failing.load(Ordering::SeqCst) {
                return Err(TokenError::Storage("/private/tokens.json is full".into()));
            }
            let mut records = locked(&self.records);
            if records
                .iter()
                .any(|record| record.label == label && record.is_active())
            {
                return Err(TokenError::LabelInUse);
            }
            let n = records.len() + 1;
            let record = TokenRecord {
                id: TokenId::new(format!("t-{n:08x}")).unwrap(),
                label,
                created: now,
                revoked: None,
            };
            let secret = format!("dara_secret-value-{n}");
            locked(&self.secrets).push(secret.clone());
            records.push(record.clone());
            Ok(IssuedToken {
                record,
                secret: TokenSecret::new(secret),
            })
        })
    }

    fn list(&self) -> Vec<TokenRecord> {
        locked(&self.records).clone()
    }

    fn revoke<'a>(
        &'a self,
        id: &'a TokenId,
        now: WallTime,
    ) -> BoxFuture<'a, Result<TokenRecord, TokenError>> {
        Box::pin(async move {
            let mut records = locked(&self.records);
            let record = records
                .iter_mut()
                .find(|record| &record.id == id)
                .ok_or(TokenError::NotFound)?;
            if record.revoked.is_none() {
                record.revoked = Some(now);
                self.revoked.send_modify(|count| *count += 1);
            }
            Ok(record.clone())
        })
    }

    fn authenticate(&self, _: &str) -> Credential {
        Credential::Unknown
    }

    fn is_active(&self, id: &TokenId) -> bool {
        locked(&self.records)
            .iter()
            .any(|record| &record.id == id && record.is_active())
    }

    fn changes(&self) -> watch::Receiver<u64> {
        self.revoked.subscribe()
    }
}

async fn start_with_tokens() -> (AgentHarness, Arc<FakeTokens>) {
    let tokens = FakeTokens::new();
    let h = start(Setup {
        tokens: Some(tokens.shared()),
        ..Setup::default()
    })
    .await;
    (h, tokens)
}

fn refusal(reason: RefusalReason, uid: u32) -> AccessRefusal {
    AccessRefusal {
        endpoint: EndpointKind::Agent,
        reason,
        principal: None,
        peer_uid: Some(uid),
        resource: None,
    }
}

fn refusals_written(h: &AgentHarness) -> Vec<AuditRecord> {
    h.audit
        .records()
        .into_iter()
        .filter(|record| matches!(record.event, AuditEvent::AccessRefused { .. }))
        .collect()
}

fn suppressed_in(record: &AuditRecord) -> u32 {
    match &record.event {
        AuditEvent::AccessRefused { suppressed, .. } => *suppressed,
        other => panic!("not a refusal: {other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn issuing_a_token_applies_the_label_rule() {
    let (h, tokens) = start_with_tokens().await;

    for bad in ["Claude-Code", "a b", "-x", "unauthenticated"] {
        let error = h.operator.issue_token(label(bad)).await.unwrap_err();
        assert!(
            matches!(error, ControlError::InvalidRequest(_)),
            "{bad}: {error:?}"
        );
    }
    assert!(tokens.list().is_empty(), "nothing was issued");

    let issued = h.operator.issue_token(label("claude-code")).await.unwrap();
    assert_eq!(issued.record.label, label("claude-code"));
    assert_eq!(issued.record.created, h.clock.now());
    assert!(issued.secret.expose().starts_with("dara_"));

    // A second active token for the label is refused until the first is revoked.
    let again = h
        .operator
        .issue_token(label("claude-code"))
        .await
        .unwrap_err();
    assert!(
        matches!(&again, ControlError::InvalidRequest(why) if why.contains("revoke")),
        "{again:?}"
    );
    h.operator
        .revoke_token(issued.record.id.clone())
        .await
        .unwrap();
    h.operator.issue_token(label("claude-code")).await.unwrap();
    assert_eq!(h.operator.tokens().await.unwrap().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn the_token_methods_are_the_operators_alone_whatever_the_store_can_answer() {
    let (h, _tokens) = start_with_tokens().await;
    let id = TokenId::new("t-00000001").unwrap();
    assert_eq!(
        h.agent.issue_token(label("openclaw")).await.unwrap_err(),
        ControlError::Forbidden
    );
    assert_eq!(h.agent.tokens().await.unwrap_err(), ControlError::Forbidden);
    assert_eq!(
        h.agent.revoke_token(id).await.unwrap_err(),
        ControlError::Forbidden
    );
}

#[tokio::test(start_paused = true)]
async fn a_service_with_no_token_store_answers_the_operator_unavailable() {
    // An Agent path with no store.
    let h = start_default().await;
    for error in [
        h.operator.issue_token(label("openclaw")).await.unwrap_err(),
        h.operator.tokens().await.unwrap_err(),
        h.operator
            .revoke_token(TokenId::new("t-00000001").unwrap())
            .await
            .unwrap_err(),
    ] {
        assert!(matches!(error, ControlError::Unavailable(_)), "{error:?}");
    }

    // And a service with no Agent path at all.
    let plain = super::tests::harness();
    assert!(matches!(
        plain.operator.issue_token(label("openclaw")).await,
        Err(ControlError::Unavailable(_))
    ));
}

#[tokio::test(start_paused = true)]
async fn a_token_store_that_fails_is_reported_without_its_text() {
    let (h, tokens) = start_with_tokens().await;
    tokens.failing.store(true, Ordering::SeqCst);
    let error = h.operator.issue_token(label("openclaw")).await.unwrap_err();
    let ControlError::Unavailable(why) = error else {
        panic!("expected Unavailable");
    };
    assert!(!why.contains("/private"), "{why}");

    let unknown = h
        .operator
        .revoke_token(TokenId::new("t-0000ffff").unwrap())
        .await
        .unwrap_err();
    assert_eq!(unknown, ControlError::NotFound("token"));
}

#[tokio::test(start_paused = true)]
async fn issuing_and_revoking_are_audited_and_the_record_holds_no_secret() {
    let (h, tokens) = start_with_tokens().await;
    let issued = h.operator.issue_token(label("openclaw")).await.unwrap();
    let id = issued.record.id.clone();
    h.operator.revoke_token(id.clone()).await.unwrap();
    // Revoking again is not an error and is not recorded again.
    let again = h.operator.revoke_token(id.clone()).await.unwrap();
    assert!(again.revoked.is_some());

    let records = h.audit.records();
    let token_events: Vec<&AuditRecord> = records
        .iter()
        .filter(|record| {
            matches!(
                record.event,
                AuditEvent::TokenIssued { .. } | AuditEvent::TokenRevoked { .. }
            )
        })
        .collect();
    assert_eq!(token_events.len(), 2);
    assert_eq!(
        token_events[0].event,
        AuditEvent::TokenIssued {
            id: id.clone(),
            label: label("openclaw")
        }
    );
    assert_eq!(
        token_events[1].event,
        AuditEvent::TokenRevoked {
            id,
            label: label("openclaw")
        }
    );
    assert!(token_events
        .iter()
        .all(|record| record.principal == Some(Principal::Operator)));
    for (name, durability) in locked(&h.audit.durability).iter() {
        if name.starts_with("audit:token_") {
            assert_eq!(*durability, Durability::Flushed, "{name}");
        }
    }
    let written = format!("{records:?}");
    for secret in locked(&tokens.secrets).iter() {
        assert!(!written.contains(secret), "the log holds a secret");
    }
}

#[tokio::test(start_paused = true)]
async fn the_first_refusal_of_a_key_is_written_and_the_rest_inside_a_minute_are_counted() {
    let h = start_default().await;
    for _ in 0..5 {
        h.service
            .record_refusal(refusal(RefusalReason::NoCredential, 502))
            .await;
    }
    let written = refusals_written(&h);
    assert_eq!(written.len(), 1);
    assert_eq!(suppressed_in(&written[0]), 0);
    assert_eq!(
        written[0].principal, None,
        "no valid credential, no principal"
    );
    assert_eq!(
        locked(&h.audit.durability)
            .iter()
            .find(|(name, _)| *name == "audit:access_refused")
            .map(|(_, durability)| *durability),
        Some(Durability::Flushed)
    );

    // A different reason, or a different caller, is a different key.
    h.service
        .record_refusal(refusal(RefusalReason::UnknownCredential, 502))
        .await;
    h.service
        .record_refusal(refusal(RefusalReason::NoCredential, 503))
        .await;
    assert_eq!(refusals_written(&h).len(), 3);

    // Just inside the minute, still counted.
    advance(Duration::from_secs(59)).await;
    h.service
        .record_refusal(refusal(RefusalReason::NoCredential, 502))
        .await;
    assert_eq!(refusals_written(&h).len(), 3);
}

#[tokio::test(start_paused = true)]
async fn the_next_record_after_a_minute_carries_the_suppressed_count() {
    let h = start_default().await;
    for _ in 0..5 {
        h.service
            .record_refusal(refusal(RefusalReason::NoCredential, 502))
            .await;
    }
    advance(Duration::from_secs(60)).await;
    h.service
        .record_refusal(refusal(RefusalReason::NoCredential, 502))
        .await;
    let written = refusals_written(&h);
    assert_eq!(written.len(), 2);
    assert_eq!(suppressed_in(&written[0]), 0);
    assert_eq!(suppressed_in(&written[1]), 4);
}

#[tokio::test(start_paused = true)]
async fn at_most_256_keys_and_30_records_a_minute_are_kept_whatever_the_keys() {
    let h = start_default().await;

    // A hundred different callers in one minute: thirty are written.
    for uid in 0..100 {
        h.service
            .record_refusal(refusal(RefusalReason::NoCredential, uid))
            .await;
    }
    assert_eq!(refusals_written(&h).len(), 30);

    // A minute on, the next record says how many callers went unrecorded.
    advance(Duration::from_secs(61)).await;
    h.service
        .record_refusal(refusal(RefusalReason::NoCredential, 1_000))
        .await;
    let written = refusals_written(&h);
    assert_eq!(written.len(), 31);
    assert_eq!(suppressed_in(&written[30]), 70);

    // Minute after minute of new callers never tracks more than 256.
    for round in 0..12_u32 {
        advance(Duration::from_secs(61)).await;
        for uid in 0..30 {
            h.service
                .record_refusal(refusal(
                    RefusalReason::NoCredential,
                    10_000 + round * 100 + uid,
                ))
                .await;
        }
    }
    let agent = h.service.inner.agent.as_ref().unwrap();
    assert!(
        agent.tracked_refusals() <= 256,
        "{}",
        agent.tracked_refusals()
    );
    assert!(
        agent.tracked_refusals() > 200,
        "the oldest are dropped, not all"
    );
}

#[tokio::test(start_paused = true)]
async fn a_refusal_names_a_route_and_never_a_path_and_is_bounded() {
    let h = start_default().await;
    h.service
        .record_refusal(AccessRefusal {
            endpoint: EndpointKind::Agent,
            reason: RefusalReason::ResourceNotServed,
            principal: Some(Principal::Agent(label("openclaw"))),
            peer_uid: Some(502),
            resource: Some(format!("/v1/policy\n{}", "x".repeat(10_000))),
        })
        .await;
    let written = refusals_written(&h);
    let AuditEvent::AccessRefused { resource, .. } = &written[0].event else {
        unreachable!()
    };
    let resource = resource.as_deref().unwrap();
    assert!(resource.chars().count() <= 128);
    assert!(!resource.chars().any(char::is_control));
    assert_eq!(
        written[0].principal,
        Some(Principal::Agent(label("openclaw")))
    );
}

#[tokio::test(start_paused = true)]
async fn a_refusal_record_is_flushed_and_bounded_to_a_second_on_a_stalled_disk() {
    let h = start_default().await;
    h.audit.set_mode(AuditMode::HangAppend);
    let started = tokio::time::Instant::now();
    h.service
        .record_refusal(refusal(RefusalReason::NoCredential, 502))
        .await;
    let waited = started.elapsed();
    assert!(waited >= Duration::from_secs(1), "{waited:?}");
    assert!(waited < Duration::from_secs(2), "{waited:?}");
}

#[tokio::test(start_paused = true)]
async fn a_failed_refusal_append_does_not_change_the_audit_health() {
    let h = start_default().await;
    assert_eq!(h.agent.health().await.unwrap().audit, AuditHealth::Ok);
    h.audit.set_mode(AuditMode::FailAppend);
    h.service
        .record_refusal(refusal(RefusalReason::NoCredential, 502))
        .await;
    h.audit.set_mode(AuditMode::HangAppend);
    h.service
        .record_refusal(refusal(RefusalReason::UnknownCredential, 502))
        .await;
    // Being refused must not be a way to mark the log failing, and so to have
    // every agent write refused.
    assert_eq!(h.agent.health().await.unwrap().audit, AuditHealth::Ok);
}

#[tokio::test(start_paused = true)]
async fn the_ledger_rebuild_ignores_refusals_and_token_events() {
    let now = WallTime(10_000_000);
    let at = now.saturating_sub(Duration::from_secs(120));
    let plain = |event: AuditEvent, principal: Option<Principal>| AuditRecord {
        schema: crate::audit::AUDIT_SCHEMA,
        run: WallTime(5),
        at,
        operation: None,
        principal,
        receiver: None,
        event,
    };
    let history = vec![
        plain(
            AuditEvent::AccessRefused {
                endpoint: EndpointKind::Agent,
                reason: RefusalReason::NoCredential,
                peer_uid: Some(502),
                resource: None,
                suppressed: 3,
            },
            None,
        ),
        dispatching_record(5, 1, at, Principal::Operator, -120, -90),
        plain(
            AuditEvent::TokenIssued {
                id: TokenId::new("t-00000001").unwrap(),
                label: label("openclaw"),
            },
            Some(Principal::Operator),
        ),
    ];
    let h = start(Setup {
        history: Some(history),
        ..Setup::default()
    })
    .await;
    assert!(h.agent.health().await.unwrap().ledger_ready);
    assert_eq!(counted(&h), 1, "only the volume write counts");
}

#[tokio::test(start_paused = true)]
async fn a_service_built_by_new_records_no_refusal() {
    let plain = super::tests::harness();
    plain
        .service
        .record_refusal(refusal(RefusalReason::NoCredential, 502))
        .await;
}
