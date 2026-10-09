//! The inspection reads: readiness, the Quick Select names, and the HTTP information
//! (channel slots, audio, video, and Audyssey), each field with its status.

use crate::state::{CatalogEvidenceDto, FreshnessDto};
use crate::values::{system_time_from_ms, system_time_ms};
use denon_avr_application::Readiness;
use denon_avr_domain::{
    AudioInformation, AudysseyInformation, ChannelSlot, ChannelSlotState, FieldError,
    FieldErrorKind, FieldStatus, Freshness, HttpInformationSnapshot, QuickSelectName,
    QuickSelectNameObservation, QuickSelectNameResponseEvidence, VideoInformation,
};
use serde::{Deserialize, Serialize};

/// `POST /v1/receivers/{id}/refresh`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessDto {
    pub ready: bool,
    pub degraded: bool,
    pub detail: String,
}

impl From<&Readiness> for ReadinessDto {
    fn from(readiness: &Readiness) -> Self {
        Self {
            ready: readiness.ready,
            degraded: readiness.degraded,
            detail: readiness.detail.clone(),
        }
    }
}

impl From<ReadinessDto> for Readiness {
    fn from(readiness: ReadinessDto) -> Self {
        Self {
            ready: readiness.ready,
            degraded: readiness.degraded,
            detail: readiness.detail,
        }
    }
}

fn freshness_dto(freshness: Freshness) -> FreshnessDto {
    match freshness {
        Freshness::Unknown => FreshnessDto::Unknown,
        Freshness::Live => FreshnessDto::Live,
        Freshness::Partial => FreshnessDto::Partial,
        Freshness::Invalidated => FreshnessDto::Invalidated,
    }
}

fn freshness_of(freshness: FreshnessDto) -> Freshness {
    match freshness {
        FreshnessDto::Unknown => Freshness::Unknown,
        FreshnessDto::Live => Freshness::Live,
        FreshnessDto::Partial => Freshness::Partial,
        FreshnessDto::Invalidated => Freshness::Invalidated,
    }
}

/// The receiver's Quick Select names: four slots, each with a name and the source
/// it recalls, when the receiver reported them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickSelectNamesDto {
    pub names: Vec<Option<String>>,
    pub sources: Vec<Option<String>>,
    pub freshness: FreshnessDto,
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub raw_response: String,
    pub response: CatalogEvidenceDto,
}

impl From<&QuickSelectNameObservation> for QuickSelectNamesDto {
    fn from(observation: &QuickSelectNameObservation) -> Self {
        Self {
            names: observation
                .names
                .iter()
                .map(|name| name.as_ref().map(|name| name.as_str().to_owned()))
                .collect(),
            sources: observation.sources.to_vec(),
            freshness: freshness_dto(observation.freshness),
            generation: observation.generation,
            observed_at_ms: system_time_ms(observation.observed_at),
            error: observation.error.clone(),
            raw_response: observation.raw_response.clone(),
            response: match observation.response_evidence {
                QuickSelectNameResponseEvidence::Complete => CatalogEvidenceDto::Complete,
                QuickSelectNameResponseEvidence::Partial => CatalogEvidenceDto::Partial,
                QuickSelectNameResponseEvidence::Unsupported => CatalogEvidenceDto::Unsupported,
                QuickSelectNameResponseEvidence::Malformed => CatalogEvidenceDto::Malformed,
                QuickSelectNameResponseEvidence::Timeout => CatalogEvidenceDto::Timeout,
                QuickSelectNameResponseEvidence::Disconnected => CatalogEvidenceDto::Disconnected,
            },
        }
    }
}

impl TryFrom<QuickSelectNamesDto> for QuickSelectNameObservation {
    type Error = String;

    fn try_from(dto: QuickSelectNamesDto) -> Result<Self, String> {
        let slots = |what: &str, count: usize| {
            if count == 4 {
                Ok(())
            } else {
                Err(format!("{what} has {count} slots, not four"))
            }
        };
        slots("names", dto.names.len())?;
        slots("sources", dto.sources.len())?;
        let mut names: [Option<QuickSelectName>; 4] = [None, None, None, None];
        for (slot, name) in dto.names.into_iter().enumerate() {
            names[slot] = name
                .map(|name| QuickSelectName::new(name).map_err(str::to_owned))
                .transpose()?;
        }
        let mut sources: [Option<String>; 4] = [None, None, None, None];
        for (slot, source) in dto.sources.into_iter().enumerate() {
            sources[slot] = source;
        }
        Ok(Self {
            names,
            sources,
            freshness: freshness_of(dto.freshness),
            generation: dto.generation,
            observed_at: system_time_from_ms(dto.observed_at_ms),
            error: dto.error,
            raw_response: dto.raw_response,
            response_evidence: match dto.response {
                CatalogEvidenceDto::Complete => QuickSelectNameResponseEvidence::Complete,
                CatalogEvidenceDto::Partial => QuickSelectNameResponseEvidence::Partial,
                CatalogEvidenceDto::Unsupported => QuickSelectNameResponseEvidence::Unsupported,
                CatalogEvidenceDto::Malformed => QuickSelectNameResponseEvidence::Malformed,
                CatalogEvidenceDto::Timeout => QuickSelectNameResponseEvidence::Timeout,
                CatalogEvidenceDto::Disconnected => QuickSelectNameResponseEvidence::Disconnected,
            },
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldErrorKindDto {
    Unsupported,
    Timeout,
    Disconnected,
    Malformed,
    Unavailable,
}

/// A value, or why there is none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldStatusDto<T> {
    Value(T),
    Unavailable {
        kind: FieldErrorKindDto,
        message: String,
    },
}

impl<T: Clone> From<&FieldStatus<T>> for FieldStatusDto<T> {
    fn from(status: &FieldStatus<T>) -> Self {
        match status {
            FieldStatus::Value(value) => Self::Value(value.clone()),
            FieldStatus::Unavailable(error) => Self::Unavailable {
                kind: match error.kind {
                    FieldErrorKind::Unsupported => FieldErrorKindDto::Unsupported,
                    FieldErrorKind::Timeout => FieldErrorKindDto::Timeout,
                    FieldErrorKind::Disconnected => FieldErrorKindDto::Disconnected,
                    FieldErrorKind::Malformed => FieldErrorKindDto::Malformed,
                    FieldErrorKind::Unavailable => FieldErrorKindDto::Unavailable,
                },
                message: error.message.clone(),
            },
        }
    }
}

impl<T> From<FieldStatusDto<T>> for FieldStatus<T> {
    fn from(status: FieldStatusDto<T>) -> Self {
        match status {
            FieldStatusDto::Value(value) => Self::Value(value),
            FieldStatusDto::Unavailable { kind, message } => Self::Unavailable(FieldError {
                kind: match kind {
                    FieldErrorKindDto::Unsupported => FieldErrorKind::Unsupported,
                    FieldErrorKindDto::Timeout => FieldErrorKind::Timeout,
                    FieldErrorKindDto::Disconnected => FieldErrorKind::Disconnected,
                    FieldErrorKindDto::Malformed => FieldErrorKind::Malformed,
                    FieldErrorKindDto::Unavailable => FieldErrorKind::Unavailable,
                },
                message,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotStateDto {
    Active,
    Available,
    Absent,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelSlotDto {
    pub label: String,
    pub state: SlotStateDto,
}

impl From<&ChannelSlot> for ChannelSlotDto {
    fn from(slot: &ChannelSlot) -> Self {
        Self {
            label: slot.label.clone(),
            state: match slot.state {
                ChannelSlotState::Active => SlotStateDto::Active,
                ChannelSlotState::Available => SlotStateDto::Available,
                ChannelSlotState::Absent => SlotStateDto::Absent,
                ChannelSlotState::Unknown => SlotStateDto::Unknown,
            },
        }
    }
}

impl From<ChannelSlotDto> for ChannelSlot {
    fn from(slot: ChannelSlotDto) -> Self {
        Self {
            label: slot.label,
            state: match slot.state {
                SlotStateDto::Active => ChannelSlotState::Active,
                SlotStateDto::Available => ChannelSlotState::Available,
                SlotStateDto::Absent => ChannelSlotState::Absent,
                SlotStateDto::Unknown => ChannelSlotState::Unknown,
            },
        }
    }
}

type Text = FieldStatusDto<String>;

fn text(status: &FieldStatus<String>) -> Text {
    status.into()
}

fn slots(status: &FieldStatus<Vec<ChannelSlot>>) -> FieldStatusDto<Vec<ChannelSlotDto>> {
    match status {
        FieldStatus::Value(slots) => {
            FieldStatusDto::Value(slots.iter().map(ChannelSlotDto::from).collect())
        }
        FieldStatus::Unavailable(error) => {
            FieldStatusDto::from(&FieldStatus::<()>::Unavailable(error.clone())).retype()
        }
    }
}

impl<T> FieldStatusDto<T> {
    /// The same unavailable status for another value type.
    fn retype<U>(self) -> FieldStatusDto<U> {
        match self {
            Self::Value(_) => unreachable!("only an unavailable status is retyped"),
            Self::Unavailable { kind, message } => FieldStatusDto::Unavailable { kind, message },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioDto {
    pub input_mode: Text,
    pub output: Text,
    pub signal: Text,
    pub sound: Text,
    pub sample_rate: Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoDto {
    pub monitor: Text,
    pub hdmi_input: Text,
    pub hdmi_output: Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudysseyDto {
    pub multeq: Text,
    pub dynamic_eq: Text,
    pub dynamic_volume: Text,
}

/// The receiver's audio, video, and Audyssey information, read over HTTP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpInformationDto {
    pub input_slots: FieldStatusDto<Vec<ChannelSlotDto>>,
    pub output_slots: FieldStatusDto<Vec<ChannelSlotDto>>,
    pub audio: AudioDto,
    pub video: VideoDto,
    pub audyssey: AudysseyDto,
    pub freshness: FreshnessDto,
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl From<&HttpInformationSnapshot> for HttpInformationDto {
    fn from(info: &HttpInformationSnapshot) -> Self {
        Self {
            input_slots: slots(&info.input_slots),
            output_slots: slots(&info.output_slots),
            audio: AudioDto {
                input_mode: text(&info.audio.input_mode),
                output: text(&info.audio.output),
                signal: text(&info.audio.signal),
                sound: text(&info.audio.sound),
                sample_rate: text(&info.audio.sample_rate),
            },
            video: VideoDto {
                monitor: text(&info.video.monitor),
                hdmi_input: text(&info.video.hdmi_input),
                hdmi_output: text(&info.video.hdmi_output),
            },
            audyssey: AudysseyDto {
                multeq: text(&info.audyssey.multeq),
                dynamic_eq: text(&info.audyssey.dynamic_eq),
                dynamic_volume: text(&info.audyssey.dynamic_volume),
            },
            freshness: freshness_dto(info.freshness),
            generation: info.generation,
            observed_at_ms: system_time_ms(info.observed_at),
            error: info.error.clone(),
        }
    }
}

impl From<HttpInformationDto> for HttpInformationSnapshot {
    fn from(dto: HttpInformationDto) -> Self {
        let slots = |status: FieldStatusDto<Vec<ChannelSlotDto>>| match status {
            FieldStatusDto::Value(slots) => {
                FieldStatus::Value(slots.into_iter().map(ChannelSlot::from).collect())
            }
            other => other.retype::<()>().into_unavailable(),
        };
        Self {
            input_slots: slots(dto.input_slots),
            output_slots: slots(dto.output_slots),
            audio: AudioInformation {
                input_mode: dto.audio.input_mode.into(),
                output: dto.audio.output.into(),
                signal: dto.audio.signal.into(),
                sound: dto.audio.sound.into(),
                sample_rate: dto.audio.sample_rate.into(),
            },
            video: VideoInformation {
                monitor: dto.video.monitor.into(),
                hdmi_input: dto.video.hdmi_input.into(),
                hdmi_output: dto.video.hdmi_output.into(),
            },
            audyssey: AudysseyInformation {
                multeq: dto.audyssey.multeq.into(),
                dynamic_eq: dto.audyssey.dynamic_eq.into(),
                dynamic_volume: dto.audyssey.dynamic_volume.into(),
            },
            freshness: freshness_of(dto.freshness),
            generation: dto.generation,
            observed_at: system_time_from_ms(dto.observed_at_ms),
            error: dto.error,
        }
    }
}

impl FieldStatusDto<()> {
    /// An unavailable status as the domain's, for any value type.
    fn into_unavailable<T>(self) -> FieldStatus<T> {
        match self {
            Self::Value(()) => unreachable!("only an unavailable status is converted"),
            Self::Unavailable { kind, message } => {
                FieldStatusDto::<T>::Unavailable { kind, message }.into()
            }
        }
    }
}
