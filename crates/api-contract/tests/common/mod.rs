//! Fixtures shared by the contract tests: states with every kind of field, a
//! seeded generator for the property runs, a normalization that compares two
//! states by what a reader reads, and the golden-file check.

#![allow(dead_code)]

use denon_avr_domain::receiver_state::{MainZoneState, ReceiverEvidence};
use denon_avr_domain::{
    CoreField, Epoch, FieldIssue, FieldSynchronization, FrameSeq, MasterVolume, MonotonicMillis,
    MuteState, ObservationOrigin, ReceiverFieldState, ReceiverFieldValidity, ReceiverId,
    ReceiverObservation, ReceiverState, SoundModeStatus, SourceId, StaleReason, StateRevision,
    SystemPower, ZonePower,
};
use std::path::PathBuf;

pub const ADDRESS_TEXT: &str = "connecting to 192.168.1.20:23";
pub const RAW_FRAME: &str = "MVMAX 98\r";

pub fn room() -> ReceiverId {
    ReceiverId::new("living-room").unwrap()
}

pub fn observation<T>(value: T, epoch: u64, seq: u64) -> ReceiverObservation<T> {
    ReceiverObservation {
        receiver: room(),
        epoch: Epoch(epoch),
        frame_seq: FrameSeq(seq),
        observed_at: MonotonicMillis(1_234),
        origin: ObservationOrigin::ReceiverFrame,
        value,
    }
}

pub fn current<T>(value: T) -> ReceiverFieldState<T> {
    let mut field = ReceiverFieldState::default();
    field.observe(observation(value, 3, 9), MonotonicMillis(99_000));
    field
}

pub fn volume(db: f64) -> MasterVolume {
    MasterVolume::db_half_steps((db * 2.0).round() as i16).unwrap()
}

/// One field of every kind, and text in each place a session can leave it: a
/// failed query's message, a raw frame, the receiver's own status text, and its
/// own name for the sound mode.
pub fn sample_state() -> ReceiverState {
    let mut state = ReceiverState::new(room());
    state.revision = StateRevision(42);
    state.epoch = Some(Epoch(3));
    state.system_power = current(SystemPower::On);
    state.main_zone = MainZoneState {
        power: current(ZonePower::On),
        source: current(SourceId::new("BD").unwrap()),
        volume: current(volume(-35.0)),
        mute: {
            let mut field = current(MuteState::On);
            field.stale(StaleReason::Disconnected, ADDRESS_TEXT);
            field
        },
        sound_mode: current(SoundModeStatus {
            id: "DIRECT".into(),
            raw: format!("DIRECT {ADDRESS_TEXT}"),
        }),
    };
    state.zone2_power = ReceiverFieldState {
        validity: ReceiverFieldValidity::Unavailable {
            evidence: ReceiverEvidence::UnavailableStatus(RAW_FRAME.into()),
        },
        last_issue: Some(FieldIssue {
            message: ADDRESS_TEXT.into(),
        }),
        synchronization: FieldSynchronization::Suspended {
            reason: ADDRESS_TEXT.into(),
        },
        ..ReceiverFieldState::default()
    };
    state.diagnostics = vec![format!("malformed: {RAW_FRAME}"), ADDRESS_TEXT.into()];
    state
}

pub const FIELDS: [CoreField; 7] = [
    CoreField::SystemPower,
    CoreField::MainZonePower,
    CoreField::Zone2Power,
    CoreField::Source,
    CoreField::Volume,
    CoreField::Mute,
    CoreField::SoundMode,
];

// ---- A seeded generator, as the repository's other property tests use ----

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn field<T>(rng: &mut Rng, mut value: impl FnMut(&mut Rng) -> T) -> ReceiverFieldState<T> {
    let mut field = ReceiverFieldState::default();
    let has_value = rng.below(4) != 0;
    if has_value {
        field.last_good = Some(observation(value(rng), 1 + rng.below(5), rng.below(100)));
    }
    field.validity = match rng.below(5) {
        0 => ReceiverFieldValidity::Unknown,
        1 | 2 => ReceiverFieldValidity::Current {
            valid_until: MonotonicMillis(rng.below(1_000_000)),
        },
        3 => ReceiverFieldValidity::Stale {
            reason: match rng.below(4) {
                0 => StaleReason::Disconnected,
                1 => StaleReason::QueryFailed,
                2 => StaleReason::Expired,
                _ => StaleReason::ReceiverChanged,
            },
        },
        _ => ReceiverFieldValidity::Unavailable {
            evidence: ReceiverEvidence::UnavailableStatus(format!("status {}", rng.below(1_000))),
        },
    };
    if rng.below(3) == 0 {
        field.last_issue = Some(FieldIssue {
            message: format!("issue {}", rng.below(1_000)),
        });
    }
    field
}

pub fn seeded_state(seed: u64) -> ReceiverState {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut state = ReceiverState::new(room());
    state.revision = StateRevision(rng.below(10_000));
    state.epoch = (rng.below(5) != 0).then(|| Epoch(rng.below(50)));
    state.system_power = field(&mut rng, |r| {
        if r.below(2) == 0 {
            SystemPower::On
        } else {
            SystemPower::Standby
        }
    });
    let zone = |r: &mut Rng| {
        if r.below(2) == 0 {
            ZonePower::On
        } else {
            ZonePower::Off
        }
    };
    state.zone2_power = field(&mut rng, zone);
    state.main_zone = MainZoneState {
        power: field(&mut rng, zone),
        source: field(&mut rng, |r| {
            SourceId::new(["BD", "GAME", "TV AUDIO", "AUX1"][r.below(4) as usize]).unwrap()
        }),
        volume: field(&mut rng, |r| {
            if r.below(10) == 0 {
                MasterVolume::Minimum
            } else {
                MasterVolume::db_half_steps(r.below(196) as i16 - 159).unwrap()
            }
        }),
        mute: field(&mut rng, |r| {
            if r.below(2) == 0 {
                MuteState::On
            } else {
                MuteState::Off
            }
        }),
        sound_mode: field(&mut rng, |r| SoundModeStatus {
            id: format!("MODE{}", r.below(8)),
            raw: format!("raw mode {}", r.below(8)),
        }),
    };
    for n in 0..rng.below(4) {
        state.diagnostics.push(format!("frame {n}"));
    }
    state
}

// ---- What a reader reads ----

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    pub name: &'static str,
    pub value: Option<String>,
    pub validity: &'static str,
    pub reason: Option<String>,
    pub issue: Option<String>,
    pub evidence: Option<String>,
}

fn read<T: std::fmt::Debug>(name: &'static str, field: &ReceiverFieldState<T>) -> Read {
    let (validity, reason, evidence) = match &field.validity {
        ReceiverFieldValidity::Current { .. } => ("current", None, None),
        ReceiverFieldValidity::Stale { reason } => ("stale", Some(format!("{reason:?}")), None),
        ReceiverFieldValidity::Unknown => ("unknown", None, None),
        ReceiverFieldValidity::Unavailable {
            evidence: ReceiverEvidence::UnavailableStatus(text),
        } => ("unavailable", None, Some(text.clone())),
    };
    Read {
        name,
        value: field
            .last_good
            .as_ref()
            .map(|observation| format!("{:?}", observation.value)),
        validity,
        reason,
        issue: field.last_issue.as_ref().map(|issue| issue.message.clone()),
        evidence,
    }
}

/// A state by what a reader reads: the value, the validity class and reason, the
/// epoch and the revision, and, for the Operator, the issue text, the status
/// text, and the diagnostics. The stamps and the synchronization are not read.
pub fn normalize(
    state: &ReceiverState,
    operator: bool,
) -> (Vec<Read>, String, u64, Option<u64>, Vec<String>) {
    let mut reads = vec![
        read("system_power", &state.system_power),
        read("main_zone_power", &state.main_zone.power),
        read("zone2_power", &state.zone2_power),
        read("source", &state.main_zone.source),
        read("volume", &state.main_zone.volume),
        read("mute", &state.main_zone.mute),
        read("sound_mode", &state.main_zone.sound_mode),
    ];
    if !operator {
        for read in &mut reads {
            read.issue = None;
            read.evidence = None;
        }
        // The receiver's own name for the sound mode is text, so an agent's copy
        // has the id alone.
        reads[6].value = state
            .main_zone
            .sound_mode
            .last_good
            .as_ref()
            .map(|observation| format!("{:?}", observation.value.id));
    }
    (
        reads,
        state.receiver.as_str().to_owned(),
        state.revision.0,
        state.epoch.map(|epoch| epoch.0),
        if operator {
            state.diagnostics.clone()
        } else {
            Vec::new()
        },
    )
}

// ---- Golden files ----

pub fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name)
}

/// Compare `text` with the golden file `name`, or write it when
/// `UPDATE_GOLDEN=1`. A golden file is the contract as a reviewer reads it.
pub fn golden(name: &str, text: &str) {
    let path = golden_path(name);
    let text = format!("{}\n", text.trim_end());
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &text).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} is missing; run with UPDATE_GOLDEN=1", path.display()));
    assert_eq!(
        text, expected,
        "{name} changed; review it, then UPDATE_GOLDEN=1"
    );
}

pub fn pretty<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap()
}

// ---- A source catalog with text in every place it can be left ----

pub fn catalog_with_text() -> denon_avr_domain::SourceCatalogObservation {
    use denon_avr_domain::{
        CatalogResponseEvidence, Freshness, SourceCatalog, SourceCatalogObservation, SourceEntry,
        SourceVisibility,
    };
    use std::time::{Duration, UNIX_EPOCH};
    SourceCatalogObservation {
        catalog: SourceCatalog {
            entries: vec![
                SourceEntry {
                    id: SourceId::new("GAME").unwrap(),
                    display_name: Some("Console".into()),
                    visibility: SourceVisibility::Shown,
                },
                SourceEntry {
                    id: SourceId::new("AUX2").unwrap(),
                    display_name: None,
                    visibility: SourceVisibility::Hidden,
                },
            ],
            freshness: Freshness::Partial,
            generation: 4,
            observed_at: Some(UNIX_EPOCH + Duration::from_millis(1_700_000_000_123)),
            error: Some(ADDRESS_TEXT.into()),
        },
        raw_response: format!("<list>{RAW_FRAME}</list>"),
        response_evidence: CatalogResponseEvidence::Partial,
    }
}

/// Every object key in a JSON document.
pub fn keys(value: &serde_json::Value) -> std::collections::BTreeSet<String> {
    fn walk(value: &serde_json::Value, out: &mut std::collections::BTreeSet<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, inner) in map {
                    out.insert(key.clone());
                    walk(inner, out);
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|item| walk(item, out)),
            _ => {}
        }
    }
    let mut out = std::collections::BTreeSet::new();
    walk(value, &mut out);
    out
}
