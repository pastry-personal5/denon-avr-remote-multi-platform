//! Table cases for the policy engine: the owner's configuration, the shape of
//! the baseline, classification, and what configuration is refused.

mod common;

use common::*;
use denon_avr_domain::{
    CoreField, MasterVolume, MuteState, ReceiverIntent, SoundModeIntent, SourceId, SystemPower,
    ZonePower,
};
use denon_avr_policy::{
    Decision, Effect, IntentKind, IntentValue, PolicyConfig, PolicyError, Reason, RecentChange,
    Rule,
};
use std::time::Duration;

fn baseline_fields(decision: &Decision) -> Vec<CoreField> {
    decision
        .baseline()
        .map(|baseline| baseline.fields().collect())
        .unwrap_or_default()
}

fn rule_ids(decision: &Decision) -> Vec<&str> {
    decision.rules().iter().map(String::as_str).collect()
}

fn agent_decision(intent: &ReceiverIntent, state: &denon_avr_domain::ReceiverState) -> Decision {
    decide(&owners_configuration(), "agent", intent, state, &[])
}

#[test]
fn the_owners_configuration_decides_each_table_case() {
    let config = owners_configuration();
    let at = |db: f64| state_db(db);

    // Volume to -32 dB from -35.
    let d = decide(&config, "agent", &set_volume(-32.0), &at(-35.0), &[]);
    assert_eq!(d.effect(), Effect::Allow);
    assert_eq!(baseline_fields(&d), vec![CoreField::Volume]);

    // Volume to -50 dB, a decrease.
    let d = decide(&config, "agent", &set_volume(-50.0), &at(-35.0), &[]);
    assert_eq!(d.effect(), Effect::Allow);
    assert_eq!(baseline_fields(&d), vec![CoreField::Volume]);

    // Volume to -25 dB: over the ceiling, and a rise of 10 is over the step too.
    let d = decide(&config, "agent", &set_volume(-25.0), &at(-35.0), &[]);
    assert_eq!(d.effect(), Effect::RequireApproval);
    assert_eq!(rule_ids(&d), vec!["volume-ceiling", "volume-step"]);
    assert_eq!(baseline_fields(&d), vec![CoreField::Volume]);

    // Volume to -15 dB: denied, and every other volume rule matched too. A Deny
    // has no baseline.
    let d = decide(&config, "agent", &set_volume(-15.0), &at(-35.0), &[]);
    assert_eq!(d.effect(), Effect::Deny);
    assert_eq!(
        rule_ids(&d),
        vec![
            "volume-hard-limit",
            "volume-ceiling",
            "volume-step",
            "volume-budget"
        ]
    );
    assert_eq!(d.baseline(), None);

    // Volume to -31 dB from -40: a rise of 9, over the step only.
    let d = decide(&config, "agent", &set_volume(-31.0), &at(-40.0), &[]);
    assert_eq!(d.effect(), Effect::RequireApproval);
    assert_eq!(rule_ids(&d), vec!["volume-step"]);
    assert_eq!(baseline_fields(&d), vec![CoreField::Volume]);
}

#[test]
fn the_budget_holds_a_series_of_steps_that_each_pass_the_step_limit() {
    let config = owners_configuration();
    // From -50 dB, within ten minutes: -44, -50, -44, then -38.
    let first = change(minutes(3), Some(-50.0), -44.0);
    let second = change(minutes(2), Some(-44.0), -50.0);
    let third = change(minutes(1), Some(-50.0), -44.0);

    let d = decide(&config, "agent", &set_volume(-44.0), &state_db(-50.0), &[]);
    assert_eq!(d.effect(), Effect::Allow);
    let d = decide(
        &config,
        "agent",
        &set_volume(-50.0),
        &state_db(-44.0),
        &[first],
    );
    assert_eq!(d.effect(), Effect::Allow);
    let d = decide(
        &config,
        "agent",
        &set_volume(-44.0),
        &state_db(-50.0),
        &[first, second],
    );
    assert_eq!(d.effect(), Effect::Allow);

    // The step is 6 and -38 is under the ceiling, but the rise is 12 above the
    // floor of -50.
    let d = decide(
        &config,
        "agent",
        &set_volume(-38.0),
        &state_db(-44.0),
        &[first, second, third],
    );
    assert_eq!(d.effect(), Effect::RequireApproval);
    assert_eq!(rule_ids(&d), vec!["volume-budget"]);
    assert_eq!(baseline_fields(&d), vec![CoreField::Volume]);
}

#[test]
fn the_operators_own_raise_stays_in_the_window() {
    let config = owners_configuration();
    // The Operator raised -60 to -45 two minutes ago.
    let raised = change(minutes(2), Some(-60.0), -45.0);
    let state = state_db(-45.0);

    // A decrease is never over the budget, though -50 is 10 above the floor.
    let d = decide(&config, "agent", &set_volume(-50.0), &state, &[raised]);
    assert_eq!(d.effect(), Effect::Allow);

    // An increase of 0.5 is 15.5 above the floor of -60.
    let d = decide(&config, "agent", &set_volume(-44.5), &state, &[raised]);
    assert_eq!(d.effect(), Effect::RequireApproval);
    assert_eq!(rule_ids(&d), vec!["volume-budget"]);

    // Outside the window the same raise no longer counts.
    let old = change(minutes(11), Some(-60.0), -45.0);
    let d = decide(&config, "agent", &set_volume(-44.5), &state, &[old]);
    assert_eq!(d.effect(), Effect::Allow);
}

#[test]
fn an_unmute_follows_the_volume_it_would_unmute_at() {
    let at_the_ceiling = agent_decision(&unmute(), &state_db(-30.0));
    assert_eq!(at_the_ceiling.effect(), Effect::Allow);

    let above = agent_decision(&unmute(), &state_db(-29.5));
    assert_eq!(above.effect(), Effect::RequireApproval);
    assert_eq!(rule_ids(&above), vec!["loud-or-unknown-baseline"]);

    for state in [
        state_with_stale_volume(volume(-60.0)),
        state_with_unknown_volume(),
        state_with_unavailable_volume(),
    ] {
        let d = agent_decision(&unmute(), &state);
        assert_eq!(d.effect(), Effect::RequireApproval);
    }

    let power_on = ReceiverIntent::MainZonePower(ZonePower::On);
    assert_eq!(
        agent_decision(&power_on, &state_db(-30.0)).effect(),
        Effect::Allow
    );
    assert_eq!(
        agent_decision(&power_on, &state_db(-29.5)).effect(),
        Effect::RequireApproval
    );
}

#[test]
fn an_unmute_that_no_rule_matched_still_carries_the_volume_baseline() {
    let d = agent_decision(&unmute(), &state_db(-45.0));
    assert_eq!(d.effect(), Effect::Allow);
    assert_eq!(
        baseline_fields(&d),
        vec![CoreField::Volume],
        "the loud-baseline rule did not match, but the volume it read must still hold"
    );
}

#[test]
fn a_volume_change_on_an_unusable_volume_needs_approval_even_as_a_decrease() {
    for state in [
        state_with_stale_volume(volume(-35.0)),
        state_with_unknown_volume(),
    ] {
        let d = agent_decision(&set_volume(-50.0), &state);
        assert_eq!(d.effect(), Effect::RequireApproval);
        assert_eq!(rule_ids(&d), vec!["volume-step", "volume-budget"]);
        assert_eq!(baseline_fields(&d), vec![CoreField::Volume]);
    }
}

#[test]
fn low_risk_intents_are_allowed_without_a_baseline() {
    let state = state_db(-35.0);
    for intent in [
        ReceiverIntent::MainZonePower(ZonePower::Off),
        ReceiverIntent::Mute(MuteState::On),
        ReceiverIntent::Source(SourceId::new("TV").unwrap()),
        ReceiverIntent::SoundMode(SoundModeIntent::Stereo),
    ] {
        let d = agent_decision(&intent, &state);
        assert_eq!(d.effect(), Effect::Allow, "{intent:?}");
        assert_eq!(baseline_fields(&d), vec![], "{intent:?}");
    }
}

#[test]
fn system_and_zone2_power_need_approval_whatever_the_volume() {
    for state in [state_db(-60.0), state_with_unknown_volume()] {
        for intent in [
            ReceiverIntent::SystemPower(SystemPower::On),
            ReceiverIntent::SystemPower(SystemPower::Standby),
            ReceiverIntent::Zone2Power(ZonePower::On),
            ReceiverIntent::Zone2Power(ZonePower::Off),
        ] {
            let d = agent_decision(&intent, &state);
            assert_eq!(d.effect(), Effect::RequireApproval, "{intent:?}");
            assert_eq!(baseline_fields(&d), vec![], "{intent:?}");
        }
    }
}

#[test]
fn a_rule_naming_only_an_agent_denies_every_intent_for_that_agent() {
    let mut rules = owners_rules();
    rules.push(Rule::new("claude-code-read-only", Effect::Deny).agents(&["claude-code"]));
    let config = PolicyConfig::new(rules, Duration::from_secs(300)).unwrap();
    let state = state_db(-45.0);

    for intent in every_intent() {
        let d = decide(&config, "claude-code", &intent, &state, &[]);
        assert_eq!(d.effect(), Effect::Deny, "{intent:?}");
        assert!(rule_ids(&d).contains(&"claude-code-read-only"));
        // Another agent keeps the general rules.
        let other = decide(&config, "openclaw", &intent, &state, &[]);
        let general = decide(&owners_configuration(), "openclaw", &intent, &state, &[]);
        assert_eq!(other, general, "{intent:?}");
    }
}

#[test]
fn a_rule_naming_only_an_agent_restricts_without_classifying() {
    // The one rule names no intent and restricts only a loud receiver, so it
    // classifies nothing: on a quiet receiver it does not match, and the intent
    // is unclassified rather than allowed.
    let config = PolicyConfig::new(
        vec![Rule::new("loud-only", Effect::Deny)
            .agents(&["a"])
            .baseline_volume_above(level(-30.0))],
        Duration::from_secs(300),
    )
    .unwrap();
    let source = ReceiverIntent::Source(SourceId::new("TV").unwrap());

    let quiet = decide(&config, "a", &source, &state_db(-50.0), &[]);
    assert_eq!(quiet.effect(), Effect::RequireApproval);
    assert_eq!(quiet.reasons(), &[Reason::Unclassified]);
    assert_eq!(baseline_fields(&quiet), vec![CoreField::Volume]);

    let loud = decide(&config, "a", &source, &state_db(-20.0), &[]);
    assert_eq!(loud.effect(), Effect::Deny);
    assert_eq!(rule_ids(&loud), vec!["loud-only"]);
}

#[test]
fn a_rule_narrowed_to_one_agent_does_not_classify_for_another() {
    let config = PolicyConfig::new(
        vec![Rule::new("a-volume", Effect::Allow)
            .agents(&["a"])
            .intent(IntentKind::Volume)],
        Duration::from_secs(300),
    )
    .unwrap();
    let state = state_db(-50.0);

    let a = decide(&config, "a", &set_volume(-55.0), &state, &[]);
    assert_eq!(a.effect(), Effect::Allow);

    let b = decide(&config, "b", &set_volume(-55.0), &state, &[]);
    assert_eq!(b.effect(), Effect::RequireApproval);
    assert_eq!(b.reasons(), &[Reason::Unclassified]);
    assert!(b.rules().is_empty());
}

#[test]
fn a_rule_narrowed_to_one_receiver_applies_to_that_receiver_only() {
    let config = PolicyConfig::new(
        vec![Rule::new("elsewhere", Effect::Deny)
            .receivers(&["bedroom"])
            .intent(IntentKind::Volume)],
        Duration::from_secs(300),
    )
    .unwrap();
    let d = decide(&config, "a", &set_volume(-50.0), &state_db(-50.0), &[]);
    assert_eq!(d.effect(), Effect::RequireApproval);
    assert_eq!(d.reasons(), &[Reason::Unclassified]);
}

#[test]
fn a_value_scoped_allow_does_not_classify_the_other_value() {
    // Ruling: an allow rule that names a value classifies that value only, so
    // dropping the system-power rule does not make power-on an Allow.
    let config = PolicyConfig::new(
        vec![Rule::new("standby-only", Effect::Allow)
            .intent_value(IntentKind::SystemPower, IntentValue::Standby)],
        Duration::from_secs(300),
    )
    .unwrap();
    let state = state_db(-50.0);

    let standby = decide(
        &config,
        "a",
        &ReceiverIntent::SystemPower(SystemPower::Standby),
        &state,
        &[],
    );
    assert_eq!(standby.effect(), Effect::Allow);

    let on = decide(
        &config,
        "a",
        &ReceiverIntent::SystemPower(SystemPower::On),
        &state,
        &[],
    );
    assert_eq!(on.effect(), Effect::RequireApproval);
    assert_eq!(on.reasons(), &[Reason::Unclassified]);
}

#[test]
fn an_empty_policy_requires_approval_for_everything() {
    let config = PolicyConfig::new(vec![], Duration::from_secs(300)).unwrap();
    for intent in every_intent() {
        let d = decide(&config, "a", &intent, &state_db(-50.0), &[]);
        assert_eq!(d.effect(), Effect::RequireApproval, "{intent:?}");
        assert_eq!(d.reasons(), &[Reason::Unclassified], "{intent:?}");
    }
}

#[test]
fn an_allow_rule_with_a_condition_is_rejected() {
    let with_condition =
        |rule: Rule| PolicyConfig::new(vec![rule], Duration::from_secs(60)).unwrap_err();
    let volume_allow = || Rule::new("loose", Effect::Allow).intent(IntentKind::Volume);

    for rule in [
        volume_allow().target_above(level(-30.0)),
        volume_allow().increase_over_baseline(span(6.0)),
        volume_allow().budget(span(10.0), 10),
        volume_allow().baseline_volume_above(level(-30.0)),
    ] {
        assert_eq!(
            with_condition(rule),
            PolicyError::AllowWithCondition("loose".into())
        );
    }
}

#[test]
fn an_allow_rule_naming_no_intent_is_rejected() {
    let error = PolicyConfig::new(
        vec![Rule::new("everything", Effect::Allow)],
        Duration::from_secs(60),
    )
    .unwrap_err();
    assert_eq!(error, PolicyError::AllowNamesNoIntent("everything".into()));
}

#[test]
fn a_volume_only_condition_on_another_intent_is_rejected() {
    let build = |rule: Rule| PolicyConfig::new(vec![rule], Duration::from_secs(60));
    let mute = || Rule::new("odd", Effect::Deny).intent(IntentKind::Mute);

    for rule in [
        mute().target_above(level(-30.0)),
        mute().increase_over_baseline(span(6.0)),
        mute().budget(span(10.0), 10),
        // One non-volume alternative among volume ones is enough.
        Rule::new("odd", Effect::Deny)
            .intent(IntentKind::Volume)
            .intent(IntentKind::Source)
            .target_above(level(-30.0)),
        // A rule naming no intent cannot carry one either.
        Rule::new("odd", Effect::Deny).target_above(level(-30.0)),
    ] {
        assert_eq!(
            build(rule).unwrap_err(),
            PolicyError::VolumeConditionOnOtherIntent("odd".into())
        );
    }

    // A baseline-volume condition is not volume-only: it reads the volume to
    // decide about another intent.
    assert!(build(mute().baseline_volume_above(level(-30.0))).is_ok());
}

#[test]
fn a_value_that_does_not_belong_to_the_intent_is_rejected() {
    let build = |kind, value| {
        PolicyConfig::new(
            vec![Rule::new("odd", Effect::Deny).intent_value(kind, value)],
            Duration::from_secs(60),
        )
    };
    for (kind, value) in [
        (IntentKind::Volume, IntentValue::On),
        (IntentKind::Source, IntentValue::Off),
        (IntentKind::SoundMode, IntentValue::On),
        (IntentKind::SystemPower, IntentValue::Off),
        (IntentKind::MainZonePower, IntentValue::Standby),
        (IntentKind::Mute, IntentValue::Standby),
    ] {
        assert_eq!(
            build(kind, value).unwrap_err(),
            PolicyError::ValueNotValidForIntent {
                rule: "odd".into(),
                kind,
                value
            }
        );
    }
    for (kind, value) in [
        (IntentKind::SystemPower, IntentValue::On),
        (IntentKind::SystemPower, IntentValue::Standby),
        (IntentKind::MainZonePower, IntentValue::Off),
        (IntentKind::Zone2Power, IntentValue::On),
        (IntentKind::Mute, IntentValue::Off),
    ] {
        assert!(build(kind, value).is_ok(), "{kind:?} {value:?}");
    }
}

#[test]
fn rule_ids_are_unique_and_non_empty_and_lists_are_not_empty() {
    let two = |a: &str, b: &str| {
        PolicyConfig::new(
            vec![
                Rule::new(a, Effect::Deny).intent(IntentKind::Volume),
                Rule::new(b, Effect::Deny).intent(IntentKind::Mute),
            ],
            Duration::from_secs(60),
        )
    };
    assert_eq!(
        two("same", "same").unwrap_err(),
        PolicyError::DuplicateRuleId("same".into())
    );
    assert_eq!(two("", "b").unwrap_err(), PolicyError::EmptyRuleId);
    assert_eq!(two("  ", "b").unwrap_err(), PolicyError::EmptyRuleId);
    assert!(two("a", "b").is_ok());

    // `agents: []` would silently apply to nobody.
    let empty_agents = Rule::new("nobody", Effect::Deny).agents(&[]);
    assert_eq!(
        PolicyConfig::new(vec![empty_agents], Duration::from_secs(60)).unwrap_err(),
        PolicyError::EmptyScopeList("nobody".into())
    );
    let empty_receivers = Rule::new("nowhere", Effect::Deny).receivers(&[]);
    assert_eq!(
        PolicyConfig::new(vec![empty_receivers], Duration::from_secs(60)).unwrap_err(),
        PolicyError::EmptyScopeList("nowhere".into())
    );
}

#[test]
fn a_budget_window_is_one_minute_to_one_day() {
    let build = |minutes: u16| {
        PolicyConfig::new(
            vec![Rule::new("b", Effect::RequireApproval)
                .intent(IntentKind::Volume)
                .budget(span(10.0), minutes)],
            Duration::from_secs(60),
        )
    };
    assert_eq!(
        build(0).unwrap_err(),
        PolicyError::WindowOutOfRange("b".into())
    );
    assert_eq!(
        build(1_441).unwrap_err(),
        PolicyError::WindowOutOfRange("b".into())
    );
    assert!(build(1).is_ok());
    assert_eq!(
        build(1_440).unwrap().longest_window(),
        Duration::from_secs(86_400)
    );
}

#[test]
fn the_longest_window_is_the_widest_budget() {
    assert_eq!(
        owners_configuration().longest_window(),
        Duration::from_secs(600)
    );
    let none = PolicyConfig::new(vec![], Duration::from_secs(60)).unwrap();
    assert_eq!(none.longest_window(), Duration::ZERO);
}

#[test]
fn limits_must_lie_on_the_half_decibel_grid_and_in_range() {
    use denon_avr_policy::{Level, Span};
    assert!(Level::from_db(-20.0).is_ok());
    assert!(Level::from_db(-20.5).is_ok());
    assert!(Level::from_db(-20.25).is_err());
    assert!(Level::from_db(-80.5).is_err());
    assert!(Level::from_db(18.5).is_err());
    assert!(Level::from_db(f64::NAN).is_err());
    assert!(Level::from_db(f64::INFINITY).is_err());
    assert!(Span::from_db(6.0).is_ok());
    assert!(Span::from_db(6.25).is_err());
    assert!(Span::from_db(-0.5).is_err());
    assert!(Span::from_db(99.0).is_err());
}

#[test]
fn minimum_is_a_decrease_as_a_target_and_overstates_a_rise_as_a_baseline() {
    let config = owners_configuration();

    // A target of Minimum is a decrease from every other level.
    let d = decide(
        &config,
        "a",
        &ReceiverIntent::Volume(MasterVolume::Minimum),
        &state_db(-35.0),
        &[],
    );
    assert_eq!(d.effect(), Effect::Allow);

    // Minimum counts as -80.0 dB, half a decibel under -79.5. A rise from it to
    // -73.5 is 6.5 dB, over the 6 dB step, where from -79.5 it would be 6.0.
    let from_minimum = state_at(MasterVolume::Minimum);
    let d = decide(&config, "a", &set_volume(-73.5), &from_minimum, &[]);
    assert_eq!(d.effect(), Effect::RequireApproval);
    assert_eq!(rule_ids(&d), vec!["volume-step"]);
    let d = decide(&config, "a", &set_volume(-74.0), &from_minimum, &[]);
    assert_eq!(d.effect(), Effect::Allow);
}

#[test]
fn every_intent_kind_is_mapped() {
    // An exhaustive match: a new `ReceiverIntent` variant fails to compile here
    // as well as in the policy crate.
    fn expected(intent: &ReceiverIntent) -> IntentKind {
        match intent {
            ReceiverIntent::SystemPower(_) => IntentKind::SystemPower,
            ReceiverIntent::MainZonePower(_) => IntentKind::MainZonePower,
            ReceiverIntent::Zone2Power(_) => IntentKind::Zone2Power,
            ReceiverIntent::Source(_) => IntentKind::Source,
            ReceiverIntent::Volume(_) => IntentKind::Volume,
            ReceiverIntent::Mute(_) => IntentKind::Mute,
            ReceiverIntent::SoundMode(_) => IntentKind::SoundMode,
        }
    }
    for intent in every_intent() {
        assert_eq!(IntentKind::of(&intent), expected(&intent), "{intent:?}");
    }
    let mapped: std::collections::BTreeSet<_> = every_intent().iter().map(IntentKind::of).collect();
    assert_eq!(mapped.len(), IntentKind::ALL.len());
    for kind in IntentKind::ALL {
        assert!(mapped.contains(&kind), "{kind:?}");
        assert_eq!(IntentKind::parse(kind.as_str()), Some(kind));
    }
}

#[test]
fn the_extremes_of_the_scale_do_not_overflow() {
    let extremes = [
        MasterVolume::Minimum,
        MasterVolume::db_half_steps(-159).unwrap(),
        MasterVolume::db_half_steps(36).unwrap(),
    ];
    let config = owners_configuration();
    for baseline in extremes {
        for target in extremes {
            let recent = [
                change(minutes(1), None, -80.0),
                change(minutes(2), Some(18.0), 18.0),
            ];
            let d = decide(
                &config,
                "a",
                &ReceiverIntent::Volume(target),
                &state_at(baseline),
                &recent,
            );
            // Reaching a decision without a panic is the point; the highest target
            // is above the hard limit.
            if target == MasterVolume::db_half_steps(36).unwrap() {
                assert_eq!(d.effect(), Effect::Deny);
            }
        }
    }
}

#[test]
fn every_limit_is_strict_so_a_value_exactly_at_it_is_not_over_it() {
    let config = owners_configuration();
    let outcome = |intent: &ReceiverIntent, observed: f64, recent: &[RecentChange]| {
        let d = decide(&config, "agent", intent, &state_db(observed), recent);
        (
            d.effect(),
            d.rules().iter().map(String::to_string).collect::<Vec<_>>(),
        )
    };
    let only = |effect: Effect, id: &str| (effect, vec![id.to_string()]);
    let allowed = || (Effect::Allow, Vec::<String>::new());

    // The ceiling is -30 dB and the hard limit -20 dB.
    assert_eq!(outcome(&set_volume(-30.0), -35.0, &[]), allowed());
    assert_eq!(
        outcome(&set_volume(-29.5), -35.0, &[]),
        only(Effect::RequireApproval, "volume-ceiling")
    );
    assert_eq!(
        outcome(&set_volume(-20.0), -24.0, &[]),
        only(Effect::RequireApproval, "volume-ceiling")
    );
    assert_eq!(outcome(&set_volume(-19.5), -24.0, &[]).0, Effect::Deny);

    // The step is 6 dB.
    assert_eq!(outcome(&set_volume(-34.0), -40.0, &[]), allowed());
    assert_eq!(
        outcome(&set_volume(-33.5), -40.0, &[]),
        only(Effect::RequireApproval, "volume-step")
    );

    // The budget is 10 dB above the lowest level in the last ten minutes.
    let recent = [change(minutes(1), Some(-54.0), -48.0)];
    assert_eq!(outcome(&set_volume(-44.0), -48.0, &recent), allowed());
    assert_eq!(
        outcome(&set_volume(-43.5), -48.0, &recent),
        only(Effect::RequireApproval, "volume-budget")
    );

    // A change exactly as old as the window has left it; one a millisecond
    // younger is still inside.
    let edge = [change(minutes(10), Some(-54.0), -48.0)];
    assert_eq!(outcome(&set_volume(-43.5), -48.0, &edge), allowed());
    let almost = Duration::from_secs(10 * 60 - 1) + Duration::from_millis(999);
    let inside = [change(almost, Some(-54.0), -48.0)];
    assert_eq!(
        outcome(&set_volume(-43.5), -48.0, &inside),
        only(Effect::RequireApproval, "volume-budget")
    );
}

fn every_intent() -> Vec<ReceiverIntent> {
    vec![
        ReceiverIntent::SystemPower(SystemPower::On),
        ReceiverIntent::SystemPower(SystemPower::Standby),
        ReceiverIntent::MainZonePower(ZonePower::On),
        ReceiverIntent::MainZonePower(ZonePower::Off),
        ReceiverIntent::Zone2Power(ZonePower::On),
        ReceiverIntent::Zone2Power(ZonePower::Off),
        ReceiverIntent::Source(SourceId::new("TV").unwrap()),
        ReceiverIntent::Volume(volume(-45.0)),
        ReceiverIntent::Mute(MuteState::On),
        ReceiverIntent::Mute(MuteState::Off),
        ReceiverIntent::SoundMode(SoundModeIntent::Direct),
    ]
}
