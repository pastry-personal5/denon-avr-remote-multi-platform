//! Evaluation: which rules apply, which match, and what the decision rests on.

use crate::config::{Effect, PolicyConfig, Rule};
use crate::intent::{IntentKind, IntentValue};
use crate::level::{Level, Span};
use denon_avr_domain::{
    CoreField, FieldBaseline, FieldValue, ReceiverId, ReceiverIntent, ReceiverState, WallTime,
};
use std::collections::BTreeSet;
use std::fmt;

/// A volume change on a receiver: when it was made, the level the receiver showed
/// before it (when that was usable), and the level that was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecentChange {
    pub at: WallTime,
    pub before: Option<Level>,
    pub target: Level,
}

/// Everything a decision depends on.
#[derive(Debug, Clone, Copy)]
pub struct PolicyInput<'a> {
    pub agent: &'a str,
    pub receiver: &'a ReceiverId,
    pub intent: &'a ReceiverIntent,
    pub state: &'a ReceiverState,
    pub recent: &'a [RecentChange],
    pub now: WallTime,
}

/// The state fields a decision read. The gate adds the receiver epoch and turns
/// them into the session request's precondition.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Baseline {
    fields: BTreeSet<CoreField>,
}

impl Baseline {
    pub fn fields(&self) -> impl Iterator<Item = CoreField> + '_ {
        self.fields.iter().copied()
    }

    pub fn contains(&self, field: CoreField) -> bool {
        self.fields.contains(&field)
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

/// Why a rule held a request back, with the numbers that fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    TargetAbove {
        target: Level,
        limit: Level,
    },
    /// `observed` is `None` when the volume was unusable.
    IncreaseOver {
        target: Level,
        observed: Option<Level>,
        limit: Span,
    },
    BudgetExceeded {
        target: Level,
        observed: Option<Level>,
        floor: Option<Level>,
        limit: Span,
        window_minutes: u16,
    },
    BaselineLoudOrUnknown {
        observed: Option<Level>,
        limit: Level,
    },
    /// A matched rule that names no number: the intent itself is restricted.
    IntentRestricted,
    /// No rule names this intent for this agent.
    Unclassified,
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::TargetAbove { target, limit } => {
                write!(f, "the volume target {target} is above the limit {limit}")
            }
            Self::IncreaseOver {
                target,
                observed: Some(observed),
                limit,
            } => write!(
                f,
                "raising the volume from {observed} to {target} is more than the {limit} step"
            ),
            Self::IncreaseOver {
                target,
                observed: None,
                limit,
            } => write!(
                f,
                "the volume is unknown or stale, so a rise to {target} cannot be checked against the {limit} step"
            ),
            Self::BudgetExceeded {
                target,
                floor: Some(floor),
                limit,
                window_minutes,
                ..
            } => write!(
                f,
                "raising the volume to {target} is {} above the lowest level, {floor}, in the last {window_minutes} minutes; the budget is {limit}",
                half_steps_as_db(target.above(floor))
            ),
            Self::BudgetExceeded {
                limit,
                window_minutes,
                ..
            } => write!(
                f,
                "the volume is unknown or stale, so the {limit} budget over {window_minutes} minutes cannot be checked"
            ),
            Self::BaselineLoudOrUnknown {
                observed: Some(observed),
                limit,
            } => write!(f, "the volume is {observed}, above the limit {limit}"),
            Self::BaselineLoudOrUnknown {
                observed: None,
                limit,
            } => write!(f, "the volume is unknown or stale; the limit is {limit}"),
            Self::IntentRestricted => f.write_str("this kind of change is restricted"),
            Self::Unclassified => f.write_str("no rule covers this change"),
        }
    }
}

fn half_steps_as_db(half_steps: i32) -> String {
    format!("{:.1} dB", f64::from(half_steps) / 2.0)
}

/// The outcome of evaluating a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow {
        baseline: Baseline,
    },
    RequireApproval {
        reasons: Vec<Reason>,
        rules: Vec<String>,
        baseline: Baseline,
    },
    Deny {
        reasons: Vec<Reason>,
        rules: Vec<String>,
    },
}

impl Decision {
    pub fn effect(&self) -> Effect {
        match self {
            Self::Allow { .. } => Effect::Allow,
            Self::RequireApproval { .. } => Effect::RequireApproval,
            Self::Deny { .. } => Effect::Deny,
        }
    }

    /// The ids of every rule that matched and restricted the request.
    pub fn rules(&self) -> &[String] {
        match self {
            Self::Allow { .. } => &[],
            Self::RequireApproval { rules, .. } | Self::Deny { rules, .. } => rules,
        }
    }

    pub fn reasons(&self) -> &[Reason] {
        match self {
            Self::Allow { .. } => &[],
            Self::RequireApproval { reasons, .. } | Self::Deny { reasons, .. } => reasons,
        }
    }

    /// What the decision read. A `Deny` is final and has none.
    pub fn baseline(&self) -> Option<&Baseline> {
        match self {
            Self::Allow { baseline } | Self::RequireApproval { baseline, .. } => Some(baseline),
            Self::Deny { .. } => None,
        }
    }
}

/// Decide `input` under `config`.
///
/// 1. A rule *applies* when its scope matches the agent and the receiver.
/// 2. An intent is *classified* when an applicable rule, of any effect, names it:
///    its kind, and its value when the rule gives one. A rule that names no intent
///    restricts but names nothing.
/// 3. For each applicable rule that covers the intent, the state fields its
///    conditions read join the baseline whether or not the rule matches, and then
///    its conditions are tested. A rule matches when all of them hold. An `Allow`
///    rule has no conditions and only classifies.
/// 4. The most restrictive matched effect wins: `Deny`, then `RequireApproval`.
///    With none, an unclassified intent needs approval and a classified one is
///    allowed.
pub fn evaluate(input: &PolicyInput<'_>, config: &PolicyConfig) -> Decision {
    let kind = IntentKind::of(input.intent);
    let value = IntentValue::of(input.intent);
    let observed = observed_volume(input.state);
    let target = match input.intent {
        ReceiverIntent::Volume(volume) => Some(Level::of(*volume)),
        _ => None,
    };

    let mut baseline = Baseline::default();
    let mut classified = false;
    let mut matched: Vec<(&Rule, Vec<Reason>)> = Vec::new();

    for rule in config.rules() {
        if !rule.scope.matches(input.agent, input.receiver.as_str()) {
            continue;
        }
        let alternatives = &rule.filter.alternatives;
        let names_it = alternatives
            .iter()
            .any(|alternative| alternative.covers(kind, value));
        classified |= names_it;
        if !alternatives.is_empty() && !names_it {
            continue;
        }
        if rule.filter.conditions.reads_volume() {
            baseline.fields.insert(CoreField::Volume);
        }
        if rule.effect == Effect::Allow {
            continue;
        }
        if let Some(reasons) = test(rule, input, observed, target) {
            matched.push((rule, reasons));
        }
    }

    let rules: Vec<String> = matched.iter().map(|(rule, _)| rule.id.clone()).collect();
    let reasons: Vec<Reason> = matched
        .iter()
        .flat_map(|(_, reasons)| reasons.iter().copied())
        .collect();

    if matched.iter().any(|(rule, _)| rule.effect == Effect::Deny) {
        Decision::Deny { reasons, rules }
    } else if !matched.is_empty() {
        Decision::RequireApproval {
            reasons,
            rules,
            baseline,
        }
    } else if !classified {
        Decision::RequireApproval {
            reasons: vec![Reason::Unclassified],
            rules,
            baseline,
        }
    } else {
        Decision::Allow { baseline }
    }
}

/// The observed volume, or `None` when it is stale, unknown, or unavailable.
fn observed_volume(state: &ReceiverState) -> Option<Level> {
    match FieldBaseline::capture(state, CoreField::Volume) {
        FieldBaseline::Value(FieldValue::Volume(volume)) => Some(Level::of(volume)),
        _ => None,
    }
}

/// The reasons a rule matches, or `None` when one of its conditions does not
/// hold. A condition that needs the observed volume holds when there is none:
/// what cannot be checked counts as dangerous.
fn test(
    rule: &Rule,
    input: &PolicyInput<'_>,
    observed: Option<Level>,
    target: Option<Level>,
) -> Option<Vec<Reason>> {
    let conditions = &rule.filter.conditions;
    let mut reasons = Vec::new();

    if let Some(limit) = conditions.target_above {
        let target = target?;
        if target <= limit {
            return None;
        }
        reasons.push(Reason::TargetAbove { target, limit });
    }
    if let Some(limit) = conditions.increase_over_baseline {
        let target = target?;
        if let Some(observed) = observed {
            if target.above(observed) <= i32::from(limit.half_steps()) {
                return None;
            }
        }
        reasons.push(Reason::IncreaseOver {
            target,
            observed,
            limit,
        });
    }
    if let Some(budget) = conditions.budget {
        let target = target?;
        let floor = match observed {
            Some(observed) => {
                if target <= observed {
                    return None;
                }
                let floor = floor_in_window(observed, input.recent, input.now, budget.window());
                if target.above(floor) <= i32::from(budget.rise.half_steps()) {
                    return None;
                }
                Some(floor)
            }
            None => None,
        };
        reasons.push(Reason::BudgetExceeded {
            target,
            observed,
            floor,
            limit: budget.rise,
            window_minutes: budget.window_minutes,
        });
    }
    if let Some(limit) = conditions.baseline_volume_above {
        if let Some(observed) = observed {
            if observed <= limit {
                return None;
            }
        }
        reasons.push(Reason::BaselineLoudOrUnknown { observed, limit });
    }

    if conditions.is_empty() {
        reasons.push(Reason::IntentRestricted);
    }
    Some(reasons)
}

/// The lowest of the observed level and, for every change inside the window, the
/// level before it and the level it asked for. A change dated after `now` is
/// inside the window, so a clock that stepped back cannot shrink the budget.
fn floor_in_window(
    observed: Level,
    recent: &[RecentChange],
    now: WallTime,
    window: std::time::Duration,
) -> Level {
    let since = now.saturating_sub(window);
    recent
        .iter()
        .filter(|change| change.at > since)
        .flat_map(|change| change.before.into_iter().chain([change.target]))
        .fold(observed, Level::min)
}
