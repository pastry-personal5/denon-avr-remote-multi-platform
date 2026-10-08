//! Shared builders for the policy tests: receiver states, the owner's
//! configuration, and a seeded generator.

#![allow(dead_code)]

use denon_avr_domain::{
    Epoch, FrameSeq, MasterVolume, MonotonicMillis, MuteState, ObservationOrigin,
    ReceiverFieldValidity, ReceiverId, ReceiverIntent, ReceiverObservation, ReceiverState,
    StaleReason, WallTime,
};
use denon_avr_policy::{
    evaluate, Decision, Effect, IntentKind, IntentValue, Level, PolicyConfig, PolicyInput,
    RecentChange, Rule, Span,
};
use std::time::Duration;

pub fn receiver() -> ReceiverId {
    ReceiverId::new("living-room").unwrap()
}

fn observed<T>(value: T) -> ReceiverObservation<T> {
    ReceiverObservation {
        receiver: receiver(),
        epoch: Epoch(1),
        frame_seq: FrameSeq(1),
        observed_at: MonotonicMillis(0),
        origin: ObservationOrigin::ReceiverFrame,
        value,
    }
}

/// A state with an established epoch and a current volume.
pub fn state_at(volume: MasterVolume) -> ReceiverState {
    let mut state = ReceiverState::new(receiver());
    state.establish_epoch(Epoch(1));
    state
        .main_zone
        .volume
        .observe(observed(volume), MonotonicMillis(1_000));
    state
}

/// A state whose volume was observed and has since gone stale.
pub fn state_with_stale_volume(volume: MasterVolume) -> ReceiverState {
    let mut state = state_at(volume);
    state
        .main_zone
        .volume
        .stale(StaleReason::Disconnected, "gone");
    state
}

/// A state whose volume was never observed.
pub fn state_with_unknown_volume() -> ReceiverState {
    let mut state = ReceiverState::new(receiver());
    state.establish_epoch(Epoch(1));
    state
}

/// A state whose volume the receiver reports as unavailable.
pub fn state_with_unavailable_volume() -> ReceiverState {
    let mut state = state_at(MasterVolume::db_half_steps(-60).unwrap());
    state.main_zone.volume.validity = ReceiverFieldValidity::Unavailable {
        evidence: denon_avr_domain::receiver_state::ReceiverEvidence::UnavailableStatus(
            "MV?".into(),
        ),
    };
    state
}

pub fn half_steps(db: f64) -> i16 {
    (db * 2.0).round() as i16
}

pub fn volume(db: f64) -> MasterVolume {
    MasterVolume::db_half_steps(half_steps(db)).unwrap()
}

pub fn state_db(db: f64) -> ReceiverState {
    state_at(volume(db))
}

pub fn set_volume(db: f64) -> ReceiverIntent {
    ReceiverIntent::Volume(volume(db))
}

pub fn level(db: f64) -> Level {
    Level::from_db(db).unwrap()
}

pub fn span(db: f64) -> Span {
    Span::from_db(db).unwrap()
}

pub fn unmute() -> ReceiverIntent {
    ReceiverIntent::Mute(MuteState::Off)
}

pub const T0: WallTime = WallTime(10_000_000);

pub fn minutes(n: u64) -> Duration {
    Duration::from_secs(n * 60)
}

/// A change made `ago` before `T0`, from `before` dB to `target` dB.
pub fn change(ago: Duration, before: Option<f64>, target: f64) -> RecentChange {
    RecentChange {
        at: T0.saturating_sub(ago),
        before: before.map(level),
        target: level(target),
    }
}

pub fn decide(
    config: &PolicyConfig,
    agent: &str,
    intent: &ReceiverIntent,
    state: &ReceiverState,
    recent: &[RecentChange],
) -> Decision {
    decide_at(config, agent, intent, state, recent, T0)
}

pub fn decide_at(
    config: &PolicyConfig,
    agent: &str,
    intent: &ReceiverIntent,
    state: &ReceiverState,
    recent: &[RecentChange],
    now: WallTime,
) -> Decision {
    evaluate(
        &PolicyInput {
            agent,
            receiver: &receiver(),
            intent,
            state,
            recent,
            now,
        },
        config,
    )
}

/// The owner's configuration, as in the architecture's sample policy file.
pub fn owners_rules() -> Vec<Rule> {
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

pub fn owners_configuration() -> PolicyConfig {
    PolicyConfig::new(owners_rules(), Duration::from_secs(300)).unwrap()
}

/// A small deterministic generator, in the style of the repository's other
/// seeded property tests.
pub struct Seeded(u64);

impl Seeded {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03)
    }

    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A value in `low..=high`.
    pub fn between(&mut self, low: i64, high: i64) -> i64 {
        low + (self.next() % ((high - low + 1) as u64)) as i64
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[(self.next() % items.len() as u64) as usize]
    }
}

pub fn half_step_volume(half_steps: i16) -> MasterVolume {
    MasterVolume::db_half_steps(half_steps).unwrap()
}
