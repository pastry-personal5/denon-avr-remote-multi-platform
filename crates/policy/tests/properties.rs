//! Seeded properties of the policy engine, in the style of the repository's
//! other property tests: a fixed set of seeds, no external generator.

mod common;

use common::*;
use denon_avr_domain::{
    MasterVolume, MuteState, ReceiverIntent, ReceiverState, WallTime, ZonePower,
};
use denon_avr_policy::{Effect, IntentKind, IntentValue, Level, PolicyConfig, RecentChange, Rule};
use std::time::Duration;

const SEEDS: u64 = 64;

fn volume_of(half_steps: i16) -> MasterVolume {
    if half_steps <= -160 {
        MasterVolume::Minimum
    } else {
        half_step_volume(half_steps)
    }
}

fn random_ledger(rng: &mut Seeded, now: WallTime) -> Vec<RecentChange> {
    (0..rng.between(0, 6))
        .map(|_| {
            let ago_seconds = rng.between(0, 20 * 60) as u64;
            let before = rng.between(-159, 36) as i16;
            let target = rng.between(-159, 36) as i16;
            RecentChange {
                at: now.saturating_sub(Duration::from_secs(ago_seconds)),
                before: (!rng.next().is_multiple_of(4))
                    .then(|| Level::from_half_steps(before).unwrap()),
                target: Level::from_half_steps(target).unwrap(),
            }
        })
        .collect()
}

fn unusable_states(rng: &mut Seeded) -> Vec<ReceiverState> {
    let level = volume_of(rng.between(-159, 36) as i16);
    vec![
        state_with_stale_volume(level),
        state_with_unknown_volume(),
        state_with_unavailable_volume(),
    ]
}

#[test]
fn raising_the_target_never_lowers_severity() {
    let config = owners_configuration();
    for seed in 0..SEEDS {
        let mut rng = Seeded::new(seed);
        let ledger = random_ledger(&mut rng, T0);
        let mut states = unusable_states(&mut rng);
        states.push(state_at(volume_of(rng.between(-160, 36) as i16)));

        for state in &states {
            let mut targets: Vec<i16> = (0..8).map(|_| rng.between(-160, 36) as i16).collect();
            targets.sort_unstable();
            let severities: Vec<Effect> = targets
                .iter()
                .map(|&t| {
                    decide(
                        &config,
                        "agent",
                        &ReceiverIntent::Volume(volume_of(t)),
                        state,
                        &ledger,
                    )
                    .effect()
                })
                .collect();
            assert!(
                severities.windows(2).all(|pair| pair[0] <= pair[1]),
                "seed {seed}: targets {targets:?} gave {severities:?}"
            );
        }
    }
}

#[test]
fn a_decrease_from_a_usable_level_is_never_held_by_the_step_or_the_budget() {
    let config = owners_configuration();
    for seed in 0..SEEDS {
        let mut rng = Seeded::new(seed);
        let ledger = random_ledger(&mut rng, T0);
        let observed = rng.between(-159, 36) as i16;
        let state = state_at(volume_of(observed));
        for _ in 0..8 {
            let target = rng.between(-160, observed as i64) as i16;
            let d = decide(
                &config,
                "agent",
                &ReceiverIntent::Volume(volume_of(target)),
                &state,
                &ledger,
            );
            let held = d
                .rules()
                .iter()
                .any(|rule| rule == "volume-step" || rule == "volume-budget");
            assert!(
                !held,
                "seed {seed}: {observed} -> {target} was held by {:?}",
                d.rules()
            );
        }
    }
}

#[test]
fn an_unusable_volume_never_allows_a_state_dependent_rule() {
    let config = owners_configuration();
    for seed in 0..SEEDS {
        let mut rng = Seeded::new(seed);
        let ledger = random_ledger(&mut rng, T0);
        for state in unusable_states(&mut rng) {
            let intents = [
                ReceiverIntent::Volume(volume_of(rng.between(-160, 36) as i16)),
                ReceiverIntent::Mute(MuteState::Off),
                ReceiverIntent::MainZonePower(ZonePower::On),
            ];
            for intent in intents {
                let d = decide(&config, "agent", &intent, &state, &ledger);
                assert_ne!(
                    d.effect(),
                    Effect::Allow,
                    "seed {seed}: {intent:?} was allowed on an unusable volume"
                );
            }
        }
    }
}

#[test]
fn an_allow_rule_never_weakens_a_deny() {
    use IntentKind::*;
    use IntentValue::{Off, On, Standby};
    // Every pair an allow rule may name.
    let pairs = [
        (Volume, None),
        (Source, None),
        (SoundMode, None),
        (SystemPower, None),
        (SystemPower, Some(On)),
        (SystemPower, Some(Standby)),
        (MainZonePower, None),
        (MainZonePower, Some(On)),
        (MainZonePower, Some(Off)),
        (Zone2Power, None),
        (Zone2Power, Some(Off)),
        (Mute, None),
        (Mute, Some(On)),
        (Mute, Some(Off)),
    ];
    for seed in 0..SEEDS {
        let mut rng = Seeded::new(seed);
        let mut rules = owners_rules();
        for n in 0..rng.between(1, 4) {
            let (kind, value) = *rng.pick(&pairs);
            let rule = Rule::new(format!("extra-{n}"), Effect::Allow);
            let rule = match value {
                Some(value) => rule.intent_value(kind, value),
                None => rule.intent(kind),
            };
            // Some extra rules are narrowed to the agent under test.
            rules.push(if rng.next().is_multiple_of(2) {
                rule.agents(&["agent"])
            } else {
                rule
            });
        }
        let widened = PolicyConfig::new(rules, Duration::from_secs(300)).unwrap();
        let owners = owners_configuration();

        let ledger = random_ledger(&mut rng, T0);
        let state = if rng.next().is_multiple_of(3) {
            state_with_unknown_volume()
        } else {
            state_at(volume_of(rng.between(-159, 36) as i16))
        };
        let intents = [
            ReceiverIntent::Volume(volume_of(rng.between(-160, 36) as i16)),
            ReceiverIntent::Mute(MuteState::Off),
            ReceiverIntent::Mute(MuteState::On),
            ReceiverIntent::MainZonePower(ZonePower::On),
            ReceiverIntent::MainZonePower(ZonePower::Off),
            ReceiverIntent::SystemPower(denon_avr_domain::SystemPower::On),
        ];
        for intent in intents {
            let before = decide(&owners, "agent", &intent, &state, &ledger);
            let after = decide(&widened, "agent", &intent, &state, &ledger);
            if before.effect() == Effect::Deny {
                assert_eq!(after.effect(), Effect::Deny, "seed {seed}: {intent:?}");
            }
            // An allow rule can only classify what was unclassified.
            if !before
                .reasons()
                .contains(&denon_avr_policy::Reason::Unclassified)
            {
                assert_eq!(after.effect(), before.effect(), "seed {seed}: {intent:?}");
            }
        }
    }
}

#[test]
fn alternating_steps_never_reset_the_budget() {
    let config = owners_configuration();
    let step = Duration::from_secs(20);
    let window = Duration::from_secs(10 * 60);
    for seed in 0..SEEDS {
        let mut rng = Seeded::new(seed);
        let start = rng.between(-70, -45) as i16;
        let mut now = T0;
        let mut observed = start;
        let mut ledger: Vec<RecentChange> = Vec::new();
        // Every level the receiver held, and when it took it.
        let mut held: Vec<(WallTime, i16)> = vec![(now, start)];

        for _ in 0..60 {
            now = now.saturating_add(step);
            let target = (observed as i64 + rng.between(-24, 16)).clamp(-159, -41) as i16;
            let d = decide_at(
                &config,
                "agent",
                &ReceiverIntent::Volume(volume_of(target)),
                &state_at(volume_of(observed)),
                &ledger,
                now,
            );
            if d.effect() != Effect::Allow {
                continue;
            }
            if target > observed {
                assert!(
                    target - observed <= 12,
                    "seed {seed}: step of {} half dB",
                    target - observed
                );
            }
            ledger.push(RecentChange {
                at: now,
                before: Some(Level::from_half_steps(observed).unwrap()),
                target: Level::from_half_steps(target).unwrap(),
            });
            observed = target;
            held.push((now, target));

            let floor = held
                .iter()
                .filter(|(at, _)| *at > now.saturating_sub(window))
                .map(|(_, level)| *level)
                .chain([observed])
                .min()
                .unwrap();
            assert!(
                observed - floor <= 20,
                "seed {seed}: level {observed} is {} half dB above {floor} inside the window",
                observed - floor
            );
        }
    }
}

#[test]
fn a_future_dated_entry_still_counts() {
    let config = owners_configuration();
    for seed in 0..SEEDS {
        let mut rng = Seeded::new(seed);
        let observed = rng.between(-100, -40) as i16;
        let target = (observed as i64 + rng.between(0, 30)).min(-41) as i16;
        let before = rng.between(-159, -41) as i16;
        let entry = |at: WallTime| RecentChange {
            at,
            before: Some(Level::from_half_steps(before).unwrap()),
            target: Level::from_half_steps(observed).unwrap(),
        };
        let state = state_at(volume_of(observed));
        let intent = ReceiverIntent::Volume(volume_of(target));

        let now_dated = decide(&config, "agent", &intent, &state, &[entry(T0)]);
        let future = T0.saturating_add(Duration::from_millis(rng.between(1, 86_400_000) as u64));
        let future_dated = decide(&config, "agent", &intent, &state, &[entry(future)]);
        assert_eq!(future_dated, now_dated, "seed {seed}");
    }
}

#[test]
fn evaluation_is_deterministic_and_ignores_ledger_order() {
    let config = owners_configuration();
    for seed in 0..SEEDS {
        let mut rng = Seeded::new(seed);
        let ledger = random_ledger(&mut rng, T0);
        let mut reversed = ledger.clone();
        reversed.reverse();
        let state = if rng.next().is_multiple_of(4) {
            state_with_stale_volume(volume_of(-60))
        } else {
            state_at(volume_of(rng.between(-159, 36) as i16))
        };
        let intent = ReceiverIntent::Volume(volume_of(rng.between(-160, 36) as i16));

        let first = decide(&config, "agent", &intent, &state, &ledger);
        assert_eq!(first, decide(&config, "agent", &intent, &state, &ledger));
        assert_eq!(first, decide(&config, "agent", &intent, &state, &reversed));
    }
}
