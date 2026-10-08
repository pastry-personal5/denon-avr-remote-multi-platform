//! The policy file as a [`PolicySource`].
//!
//! The file is read whole, hashed, and parsed into a private shape that refuses
//! any key it does not know (a time-of-day key among them) before it becomes a
//! typed [`PolicyConfig`] through the engine's own constructors, which check the
//! rules again. An error names the rule or the key at fault and never quotes
//! the file.

use denon_avr_application::ports::BoxFuture;
use denon_avr_application::{LoadedPolicy, PolicyDigest, PolicyLoadError, PolicySource};
use denon_avr_policy::{Effect, IntentKind, IntentValue, Level, PolicyConfig, Rule, Span};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncReadExt;

/// The largest policy file that loads, in bytes.
pub const MAX_POLICY_BYTES: usize = 256 * 1024;

/// How long an approval lasts when the file does not say.
const DEFAULT_APPROVAL_MINUTES: u64 = 5;
/// The longest approval lifetime, as long as the longest budget window.
const MAX_APPROVAL_MINUTES: u64 = 1_440;

/// Reads `policy.yaml`, or whichever file it is given.
#[derive(Debug, Clone)]
pub struct YamlPolicySource {
    path: PathBuf,
}

impl YamlPolicySource {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl PolicySource for YamlPolicySource {
    fn load(&self) -> BoxFuture<'_, Result<LoadedPolicy, PolicyLoadError>> {
        Box::pin(async move {
            let bytes = read_bounded(&self.path).await?;
            let digest = digest_of(&bytes);
            let text = String::from_utf8(bytes)
                .map_err(|_| PolicyLoadError::Invalid("the file is not valid UTF-8 text".into()))?;
            let config = parse(&text).map_err(PolicyLoadError::Invalid)?;
            Ok(LoadedPolicy {
                config,
                digest,
                text,
            })
        })
    }
}

async fn read_bounded(path: &Path) -> Result<Vec<u8>, PolicyLoadError> {
    let file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(PolicyLoadError::Missing)
        }
        Err(error) => return Err(PolicyLoadError::Unreadable(error.to_string())),
    };
    // One byte past the limit is enough to tell the file is too big, without
    // reading all of a file that is far bigger.
    let mut bytes = Vec::new();
    file.take(MAX_POLICY_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| PolicyLoadError::Unreadable(error.to_string()))?;
    if bytes.len() > MAX_POLICY_BYTES {
        return Err(PolicyLoadError::Invalid(format!(
            "the file is larger than {} KiB",
            MAX_POLICY_BYTES / 1024
        )));
    }
    Ok(bytes)
}

fn digest_of(bytes: &[u8]) -> PolicyDigest {
    let hash = Sha256::digest(bytes);
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&hash);
    PolicyDigest::from_bytes(digest)
}

// ---- The file's shape ----

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyFile {
    unclassified: Option<String>,
    agent: Option<AgentSection>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct AgentSection {
    approval_lifetime_minutes: Option<serde_yaml::Value>,
    #[serde(default)]
    rules: Vec<RuleFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    id: String,
    when: WhenFile,
    then: EffectFile,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum EffectFile {
    Allow,
    RequireApproval,
    Deny,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct WhenFile {
    intent: Option<String>,
    value: Option<String>,
    any: Option<Vec<AlternativeFile>>,
    agents: Option<Vec<String>>,
    receivers: Option<Vec<String>>,
    target_above_db: Option<f64>,
    increase_over_baseline_db: Option<f64>,
    rise_over_window_db: Option<f64>,
    window_minutes: Option<u64>,
    baseline_volume: Option<BaselineFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AlternativeFile {
    intent: String,
    value: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BaselineFile {
    above_db: f64,
    or_unknown: Option<bool>,
}

fn parse(text: &str) -> Result<PolicyConfig, String> {
    // A file with nothing in it is a policy with no rules, which fails closed.
    let file = serde_yaml::from_str::<Option<PolicyFile>>(text)
        .map_err(|error| error.to_string())?
        .unwrap_or(PolicyFile {
            unclassified: None,
            agent: None,
        });
    if let Some(value) = &file.unclassified {
        if value != "require_approval" {
            return Err(
                "unclassified must be require_approval: a request no rule names \
                        is never allowed"
                    .into(),
            );
        }
    }
    let agent = file.agent.unwrap_or_default();
    let minutes = approval_minutes(agent.approval_lifetime_minutes.as_ref())?;
    let rules = agent
        .rules
        .into_iter()
        .map(convert_rule)
        .collect::<Result<Vec<_>, _>>()?;
    PolicyConfig::new(rules, Duration::from_secs(minutes * 60)).map_err(|error| error.to_string())
}

fn approval_minutes(value: Option<&serde_yaml::Value>) -> Result<u64, String> {
    let Some(value) = value else {
        return Ok(DEFAULT_APPROVAL_MINUTES);
    };
    match value.as_u64() {
        Some(minutes) if (1..=MAX_APPROVAL_MINUTES).contains(&minutes) => Ok(minutes),
        _ => Err(format!(
            "agent.approval_lifetime_minutes must be a whole number from 1 to {MAX_APPROVAL_MINUTES}"
        )),
    }
}

fn convert_rule(file: RuleFile) -> Result<Rule, String> {
    let id = file.id;
    let effect = match file.then {
        EffectFile::Allow => Effect::Allow,
        EffectFile::RequireApproval => Effect::RequireApproval,
        EffectFile::Deny => Effect::Deny,
    };
    let at = |what: &str| format!("rule {id:?}: {what}");
    let when = file.when;
    let mut rule = Rule::new(id.clone(), effect);

    // What the rule names.
    match (&when.intent, &when.any) {
        (Some(_), Some(_)) => return Err(at("names both intent and any; use one")),
        (None, _) if when.value.is_some() => return Err(at("has a value but no intent")),
        _ => {}
    }
    if let Some(name) = &when.intent {
        rule = name_intent(rule, name, when.value.as_deref()).map_err(|why| at(&why))?;
    }
    if let Some(alternatives) = &when.any {
        if alternatives.is_empty() {
            return Err(at("any lists no intents"));
        }
        for alternative in alternatives {
            rule = name_intent(rule, &alternative.intent, alternative.value.as_deref())
                .map_err(|why| at(&why))?;
        }
    }

    // Who it is for.
    if let Some(agents) = when.agents {
        rule = rule.agents_from(agents);
    }
    if let Some(receivers) = when.receivers {
        rule = rule.receivers_from(receivers);
    }

    // What else must hold.
    let level =
        |key: &str, db: f64| Level::from_db(db).map_err(|error| at(&format!("{key} {error}")));
    let distance =
        |key: &str, db: f64| Span::from_db(db).map_err(|error| at(&format!("{key} {error}")));
    if let Some(db) = when.target_above_db {
        rule = rule.target_above(level("target_above_db", db)?);
    }
    if let Some(db) = when.increase_over_baseline_db {
        rule = rule.increase_over_baseline(distance("increase_over_baseline_db", db)?);
    }
    match (when.rise_over_window_db, when.window_minutes) {
        (Some(db), Some(minutes)) => {
            let minutes =
                u16::try_from(minutes).map_err(|_| at("window_minutes must be from 1 to 1440"))?;
            rule = rule.budget(distance("rise_over_window_db", db)?, minutes);
        }
        (None, None) => {}
        _ => {
            return Err(at(
                "a budget needs both rise_over_window_db and window_minutes",
            ))
        }
    }
    if let Some(baseline) = when.baseline_volume {
        if baseline.or_unknown == Some(false) {
            return Err(at(
                "baseline_volume.or_unknown must be true: an unknown or stale volume always matches",
            ));
        }
        rule = rule.baseline_volume_above(level("baseline_volume.above_db", baseline.above_db)?);
    }
    Ok(rule)
}

fn name_intent(rule: Rule, intent: &str, value: Option<&str>) -> Result<Rule, String> {
    let kind = IntentKind::parse(intent).ok_or_else(|| format!("unknown intent {intent}"))?;
    Ok(match value {
        None => rule.intent(kind),
        Some(value) => {
            let value =
                IntentValue::parse(value).ok_or_else(|| format!("unknown value {value}"))?;
            rule.intent_value(kind, value)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_digest_is_sha256_of_the_bytes() {
        // The standard test vector for "abc".
        assert_eq!(
            digest_of(b"abc").to_string(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
