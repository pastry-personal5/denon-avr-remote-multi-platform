//! The YAML policy loader: what it accepts, what it refuses, and the digest it
//! reports.

use denon_avr_application::{PolicyLoadError, PolicySource};
use denon_avr_infrastructure::policy_yaml::{YamlPolicySource, MAX_POLICY_BYTES};
use denon_avr_policy::{Effect, IntentKind, IntentValue, Level, PolicyConfig, Rule, Span};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A directory under the system's temporary one, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "denon-policy-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn file(&self, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join("policy.yaml");
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sample_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples/policy.yaml")
}

async fn load_text(text: &str) -> Result<denon_avr_application::LoadedPolicy, PolicyLoadError> {
    let scratch = Scratch::new();
    YamlPolicySource::new(scratch.file(text)).load().await
}

/// The text of the load error, which every refusal must be.
async fn refusal(text: &str) -> String {
    match load_text(text).await {
        Err(PolicyLoadError::Invalid(why)) => why,
        other => panic!("expected the policy to be refused as invalid, got {other:?}"),
    }
}

fn level(db: f64) -> Level {
    Level::from_db(db).unwrap()
}

fn span(db: f64) -> Span {
    Span::from_db(db).unwrap()
}

/// The owner's configuration, built the way the engine's own tests build it.
fn owners_rules() -> Vec<Rule> {
    vec![
        Rule::new("volume-hard-limit", Effect::Deny)
            .intent(IntentKind::Volume)
            .target_above(level(-20.0)),
        Rule::new("volume-ceiling", Effect::RequireApproval)
            .intent(IntentKind::Volume)
            .target_above(level(-30.0)),
        Rule::new("volume-step", Effect::RequireApproval)
            .intent(IntentKind::Volume)
            .increase_over_baseline(span(6.0)),
        Rule::new("volume-budget", Effect::RequireApproval)
            .intent(IntentKind::Volume)
            .budget(span(10.0), 10),
        Rule::new("loud-or-unknown-baseline", Effect::RequireApproval)
            .intent_value(IntentKind::MainZonePower, IntentValue::On)
            .intent_value(IntentKind::Mute, IntentValue::Off)
            .baseline_volume_above(level(-30.0)),
        Rule::new("system-power", Effect::RequireApproval).intent(IntentKind::SystemPower),
        Rule::new("zone2-power", Effect::RequireApproval).intent(IntentKind::Zone2Power),
        Rule::new("low-risk", Effect::Allow)
            .intent_value(IntentKind::MainZonePower, IntentValue::Off)
            .intent_value(IntentKind::Mute, IntentValue::On)
            .intent(IntentKind::Source)
            .intent(IntentKind::SoundMode),
    ]
}

/// SHA-256 of `docs/examples/policy.yaml`, from `shasum -a 256`, not from this
/// code. Editing the sample means computing it again.
const SAMPLE_DIGEST: &str = "aca9bcb4f993c5e1bc3ca73ad820747c025a74137f37339d3efeb9ca5a680525";

#[tokio::test]
async fn the_sample_loads_and_has_the_expected_digest() {
    let loaded = YamlPolicySource::new(sample_path()).load().await.unwrap();

    assert_eq!(loaded.digest.to_string(), SAMPLE_DIGEST);
    assert_eq!(
        loaded.config,
        PolicyConfig::new(owners_rules(), Duration::from_secs(300)).unwrap()
    );
    assert_eq!(
        loaded.text,
        std::fs::read_to_string(sample_path()).unwrap(),
        "the text is what was read"
    );
}

#[tokio::test]
async fn the_sample_classifies_every_intent_kind_for_any_agent() {
    let loaded = YamlPolicySource::new(sample_path()).load().await.unwrap();
    for kind in IntentKind::ALL {
        let values: Vec<Option<IntentValue>> = {
            let accepted: Vec<_> = [IntentValue::On, IntentValue::Off, IntentValue::Standby]
                .into_iter()
                .filter(|value| kind.accepts(*value))
                .map(Some)
                .collect();
            if accepted.is_empty() {
                vec![None]
            } else {
                accepted
            }
        };
        for value in values {
            let named = loaded.config.rules().iter().any(|rule| {
                rule.scope.matches("any-agent-at-all", "any-receiver")
                    && rule
                        .filter
                        .alternatives
                        .iter()
                        .any(|alternative| alternative.covers(kind, value))
            });
            assert!(named, "{} {value:?} is unclassified", kind.as_str());
        }
    }
}

#[tokio::test]
async fn an_empty_file_is_an_empty_policy_that_requires_approval_for_everything() {
    for text in ["", "   \n", "# nothing but a comment\n"] {
        let loaded = load_text(text).await.unwrap();
        assert!(loaded.config.rules().is_empty(), "{text:?}");
        assert_eq!(loaded.config.approval_lifetime(), Duration::from_secs(300));
    }
}

#[tokio::test]
async fn unclassified_allow_is_rejected() {
    for value in ["allow", "deny", "\"\""] {
        let why = refusal(&format!("unclassified: {value}\n")).await;
        assert!(why.contains("unclassified"), "{why}");
    }
    assert!(load_text("unclassified: require_approval\n").await.is_ok());
}

#[tokio::test]
async fn or_unknown_false_is_rejected() {
    let why = refusal(
        "agent:\n  rules:\n    - id: loud\n      when:\n        intent: mute\n        value: off\n        baseline_volume: { above_db: -30.0, or_unknown: false }\n      then: require_approval\n",
    )
    .await;
    assert!(why.contains("loud") && why.contains("or_unknown"), "{why}");

    // Left out, it is true: an unknown or stale volume always matches.
    let loaded = load_text(
        "agent:\n  rules:\n    - id: loud\n      when:\n        intent: mute\n        value: off\n        baseline_volume: { above_db: -30.0 }\n      then: require_approval\n",
    )
    .await
    .unwrap();
    assert_eq!(loaded.config.rules().len(), 1);
}

#[tokio::test]
async fn a_limit_off_the_half_decibel_grid_is_rejected() {
    for condition in [
        "target_above_db: -20.3",
        "increase_over_baseline_db: 6.25",
        "rise_over_window_db: 10.1, window_minutes: 10",
        "target_above_db: 40.0",
        "target_above_db: .nan",
    ] {
        let why = refusal(&format!(
            "agent:\n  rules:\n    - id: limit\n      when: {{ intent: volume, {condition} }}\n      then: deny\n"
        ))
        .await;
        assert!(why.contains("limit"), "{condition}: {why}");
    }
}

#[tokio::test]
async fn a_budget_needs_both_its_limit_and_its_window() {
    for condition in [
        "rise_over_window_db: 10.0",
        "window_minutes: 10",
        "rise_over_window_db: 10.0, window_minutes: 0",
        "rise_over_window_db: 10.0, window_minutes: 1441",
        "rise_over_window_db: 10.0, window_minutes: 70000",
    ] {
        let why = refusal(&format!(
            "agent:\n  rules:\n    - id: budget\n      when: {{ intent: volume, {condition} }}\n      then: require_approval\n"
        ))
        .await;
        assert!(why.contains("budget"), "{condition}: {why}");
    }
    assert!(load_text(
        "agent:\n  rules:\n    - id: budget\n      when: { intent: volume, rise_over_window_db: 10.0, window_minutes: 1440 }\n      then: require_approval\n"
    )
    .await
    .is_ok());
}

#[tokio::test]
async fn an_allow_rule_with_a_condition_is_rejected() {
    let why = refusal(
        "agent:\n  rules:\n    - id: quiet-only\n      when: { intent: volume, target_above_db: -50.0 }\n      then: allow\n",
    )
    .await;
    assert!(why.contains("quiet-only") && why.contains("allow"), "{why}");

    let why =
        refusal("agent:\n  rules:\n    - id: everything\n      when: {}\n      then: allow\n")
            .await;
    assert!(why.contains("everything"), "{why}");
}

#[tokio::test]
async fn a_time_of_day_key_is_rejected() {
    let why = refusal(
        "agent:\n  rules:\n    - id: night\n      when: { intent: volume, time_of_day: night }\n      then: deny\n",
    )
    .await;
    assert!(why.contains("time_of_day"), "{why}");

    let why = refusal("time_of_day: night\n").await;
    assert!(why.contains("time_of_day"), "{why}");
}

#[tokio::test]
async fn unknown_keys_are_rejected_by_name_wherever_they_are() {
    for (text, key) in [
        ("surprise: 1\n", "surprise"),
        ("agent:\n  surprise: 1\n", "surprise"),
        (
            "agent:\n  rules:\n    - id: r\n      then: deny\n      surprise: 1\n      when: {}\n",
            "surprise",
        ),
        (
            "agent:\n  rules:\n    - id: r\n      when: { intent: volume, targt_above_db: -20.0 }\n      then: deny\n",
            "targt_above_db",
        ),
        (
            "agent:\n  rules:\n    - id: r\n      when:\n        any:\n          - { intent: mute, surprise: 1 }\n      then: deny\n",
            "surprise",
        ),
    ] {
        let why = refusal(text).await;
        assert!(why.contains(key), "{key}: {why}");
    }
}

#[tokio::test]
async fn a_duplicate_rule_id_is_rejected() {
    let why = refusal(
        "agent:\n  rules:\n    - id: same\n      when: { intent: volume }\n      then: require_approval\n    - id: same\n      when: { intent: mute }\n      then: deny\n",
    )
    .await;
    assert!(why.contains("same"), "{why}");
}

#[tokio::test]
async fn what_a_rule_names_must_make_sense() {
    for (rule, expected) in [
        // A value that does not belong to the intent.
        ("{ intent: volume, value: on }", "value"),
        ("{ intent: mute, value: standby }", "standby"),
        // A value with no intent to belong to.
        ("{ value: on }", "value"),
        // Two ways of naming intents at once.
        ("{ intent: mute, any: [{ intent: source }] }", "any"),
        // An intent that does not exist.
        ("{ intent: surround_back }", "surround_back"),
        ("{ any: [{ intent: surround_back }] }", "surround_back"),
        // A volume limit on a rule that is not only about volume.
        ("{ intent: mute, target_above_db: -20.0 }", "volume"),
        ("{ target_above_db: -20.0 }", "volume"),
        // Scopes that apply to nobody.
        ("{ agents: [] }", "agents"),
        ("{ receivers: [] }", "receivers"),
    ] {
        let why = refusal(&format!(
            "agent:\n  rules:\n    - id: odd\n      when: {rule}\n      then: deny\n"
        ))
        .await;
        assert!(
            why.contains("odd") || why.contains(expected),
            "{rule}: {why}"
        );
        assert!(why.contains(expected), "{rule}: {why}");
    }
}

#[tokio::test]
async fn a_rule_can_be_narrowed_to_agents_and_receivers() {
    let loaded = load_text(
        "agent:\n  rules:\n    - id: read-only\n      when: { agents: [claude-code, other], receivers: [living-room] }\n      then: deny\n",
    )
    .await
    .unwrap();
    let rule = &loaded.config.rules()[0];
    assert!(rule.scope.matches("claude-code", "living-room"));
    assert!(rule.scope.matches("other", "living-room"));
    assert!(!rule.scope.matches("Other", "living-room"), "exact match");
    assert!(!rule.scope.matches("claude-code", "kitchen"));
}

#[tokio::test]
async fn an_uppercase_label_in_a_policy_file_is_refused() {
    // A token label is lowercase, so a rule naming `Claude-Code` would apply to
    // nobody and read as though it restricted somebody.
    for labels in [
        "[Claude-Code]",
        "[claude-code, Other]",
        "[unauthenticated]",
        "[\"a b\"]",
    ] {
        let why = refusal(&format!(
            "agent:\n  rules:\n    - id: read-only\n      when: {{ agents: {labels} }}\n      then: deny\n"
        ))
        .await;
        assert!(
            why.contains("read-only") && why.contains("label"),
            "{labels}: {why}"
        );
        assert!(
            !why.contains("Claude-Code") && !why.contains("Other"),
            "{why}"
        );
    }
}

#[tokio::test]
async fn the_approval_lifetime_is_in_minutes_and_has_a_default() {
    let loaded = load_text("agent:\n  approval_lifetime_minutes: 12\n")
        .await
        .unwrap();
    assert_eq!(
        loaded.config.approval_lifetime(),
        Duration::from_secs(12 * 60)
    );
    for bad in ["0", "-3", "1.5", "\"five\"", "100000000000"] {
        let why = refusal(&format!("agent:\n  approval_lifetime_minutes: {bad}\n")).await;
        assert!(why.contains("approval_lifetime_minutes"), "{bad}: {why}");
    }
}

#[tokio::test]
async fn an_oversize_file_is_rejected() {
    let scratch = Scratch::new();
    let at_the_limit = "#".repeat(MAX_POLICY_BYTES - 1) + "\n";
    assert_eq!(at_the_limit.len(), MAX_POLICY_BYTES);
    assert!(YamlPolicySource::new(scratch.file(&at_the_limit))
        .load()
        .await
        .is_ok());

    let over = at_the_limit + "#";
    let result = YamlPolicySource::new(scratch.file(&over)).load().await;
    assert!(
        matches!(&result, Err(PolicyLoadError::Invalid(why)) if why.contains("larger")),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_missing_file_is_reported_as_missing() {
    let scratch = Scratch::new();
    let result = YamlPolicySource::new(scratch.0.join("nope.yaml"))
        .load()
        .await;
    assert_eq!(result.unwrap_err(), PolicyLoadError::Missing);
}

#[tokio::test]
async fn a_file_that_cannot_be_read_is_unreadable_not_missing() {
    let scratch = Scratch::new();
    // A directory is there, and is not a policy.
    let result = YamlPolicySource::new(&scratch.0).load().await;
    assert!(
        matches!(result, Err(PolicyLoadError::Unreadable(_))),
        "{result:?}"
    );
    let result = YamlPolicySource::new(scratch.file([0xff, 0xfe, 0x00]))
        .load()
        .await;
    assert!(
        matches!(&result, Err(PolicyLoadError::Invalid(why)) if why.contains("UTF-8")),
        "{result:?}"
    );
}

#[tokio::test]
async fn one_changed_byte_changes_the_digest() {
    let original = std::fs::read(sample_path()).unwrap();
    let scratch = Scratch::new();
    let loaded = YamlPolicySource::new(scratch.file(&original))
        .load()
        .await
        .unwrap();
    assert_eq!(
        loaded.digest.to_string(),
        SAMPLE_DIGEST,
        "copying changes nothing"
    );

    // A comment byte: the rules are the same, the file is not.
    let mut changed = original.clone();
    changed.extend_from_slice(b"#");
    let other = YamlPolicySource::new(scratch.file(&changed))
        .load()
        .await
        .unwrap();
    assert_eq!(other.config, loaded.config);
    assert_ne!(other.digest, loaded.digest);
}

#[tokio::test]
async fn an_edit_is_seen_by_the_next_load() {
    let scratch = Scratch::new();
    let path = scratch.file("agent:\n  approval_lifetime_minutes: 5\n");
    let source = YamlPolicySource::new(&path);
    let first = source.load().await.unwrap();
    std::fs::write(&path, "agent:\n  approval_lifetime_minutes: 9\n").unwrap();
    let second = source.load().await.unwrap();
    assert_eq!(first.config.approval_lifetime(), Duration::from_secs(300));
    assert_eq!(second.config.approval_lifetime(), Duration::from_secs(540));
    assert_ne!(first.digest, second.digest);
}

#[tokio::test]
async fn errors_name_the_rule_or_key_and_never_quote_the_file() {
    let why = refusal(
        "# sk-secret-token-1234 and a comment that must not be echoed\nagent:\n  rules:\n    - id: loud\n      when: { intent: volume, target_above_db: -20.3 }\n      then: deny\n# another-private-line\n",
    )
    .await;
    assert!(why.contains("loud"), "{why}");
    assert!(
        !why.contains("sk-secret-token") && !why.contains("another-private-line"),
        "{why}"
    );

    // Malformed YAML names where, not what.
    let why = refusal("agent: [unclosed\n# sk-secret-token-1234\n").await;
    assert!(!why.contains("sk-secret-token"), "{why}");
}
