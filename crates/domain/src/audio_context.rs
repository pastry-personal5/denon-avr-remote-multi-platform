//! Audio observations.  These types deliberately distinguish receiver facts
//! from application inferences; channel maps are not inferred from CV?/MS?.

use super::{FieldError, SurroundMode};

macro_rules! text_value {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $name(String);
        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
                let value = value.into();
                if value.trim().is_empty() {
                    Err("audio context value must not be empty")
                } else {
                    Ok(Self(value))
                }
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}
text_value!(SignalCodec);
text_value!(SignalFormat);
text_value!(InputChannelLayout);
text_value!(ConfiguredSpeakerLayout);
text_value!(OutputChannelLayout);
text_value!(SampleRate);
text_value!(SignalFlags);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation<T> {
    Known(T),
    Unavailable(FieldError),
    NotValidated,
}

impl<T> Observation<T> {
    pub fn known(&self) -> Option<&T> {
        match self {
            Self::Known(value) => Some(value),
            _ => None,
        }
    }
    pub fn is_usable(&self) -> bool {
        matches!(self, Self::Known(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    pub source: &'static str,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    Validated,
    Observed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed<T> {
    pub value: Observation<T>,
    pub provenance: Option<Provenance>,
}

impl<T> Observed<T> {
    pub fn not_validated() -> Self {
        Self {
            value: Observation::NotValidated,
            provenance: None,
        }
    }
    pub fn known(value: T, source: &'static str, confidence: Confidence) -> Self {
        Self {
            value: Observation::Known(value),
            provenance: Some(Provenance { source, confidence }),
        }
    }
    pub fn unavailable(error: FieldError, source: &'static str) -> Self {
        Self {
            value: Observation::Unavailable(error),
            provenance: Some(Provenance {
                source,
                confidence: Confidence::Observed,
            }),
        }
    }
    pub fn malformed(source: &'static str) -> Self {
        Self::unavailable(
            FieldError {
                kind: super::FieldErrorKind::Malformed,
                message: "malformed context response".into(),
            },
            source,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawObservation {
    pub response: Option<String>,
    pub error: Option<String>,
    pub elapsed_millis: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AudioContextSnapshot {
    pub input_selection: Observed<String>,
    pub input_mode: Observed<String>,
    pub digital_mode: Observed<String>,
    pub current_mode: Observed<SurroundMode>,
    pub channel_volume: Observed<String>,
    pub codec: Observed<SignalCodec>,
    pub signal_format: Observed<SignalFormat>,
    pub input_channels: Observed<InputChannelLayout>,
    pub configured_speakers: Observed<ConfiguredSpeakerLayout>,
    pub output_channels: Observed<OutputChannelLayout>,
    pub sample_rate: Observed<SampleRate>,
    pub signal_flags: Observed<SignalFlags>,
    pub raw: Vec<(String, RawObservation)>,
    pub invalidated: bool,
}

impl<T> Default for Observed<T> {
    fn default() -> Self {
        Self::not_validated()
    }
}

impl AudioContextSnapshot {
    pub fn invalidate(&mut self) {
        *self = Self {
            invalidated: true,
            ..Self::default()
        };
    }
    pub fn channel_maps_are_validated(&self) -> bool {
        self.input_channels.value.is_usable() && self.output_channels.value.is_usable()
    }
    pub fn record_raw(&mut self, command: impl Into<String>, observation: RawObservation) {
        self.raw.push((command.into(), observation));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_not_validated_and_trim_is_not_a_channel_map() {
        let snapshot = AudioContextSnapshot::default();
        assert!(matches!(snapshot.codec.value, Observation::NotValidated));
        assert!(!snapshot.channel_maps_are_validated());
        assert!(matches!(
            snapshot.channel_volume.value,
            Observation::NotValidated
        ));
    }
    #[test]
    fn raw_observations_survive_partial_results() {
        let mut snapshot = AudioContextSnapshot::default();
        snapshot.record_raw(
            "SD?",
            RawObservation {
                response: None,
                error: Some("timeout".into()),
                elapsed_millis: 10,
            },
        );
        assert_eq!(snapshot.raw[0].0, "SD?");
        assert_eq!(snapshot.raw[0].1.error.as_deref(), Some("timeout"));
    }

    #[test]
    fn signal_fields_cannot_become_usable_without_validated_provenance() {
        let snapshot = AudioContextSnapshot {
            input_selection: Observed::known("CD".into(), "SI?", Confidence::Observed),
            ..Default::default()
        };
        assert!(!snapshot.channel_maps_are_validated());
        assert!(matches!(
            snapshot.input_selection.value,
            Observation::Known(_)
        ));
    }
}
