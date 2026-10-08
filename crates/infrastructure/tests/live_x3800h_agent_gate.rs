//! Explicitly armed, state-restoring live validation of the Agent path.
//!
//! It starts a control service with the real connector, a temporary policy file,
//! and a temporary audit directory, and makes one volume change through an Agent
//! handle that the policy allows and two that it holds or denies. It is never
//! part of ordinary checks, and it writes to the receiver only when armed.
//!
//! The policy's limits are built from the volume `L` the receiver shows now, so
//! that no request the test makes can reach the declared safe level `S` even if
//! the gate were wrong: the hard limit is `L + 3.0 dB`, the ceiling `L + 1.5 dB`,
//! and the highest target the test names is `L + 3.5 dB`. The test asserts
//! `L + 3.5 dB <= S` before it writes anything.
//!
//! Arm it with `ALLOW_RECEIVER_WRITES=1`, `DENON_X3800H_HOST`, and
//! `DENON_X3800H_SAFE_VOLUME_HALF_STEPS`; `make test-live-x3800h-agent` checks
//! them. The audit directory is left in place and its path printed, so its
//! permissions can be inspected afterwards.

use denon_avr_application::ports::AsyncConfigRepository;
use denon_avr_application::{
    AgentLabel, AgentLimits, AgentPath, AuditEvent, AuditQuery, ControlService, OperationControl,
    OperationSnapshot, OperationStatus, OperationSubmission, Principal, ServiceConfig,
};
use denon_avr_domain::{
    ConfiguredReceivers, DispatchCertainty, MasterVolume, ReceiverId, ReceiverIdentity,
    ReceiverIntent,
};
use denon_avr_infrastructure::{
    AvrSessionConfig, JsonlAuditLog, SsdpDiscoveryAdapter, SystemClock, X3800hConnector,
    YamlConfigRepository, YamlPolicySource,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

const RECEIVER: &str = "live";

fn volume(half_steps: i16) -> ReceiverIntent {
    ReceiverIntent::Volume(
        MasterVolume::db_half_steps(half_steps).expect("a target on the receiver's scale"),
    )
}

/// The receiver's volume now, in half steps, once it has been read.
async fn observed_half_steps(operator: &dyn denon_avr_application::OperatorControl) -> Option<i16> {
    let id = ReceiverId::new(RECEIVER).unwrap();
    let state = operator.state(&id).await.ok()?;
    match state.latest().main_zone.volume.last_good?.value {
        MasterVolume::DbHalfSteps(steps) => Some(steps),
        MasterVolume::Minimum => None,
    }
}

async fn finish(
    control: &dyn OperationControl,
    intent: ReceiverIntent,
) -> Result<OperationSnapshot, String> {
    let id = ReceiverId::new(RECEIVER).unwrap();
    let first = control
        .submit(&id, OperationSubmission::new(intent))
        .await
        .map_err(|error| error.to_string())?;
    control
        .operation(first.id, Some(Duration::from_secs(20)))
        .await
        .map_err(|error| error.to_string())
}

fn db(half_steps: i16) -> String {
    format!("{:.1}", f64::from(half_steps) / 2.0)
}

#[tokio::test]
#[ignore = "requires explicit write arm and a physical AVC-X3800H"]
async fn armed_live_agent_volume_change_is_gated_audited_and_restored() {
    assert_eq!(
        std::env::var("ALLOW_RECEIVER_WRITES").as_deref(),
        Ok("1"),
        "set ALLOW_RECEIVER_WRITES=1 to arm live writes"
    );
    let safe = std::env::var("DENON_X3800H_SAFE_VOLUME_HALF_STEPS")
        .expect("declare DENON_X3800H_SAFE_VOLUME_HALF_STEPS before live writes")
        .parse::<i16>()
        .expect("safe volume ceiling must be an integer half-step value");
    assert!((-159..=36).contains(&safe));
    let host =
        std::env::var("DENON_X3800H_HOST").expect("set DENON_X3800H_HOST before live writes");

    // A directory of our own: a configuration naming the receiver, a policy, and
    // the audit log.
    let root = std::env::temp_dir().join(format!("denon-live-agent-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let audit_directory = root.join("audit");
    println!("audit directory: {}", audit_directory.display());

    let config = Arc::new(YamlConfigRepository::new(
        root.join("denon-avr-remote.yaml"),
    ));
    config
        .save(&ConfiguredReceivers {
            current: Some(RECEIVER.into()),
            receivers: BTreeMap::from([(
                RECEIVER.into(),
                ReceiverIdentity {
                    host,
                    model: Some("AVR-X3800H".into()),
                    friendly_name: None,
                },
            )]),
            ..ConfiguredReceivers::default()
        })
        .await
        .expect("write the temporary configuration");

    // Read the level the limits are built from before any policy exists. A
    // service without an Agent path has no policy to wait for.
    let reader = ControlService::new(
        Arc::new(X3800hConnector::new(AvrSessionConfig::default())),
        config.clone(),
        Arc::new(SsdpDiscoveryAdapter),
        ServiceConfig::default(),
    );
    let level = observed_half_steps(reader.operator().as_ref())
        .await
        .expect("the receiver must show a volume before the test writes");
    reader.shutdown().await;

    let hard = level + 6; // +3.0 dB
    let ceiling = level + 3; // +1.5 dB
    let allowed = level + 2; // +1.0 dB
    let held = level + 4; // +2.0 dB
    let denied = level + 7; // +3.5 dB
    assert!(
        denied <= safe,
        "the highest target, {} dB, is above the declared safe level {} dB",
        db(denied),
        db(safe)
    );
    assert!(denied <= 36, "the highest target must be on the scale");

    let policy_path = root.join("policy.yaml");
    std::fs::write(
        &policy_path,
        format!(
            "unclassified: require_approval\nagent:\n  approval_lifetime_minutes: 5\n  rules:\n    \
             - id: volume-hard-limit\n      when: {{ intent: volume, target_above_db: {} }}\n      then: deny\n    \
             - id: volume-ceiling\n      when: {{ intent: volume, target_above_db: {} }}\n      then: require_approval\n    \
             - id: volume-step\n      when: {{ intent: volume, increase_over_baseline_db: 6.0 }}\n      then: require_approval\n    \
             - id: volume-budget\n      when: {{ intent: volume, rise_over_window_db: 10.0, window_minutes: 10 }}\n      then: require_approval\n",
            db(hard),
            db(ceiling)
        ),
    )
    .unwrap();

    let service = ControlService::start(
        Arc::new(X3800hConnector::new(AvrSessionConfig::default())),
        config,
        Arc::new(SsdpDiscoveryAdapter),
        ServiceConfig::default(),
        AgentPath {
            policy: Arc::new(YamlPolicySource::new(&policy_path)),
            audit: Arc::new(JsonlAuditLog::new(&audit_directory)),
            clock: Arc::new(SystemClock),
            limits: AgentLimits::default(),
            tokens: None,
        },
    )
    .await;
    let operator = service.operator();
    let agent = service
        .handle(Principal::Agent(AgentLabel::new("live-test").unwrap()))
        .expect("an agent handle from a service built by start");
    let health = operator.health().await.expect("health");
    println!("health: {health:?}");

    // The three requests. Each is observed, not assumed, and the volume is put
    // back whatever happened.
    let first = finish(agent.as_ref(), volume(allowed)).await;
    let after_first = observed_half_steps(operator.as_ref()).await;
    let second = finish(agent.as_ref(), volume(held)).await;
    let after_second = observed_half_steps(operator.as_ref()).await;
    let third = finish(agent.as_ref(), volume(denied)).await;
    let after_third = observed_half_steps(operator.as_ref()).await;
    let restore = finish(operator.as_ref(), volume(level)).await;
    let restored = observed_half_steps(operator.as_ref()).await;
    let audit = operator.audit(AuditQuery::new(100)).await;
    service.shutdown().await;

    // Only now, with the receiver back where it was, are the results judged.
    let first = first.expect("the allowed change was submitted");
    assert_eq!(first.status, OperationStatus::Completed, "{first:?}");
    assert_eq!(first.dispatch, DispatchCertainty::CompleteWrite);
    assert_eq!(after_first, Some(allowed), "the receiver shows the change");

    let second = second.expect("the held change was submitted");
    assert_eq!(
        second.status,
        OperationStatus::ApprovalUnavailable,
        "{second:?}"
    );
    assert_eq!(second.dispatch, DispatchCertainty::NotDispatched);
    assert_eq!(after_second, Some(allowed), "the receiver did not move");

    let third = third.expect("the denied change was submitted");
    assert_eq!(third.status, OperationStatus::Denied, "{third:?}");
    assert_eq!(third.dispatch, DispatchCertainty::NotDispatched);
    assert_eq!(after_third, Some(allowed), "the receiver did not move");

    let restore = restore.expect("the restoring change was submitted");
    assert_eq!(restore.status, OperationStatus::Completed, "{restore:?}");
    assert_eq!(restored, Some(level), "the volume is back where it started");

    // The log holds the records of the change and of both refusals.
    let page = audit.expect("the audit log can be read");
    let events: Vec<&AuditEvent> = page
        .entries
        .iter()
        .map(|entry| &entry.record.event)
        .collect();
    let count = |wanted: fn(&AuditEvent) -> bool| events.iter().filter(|e| wanted(e)).count();
    assert!(
        count(|e| matches!(e, AuditEvent::Dispatching { .. })) >= 2,
        "the agent's change and the Operator's restore"
    );
    assert!(count(|e| matches!(e, AuditEvent::Decided { .. })) >= 3);
    assert!(count(|e| matches!(e, AuditEvent::Finished { .. })) >= 4);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode =
            |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&audit_directory), 0o700);
        assert_eq!(mode(&audit_directory.join("audit.jsonl")), 0o600);
    }
    println!("audit directory kept at {}", audit_directory.display());
}
