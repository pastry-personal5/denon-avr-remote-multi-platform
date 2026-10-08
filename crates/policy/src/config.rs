//! A policy as typed data, and the checks that keep a rule from reading like an
//! exception while doing nothing.

use crate::intent::{IntentKind, IntentValue};
use crate::label::label_is_well_formed;
use crate::level::{Level, Span};
use std::collections::BTreeSet;
use std::fmt;
use std::time::Duration;

/// What a matched rule does. Ordered by severity: when several rules match, the
/// greatest wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Effect {
    /// Classifies the intents it names; it never relaxes a restricting rule.
    Allow,
    /// A limit the user may waive for one operation.
    RequireApproval,
    /// A limit approval cannot override.
    Deny,
}

/// Which requests a rule applies to. `None` means every agent, or every receiver.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    pub agents: Option<BTreeSet<String>>,
    pub receivers: Option<BTreeSet<String>>,
}

impl Scope {
    /// Whether the rule applies to `agent` on `receiver`. Labels and ids match
    /// exactly and are case-sensitive.
    pub fn matches(&self, agent: &str, receiver: &str) -> bool {
        self.agents.as_ref().is_none_or(|set| set.contains(agent))
            && self
                .receivers
                .as_ref()
                .is_none_or(|set| set.contains(receiver))
    }
}

/// One intent a rule names: a kind, and optionally one value of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Alternative {
    pub kind: IntentKind,
    pub value: Option<IntentValue>,
}

impl Alternative {
    /// Whether an intent of `kind` carrying `value` is the one this names. With
    /// no value, every value of the kind is.
    pub fn covers(&self, kind: IntentKind, value: Option<IntentValue>) -> bool {
        self.kind == kind && self.value.is_none_or(|named| Some(named) == value)
    }
}

/// A cumulative limit: the rise above the lowest level in a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub rise: Span,
    pub window_minutes: u16,
}

impl Budget {
    pub fn window(&self) -> Duration {
        Duration::from_secs(u64::from(self.window_minutes) * 60)
    }
}

/// What must hold, besides the intent, for a rule to match. All that are set
/// must hold.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Conditions {
    /// The volume target is above this level.
    pub target_above: Option<Level>,
    /// The target is more than this above the observed level, or that level is
    /// unusable.
    pub increase_over_baseline: Option<Span>,
    /// The target is an increase and more than this above the lowest level in the
    /// window, or the observed level is unusable.
    pub budget: Option<Budget>,
    /// The observed volume is above this level, or unusable.
    pub baseline_volume_above: Option<Level>,
}

impl Conditions {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// Conditions about a volume target, which only a volume intent has.
    pub fn about_a_volume_target(&self) -> bool {
        self.target_above.is_some()
            || self.increase_over_baseline.is_some()
            || self.budget.is_some()
    }

    /// Whether the rule reads the observed volume to decide.
    pub fn reads_volume(&self) -> bool {
        self.increase_over_baseline.is_some()
            || self.budget.is_some()
            || self.baseline_volume_above.is_some()
    }
}

/// The intents a rule names and the conditions it adds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    /// Empty means every intent.
    pub alternatives: Vec<Alternative>,
    pub conditions: Conditions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub id: String,
    pub scope: Scope,
    pub filter: Filter,
    pub effect: Effect,
}

impl Rule {
    pub fn new(id: impl Into<String>, effect: Effect) -> Self {
        Self {
            id: id.into(),
            scope: Scope::default(),
            filter: Filter::default(),
            effect,
        }
    }

    pub fn agents(self, labels: &[&str]) -> Self {
        self.agents_from(labels.iter().map(|label| (*label).to_owned()))
    }

    pub fn agents_from(mut self, labels: impl IntoIterator<Item = String>) -> Self {
        self.scope.agents = Some(labels.into_iter().collect());
        self
    }

    pub fn receivers(self, ids: &[&str]) -> Self {
        self.receivers_from(ids.iter().map(|id| (*id).to_owned()))
    }

    pub fn receivers_from(mut self, ids: impl IntoIterator<Item = String>) -> Self {
        self.scope.receivers = Some(ids.into_iter().collect());
        self
    }

    /// Name every value of `kind`.
    pub fn intent(mut self, kind: IntentKind) -> Self {
        self.filter
            .alternatives
            .push(Alternative { kind, value: None });
        self
    }

    /// Name one value of `kind`.
    pub fn intent_value(mut self, kind: IntentKind, value: IntentValue) -> Self {
        self.filter.alternatives.push(Alternative {
            kind,
            value: Some(value),
        });
        self
    }

    pub fn target_above(mut self, limit: Level) -> Self {
        self.filter.conditions.target_above = Some(limit);
        self
    }

    pub fn increase_over_baseline(mut self, limit: Span) -> Self {
        self.filter.conditions.increase_over_baseline = Some(limit);
        self
    }

    pub fn budget(mut self, rise: Span, window_minutes: u16) -> Self {
        self.filter.conditions.budget = Some(Budget {
            rise,
            window_minutes,
        });
        self
    }

    pub fn baseline_volume_above(mut self, limit: Level) -> Self {
        self.filter.conditions.baseline_volume_above = Some(limit);
        self
    }
}

/// Why a set of rules is not a policy. Errors name the rule, never the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    EmptyRuleId,
    DuplicateRuleId(String),
    /// `agents: []` or `receivers: []`, which would apply to nobody.
    EmptyScopeList(String),
    ValueNotValidForIntent {
        rule: String,
        kind: IntentKind,
        value: IntentValue,
    },
    /// An allow rule only classifies, so it carries no condition.
    AllowWithCondition(String),
    /// An allow rule that names no intent would classify nothing.
    AllowNamesNoIntent(String),
    /// A target, step, or budget condition on a rule that does not name only
    /// volume.
    VolumeConditionOnOtherIntent(String),
    /// A budget window outside 1 to 1,440 minutes.
    WindowOutOfRange(String),
    /// A rule's `agents` list names a label that no token could hold, so the rule
    /// would apply to nobody. The error names the rule and not the label, which is
    /// the file's own text.
    AgentLabelNotWellFormed(String),
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyRuleId => f.write_str("a rule has an empty id"),
            Self::DuplicateRuleId(id) => write!(f, "rule id {id:?} is used more than once"),
            Self::EmptyScopeList(id) => {
                write!(
                    f,
                    "rule {id:?} lists no agents or receivers, so applies to none"
                )
            }
            Self::ValueNotValidForIntent { rule, kind, value } => write!(
                f,
                "rule {rule:?} names value {} for {}, which has no such value",
                value.as_str(),
                kind.as_str()
            ),
            Self::AllowWithCondition(id) => write!(
                f,
                "allow rule {id:?} has a condition, but an allow rule only classifies"
            ),
            Self::AllowNamesNoIntent(id) => {
                write!(
                    f,
                    "allow rule {id:?} names no intent, so classifies nothing"
                )
            }
            Self::VolumeConditionOnOtherIntent(id) => write!(
                f,
                "rule {id:?} limits a volume target but does not name only volume"
            ),
            Self::WindowOutOfRange(id) => {
                write!(
                    f,
                    "rule {id:?} has a budget window outside 1 to 1440 minutes"
                )
            }
            Self::AgentLabelNotWellFormed(id) => write!(
                f,
                "rule {id:?} names an agent label that no token could hold; labels are \
                 lowercase letters, digits, '.', '_' and '-', starting with a letter or digit"
            ),
        }
    }
}

impl std::error::Error for PolicyError {}

/// A validated policy: the rules, in order, and how long an approval lasts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyConfig {
    rules: Vec<Rule>,
    approval_lifetime: Duration,
}

impl PolicyConfig {
    pub fn new(rules: Vec<Rule>, approval_lifetime: Duration) -> Result<Self, PolicyError> {
        let mut seen = BTreeSet::new();
        for rule in &rules {
            validate(rule)?;
            if !seen.insert(rule.id.as_str()) {
                return Err(PolicyError::DuplicateRuleId(rule.id.clone()));
            }
        }
        Ok(Self {
            rules,
            approval_lifetime,
        })
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    pub fn approval_lifetime(&self) -> Duration {
        self.approval_lifetime
    }

    /// The widest budget window of any rule, or zero when there is none. The
    /// gate needs no older ledger entry than this.
    pub fn longest_window(&self) -> Duration {
        self.rules
            .iter()
            .filter_map(|rule| rule.filter.conditions.budget)
            .map(|budget| budget.window())
            .max()
            .unwrap_or(Duration::ZERO)
    }
}

const MAX_WINDOW_MINUTES: u16 = 1_440;

fn validate(rule: &Rule) -> Result<(), PolicyError> {
    let id = &rule.id;
    if id.trim().is_empty() {
        return Err(PolicyError::EmptyRuleId);
    }
    let empty = |list: &Option<BTreeSet<String>>| list.as_ref().is_some_and(BTreeSet::is_empty);
    if empty(&rule.scope.agents) || empty(&rule.scope.receivers) {
        return Err(PolicyError::EmptyScopeList(id.clone()));
    }
    if let Some(agents) = &rule.scope.agents {
        if !agents.iter().all(|label| label_is_well_formed(label)) {
            return Err(PolicyError::AgentLabelNotWellFormed(id.clone()));
        }
    }
    for alternative in &rule.filter.alternatives {
        if let Some(value) = alternative.value {
            if !alternative.kind.accepts(value) {
                return Err(PolicyError::ValueNotValidForIntent {
                    rule: id.clone(),
                    kind: alternative.kind,
                    value,
                });
            }
        }
    }
    let conditions = &rule.filter.conditions;
    if rule.effect == Effect::Allow {
        if !conditions.is_empty() {
            return Err(PolicyError::AllowWithCondition(id.clone()));
        }
        if rule.filter.alternatives.is_empty() {
            return Err(PolicyError::AllowNamesNoIntent(id.clone()));
        }
    }
    if conditions.about_a_volume_target() {
        let alternatives = &rule.filter.alternatives;
        let only_volume = !alternatives.is_empty()
            && alternatives
                .iter()
                .all(|alternative| alternative.kind == IntentKind::Volume);
        if !only_volume {
            return Err(PolicyError::VolumeConditionOnOtherIntent(id.clone()));
        }
    }
    if let Some(budget) = conditions.budget {
        if !(1..=MAX_WINDOW_MINUTES).contains(&budget.window_minutes) {
            return Err(PolicyError::WindowOutOfRange(id.clone()));
        }
    }
    Ok(())
}
