//! Read-only, model-specific receiver information reported by AppCommand.

use super::{FieldError, FieldErrorKind, FieldStatus, Freshness};
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelSlotState {
    Active,
    Available,
    Absent,
    /// The receiver returned a state this version does not understand.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSlot {
    pub label: String,
    pub state: ChannelSlotState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioInformation {
    pub input_mode: FieldStatus<String>,
    pub output: FieldStatus<String>,
    pub signal: FieldStatus<String>,
    pub sound: FieldStatus<String>,
    pub sample_rate: FieldStatus<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoInformation {
    pub monitor: FieldStatus<String>,
    pub hdmi_input: FieldStatus<String>,
    pub hdmi_output: FieldStatus<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudysseyInformation {
    pub multeq: FieldStatus<String>,
    pub dynamic_eq: FieldStatus<String>,
    pub dynamic_volume: FieldStatus<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpInformationSnapshot {
    pub input_slots: FieldStatus<Vec<ChannelSlot>>,
    pub output_slots: FieldStatus<Vec<ChannelSlot>>,
    pub audio: AudioInformation,
    pub video: VideoInformation,
    pub audyssey: AudysseyInformation,
    pub freshness: Freshness,
    pub generation: u64,
    pub observed_at: Option<SystemTime>,
    pub error: Option<String>,
}

fn unavailable() -> FieldError {
    FieldError {
        kind: FieldErrorKind::Unavailable,
        message: "not queried".into(),
    }
}

impl Default for AudioInformation {
    fn default() -> Self {
        Self {
            input_mode: FieldStatus::Unavailable(unavailable()),
            output: FieldStatus::Unavailable(unavailable()),
            signal: FieldStatus::Unavailable(unavailable()),
            sound: FieldStatus::Unavailable(unavailable()),
            sample_rate: FieldStatus::Unavailable(unavailable()),
        }
    }
}
impl Default for VideoInformation {
    fn default() -> Self {
        Self {
            monitor: FieldStatus::Unavailable(unavailable()),
            hdmi_input: FieldStatus::Unavailable(unavailable()),
            hdmi_output: FieldStatus::Unavailable(unavailable()),
        }
    }
}
impl Default for AudysseyInformation {
    fn default() -> Self {
        Self {
            multeq: FieldStatus::Unavailable(unavailable()),
            dynamic_eq: FieldStatus::Unavailable(unavailable()),
            dynamic_volume: FieldStatus::Unavailable(unavailable()),
        }
    }
}
impl Default for HttpInformationSnapshot {
    fn default() -> Self {
        Self {
            input_slots: FieldStatus::Unavailable(unavailable()),
            output_slots: FieldStatus::Unavailable(unavailable()),
            audio: AudioInformation::default(),
            video: VideoInformation::default(),
            audyssey: AudysseyInformation::default(),
            freshness: Freshness::Unknown,
            generation: 0,
            observed_at: None,
            error: None,
        }
    }
}
impl HttpInformationSnapshot {
    pub fn invalidate(&mut self, generation: u64) {
        *self = Self {
            freshness: Freshness::Invalidated,
            generation,
            ..Self::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalidation_clears_receiver_values() {
        let mut snapshot = HttpInformationSnapshot {
            input_slots: FieldStatus::Value(vec![ChannelSlot {
                label: "FL".into(),
                state: ChannelSlotState::Active,
            }]),
            ..Default::default()
        };
        snapshot.invalidate(4);
        assert!(matches!(snapshot.input_slots, FieldStatus::Unavailable(_)));
        assert_eq!(snapshot.freshness, Freshness::Invalidated);
    }
}
