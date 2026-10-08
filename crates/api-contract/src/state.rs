//! Receiver state and the source catalog on the wire, as two views.
//!
//! The session's `ReceiverState` carries free text that can name an address: the
//! message of a failed query, a raw frame, the receiver's own status text. An
//! agent must never read it. So a view is generic over the text it carries:
//! [`NoText`] has no field to put a string in, and the Agent endpoint serves only
//! views of that type, while [`FieldText`] and [`Diagnostics`] carry the text for
//! the Operator. The type decides what can leave, not a filter that could be
//! forgotten.
//!
//! The wire carries a field's validity class and a reason, not its timestamps,
//! because the session counts time from its own start and a stamp means nothing to
//! a client. A client that rebuilds a state fills the stamps with placeholders;
//! what reads a state (`FieldBaseline::capture` and the GUI's projection) reads the
//! class, the last good value, and the issue text.

use crate::receivers::parse_receiver_id;
use crate::values::{
    system_time_from_ms, system_time_ms, MuteDto, SystemPowerDto, VolumeDto, ZonePowerDto,
};
use denon_avr_domain::receiver_state::{MainZoneState, ReceiverEvidence};
use denon_avr_domain::{
    CatalogResponseEvidence, Epoch, FieldIssue, FrameSeq, Freshness, MasterVolume, MonotonicMillis,
    MuteState, ObservationOrigin, ReceiverFieldState, ReceiverFieldValidity, ReceiverId,
    ReceiverObservation, ReceiverState, SoundModeStatus, SourceCatalog, SourceCatalogObservation,
    SourceEntry, SourceId, SourceVisibility, StaleReason, StateRevision, SystemPower, ZonePower,
};
use serde::{Deserialize, Serialize};

/// A view that cannot be read back into a state or a catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewError(pub String);

impl std::fmt::Display for ViewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ViewError {}

impl From<String> for ViewError {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ViewError {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// The text a view carries and the Agent's does not.
///
/// Used where a view has nothing to add: it serializes to nothing and has no
/// field to hold a string.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoText {}

/// What the Operator sees of one field beyond its value: the message of the last
/// failed query, the receiver's own status text when it reported the field
/// unavailable, and, for the sound mode, the receiver's own name for the value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldText {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

/// The raw lines the session kept for frames it could not use. Operator only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostics {
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

/// What a view adds to each field.
pub trait Detail: Default + Clone {
    fn about<T>(field: &ReceiverFieldState<T>, raw: Option<&str>) -> Self;
    fn issue(&self) -> Option<&str> {
        None
    }
    fn evidence(&self) -> Option<&str> {
        None
    }
    fn raw(&self) -> Option<&str> {
        None
    }
}

impl Detail for NoText {
    fn about<T>(_: &ReceiverFieldState<T>, _: Option<&str>) -> Self {
        Self {}
    }
}

impl Detail for FieldText {
    fn about<T>(field: &ReceiverFieldState<T>, raw: Option<&str>) -> Self {
        let evidence = match &field.validity {
            ReceiverFieldValidity::Unavailable {
                evidence: ReceiverEvidence::UnavailableStatus(text),
            } => Some(text.clone()),
            _ => None,
        };
        Self {
            issue: field.last_issue.as_ref().map(|issue| issue.message.clone()),
            evidence,
            raw: raw.map(str::to_owned),
        }
    }

    fn issue(&self) -> Option<&str> {
        self.issue.as_deref()
    }

    fn evidence(&self) -> Option<&str> {
        self.evidence.as_deref()
    }

    fn raw(&self) -> Option<&str> {
        self.raw.as_deref()
    }
}

/// What a view adds to the state as a whole.
pub trait Extras: Default + Clone {
    fn of(state: &ReceiverState) -> Self;
    fn diagnostics(&self) -> Vec<String> {
        Vec::new()
    }
}

impl Extras for NoText {
    fn of(_: &ReceiverState) -> Self {
        Self {}
    }
}

impl Extras for Diagnostics {
    fn of(state: &ReceiverState) -> Self {
        Self {
            diagnostics: state.diagnostics.clone(),
        }
    }

    fn diagnostics(&self) -> Vec<String> {
        self.diagnostics.clone()
    }
}

/// Whether a field's value can be acted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidityDto {
    Current,
    Stale,
    Unknown,
    Unavailable,
}

/// Why a stale field is stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonDto {
    Disconnected,
    QueryFailed,
    Expired,
    ReceiverChanged,
}

impl From<&StaleReason> for ReasonDto {
    fn from(value: &StaleReason) -> Self {
        match value {
            StaleReason::Disconnected => Self::Disconnected,
            StaleReason::QueryFailed => Self::QueryFailed,
            StaleReason::Expired => Self::Expired,
            StaleReason::ReceiverChanged => Self::ReceiverChanged,
        }
    }
}

impl From<ReasonDto> for StaleReason {
    fn from(value: ReasonDto) -> Self {
        match value {
            ReasonDto::Disconnected => Self::Disconnected,
            ReasonDto::QueryFailed => Self::QueryFailed,
            ReasonDto::Expired => Self::Expired,
            ReasonDto::ReceiverChanged => Self::ReceiverChanged,
        }
    }
}

/// One field: its last good value, whether it can be acted on, and, for the
/// Operator, the text about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(bound(
    serialize = "T: Serialize, F: Serialize",
    deserialize = "T: Deserialize<'de>, F: Deserialize<'de>"
))]
pub struct FieldView<T, F> {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<T>,
    pub validity: ValidityDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<ReasonDto>,
    #[serde(flatten)]
    pub text: F,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundModeValueDto {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MainZoneView<F> {
    pub power: FieldView<ZonePowerDto, F>,
    pub source: FieldView<String, F>,
    pub volume: FieldView<VolumeDto, F>,
    pub mute: FieldView<MuteDto, F>,
    pub sound_mode: FieldView<SoundModeValueDto, F>,
}

/// A receiver's complete state, with each field's validity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateView<F, E> {
    pub receiver: String,
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
    pub system_power: FieldView<SystemPowerDto, F>,
    pub main_zone: MainZoneView<F>,
    pub zone2_power: FieldView<ZonePowerDto, F>,
    #[serde(flatten)]
    pub extras: E,
}

/// What an agent is served: codes and values, and no text.
pub type AgentStateView = StateView<NoText, NoText>;

/// What the Operator is served: the same, with the text.
pub type OperatorStateView = StateView<FieldText, Diagnostics>;

fn view<T, U, F: Detail>(
    field: &ReceiverFieldState<T>,
    value: impl Fn(&T) -> U,
    raw: Option<&dyn Fn(&T) -> String>,
) -> FieldView<U, F> {
    let (validity, reason) = match &field.validity {
        ReceiverFieldValidity::Current { .. } => (ValidityDto::Current, None),
        ReceiverFieldValidity::Stale { reason } => (ValidityDto::Stale, Some(reason.into())),
        ReceiverFieldValidity::Unknown => (ValidityDto::Unknown, None),
        ReceiverFieldValidity::Unavailable { .. } => (ValidityDto::Unavailable, None),
    };
    let raw = field
        .last_good
        .as_ref()
        .and_then(|observation| raw.map(|raw| raw(&observation.value)));
    FieldView {
        value: field
            .last_good
            .as_ref()
            .map(|observation| value(&observation.value)),
        validity,
        reason,
        text: F::about(field, raw.as_deref()),
    }
}

impl<F: Detail, E: Extras> From<&ReceiverState> for StateView<F, E> {
    fn from(state: &ReceiverState) -> Self {
        let zone = |power: &ZonePower| ZonePowerDto::from(*power);
        let sound_mode_id = |mode: &SoundModeStatus| SoundModeValueDto {
            id: mode.id.clone(),
        };
        let sound_mode_raw = |mode: &SoundModeStatus| mode.raw.clone();
        Self {
            receiver: state.receiver.as_str().to_owned(),
            revision: state.revision.0,
            epoch: state.epoch.map(|epoch| epoch.0),
            system_power: view(
                &state.system_power,
                |power: &SystemPower| SystemPowerDto::from(*power),
                None,
            ),
            main_zone: MainZoneView {
                power: view(&state.main_zone.power, zone, None),
                source: view(
                    &state.main_zone.source,
                    |source: &SourceId| source.as_str().to_owned(),
                    None,
                ),
                volume: view(
                    &state.main_zone.volume,
                    |volume: &MasterVolume| VolumeDto::from(*volume),
                    None,
                ),
                mute: view(
                    &state.main_zone.mute,
                    |mute: &MuteState| MuteDto::from(*mute),
                    None,
                ),
                sound_mode: view(
                    &state.main_zone.sound_mode,
                    sound_mode_id,
                    Some(&sound_mode_raw),
                ),
            },
            zone2_power: view(&state.zone2_power, zone, None),
            extras: E::of(state),
        }
    }
}

/// Rebuild one field. The stamps are placeholders: the observation's epoch is the
/// state's, its sequence and time are zero, and a current field never expires
/// here, because the server sends a new snapshot when it does.
fn rebuild<T, U, F: Detail>(
    receiver: &ReceiverId,
    epoch: Epoch,
    view: &FieldView<U, F>,
    convert: impl Fn(&U, &F) -> Result<T, ViewError>,
) -> Result<ReceiverFieldState<T>, ViewError> {
    let mut field = ReceiverFieldState::default();
    if let Some(value) = &view.value {
        field.last_good = Some(ReceiverObservation {
            receiver: receiver.clone(),
            epoch,
            frame_seq: FrameSeq(0),
            observed_at: MonotonicMillis(0),
            origin: ObservationOrigin::ReceiverFrame,
            value: convert(value, &view.text)?,
        });
    }
    field.validity = match view.validity {
        ValidityDto::Current => ReceiverFieldValidity::Current {
            valid_until: MonotonicMillis(u64::MAX),
        },
        ValidityDto::Stale => ReceiverFieldValidity::Stale {
            reason: view
                .reason
                .ok_or("a stale field says why it is stale")?
                .into(),
        },
        ValidityDto::Unknown => ReceiverFieldValidity::Unknown,
        ValidityDto::Unavailable => ReceiverFieldValidity::Unavailable {
            evidence: ReceiverEvidence::UnavailableStatus(
                view.text.evidence().unwrap_or_default().to_owned(),
            ),
        },
    };
    field.last_issue = view.text.issue().map(|message| FieldIssue {
        message: message.to_owned(),
    });
    Ok(field)
}

impl<F: Detail, E: Extras> StateView<F, E> {
    /// The state this view describes, with placeholder stamps.
    pub fn into_state(self) -> Result<ReceiverState, ViewError> {
        let receiver = parse_receiver_id(&self.receiver)?;
        let epoch = Epoch(self.epoch.unwrap_or(0));
        let mut state = ReceiverState::new(receiver.clone());
        state.revision = StateRevision(self.revision);
        state.epoch = self.epoch.map(Epoch);
        state.system_power = rebuild(&receiver, epoch, &self.system_power, |v, _| {
            Ok(SystemPower::from(*v))
        })?;
        let zone = |v: &ZonePowerDto, _: &F| Ok(ZonePower::from(*v));
        state.zone2_power = rebuild(&receiver, epoch, &self.zone2_power, zone)?;
        let main = &self.main_zone;
        state.main_zone = MainZoneState {
            power: rebuild(&receiver, epoch, &main.power, zone)?,
            source: rebuild(&receiver, epoch, &main.source, |v, _| {
                SourceId::new(v.clone()).map_err(ViewError::from)
            })?,
            volume: rebuild(&receiver, epoch, &main.volume, |v, _| {
                MasterVolume::try_from(*v).map_err(ViewError::from)
            })?,
            mute: rebuild(&receiver, epoch, &main.mute, |v, _| Ok(MuteState::from(*v)))?,
            sound_mode: rebuild(&receiver, epoch, &main.sound_mode, |v, text| {
                Ok(SoundModeStatus {
                    id: v.id.clone(),
                    raw: text.raw().unwrap_or(&v.id).to_owned(),
                })
            })?,
        };
        state.diagnostics = self.extras.diagnostics();
        Ok(state)
    }
}

// ---- The source catalog ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityDto {
    Shown,
    Hidden,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessDto {
    Unknown,
    Live,
    Partial,
    Invalidated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogEvidenceDto {
    Complete,
    Partial,
    Unsupported,
    Malformed,
    Timeout,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceEntryDto {
    pub id: String,
    /// The receiver's own label for the source, which an agent needs to name it
    /// to the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub visibility: VisibilityDto,
}

/// What the Operator sees of a catalog beyond its entries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcesText {
    #[serde(default)]
    pub raw_response: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What a catalog view adds.
pub trait SourcesDetail: Default + Clone {
    fn of(observation: &SourceCatalogObservation) -> Self;
    fn raw_response(&self) -> String {
        String::new()
    }
    fn error(&self) -> Option<String> {
        None
    }
}

impl SourcesDetail for NoText {
    fn of(_: &SourceCatalogObservation) -> Self {
        Self {}
    }
}

impl SourcesDetail for SourcesText {
    fn of(observation: &SourceCatalogObservation) -> Self {
        Self {
            raw_response: observation.raw_response.clone(),
            error: observation.catalog.error.clone(),
        }
    }

    fn raw_response(&self) -> String {
        self.raw_response.clone()
    }

    fn error(&self) -> Option<String> {
        self.error.clone()
    }
}

/// The receiver's source names and visibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcesView<X> {
    pub entries: Vec<SourceEntryDto>,
    pub freshness: FreshnessDto,
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at_ms: Option<u64>,
    /// How the receiver's reply was read: whole, partial, or not at all.
    pub response: CatalogEvidenceDto,
    #[serde(flatten)]
    pub extra: X,
}

pub type AgentSourcesView = SourcesView<NoText>;
pub type OperatorSourcesView = SourcesView<SourcesText>;

impl<X: SourcesDetail> From<&SourceCatalogObservation> for SourcesView<X> {
    fn from(observation: &SourceCatalogObservation) -> Self {
        let catalog = &observation.catalog;
        Self {
            entries: catalog
                .entries
                .iter()
                .map(|entry| SourceEntryDto {
                    id: entry.id.as_str().to_owned(),
                    display_name: entry.display_name.clone(),
                    visibility: match entry.visibility {
                        SourceVisibility::Shown => VisibilityDto::Shown,
                        SourceVisibility::Hidden => VisibilityDto::Hidden,
                        SourceVisibility::Unknown => VisibilityDto::Unknown,
                    },
                })
                .collect(),
            freshness: match catalog.freshness {
                Freshness::Unknown => FreshnessDto::Unknown,
                Freshness::Live => FreshnessDto::Live,
                Freshness::Partial => FreshnessDto::Partial,
                Freshness::Invalidated => FreshnessDto::Invalidated,
            },
            generation: catalog.generation,
            observed_at_ms: system_time_ms(catalog.observed_at),
            response: match observation.response_evidence {
                CatalogResponseEvidence::Complete => CatalogEvidenceDto::Complete,
                CatalogResponseEvidence::Partial => CatalogEvidenceDto::Partial,
                CatalogResponseEvidence::Unsupported => CatalogEvidenceDto::Unsupported,
                CatalogResponseEvidence::Malformed => CatalogEvidenceDto::Malformed,
                CatalogResponseEvidence::Timeout => CatalogEvidenceDto::Timeout,
                CatalogResponseEvidence::Disconnected => CatalogEvidenceDto::Disconnected,
            },
            extra: X::of(observation),
        }
    }
}

impl<X: SourcesDetail> SourcesView<X> {
    /// The observation this view describes. An agent's view has no raw reply or
    /// error text, so those are empty.
    pub fn into_observation(self) -> Result<SourceCatalogObservation, ViewError> {
        let entries = self
            .entries
            .into_iter()
            .map(|entry| {
                Ok(SourceEntry {
                    id: SourceId::new(entry.id)?,
                    display_name: entry.display_name,
                    visibility: match entry.visibility {
                        VisibilityDto::Shown => SourceVisibility::Shown,
                        VisibilityDto::Hidden => SourceVisibility::Hidden,
                        VisibilityDto::Unknown => SourceVisibility::Unknown,
                    },
                })
            })
            .collect::<Result<Vec<_>, &'static str>>()?;
        Ok(SourceCatalogObservation {
            catalog: SourceCatalog {
                entries,
                freshness: match self.freshness {
                    FreshnessDto::Unknown => Freshness::Unknown,
                    FreshnessDto::Live => Freshness::Live,
                    FreshnessDto::Partial => Freshness::Partial,
                    FreshnessDto::Invalidated => Freshness::Invalidated,
                },
                generation: self.generation,
                observed_at: system_time_from_ms(self.observed_at_ms),
                error: self.extra.error(),
            },
            raw_response: self.extra.raw_response(),
            response_evidence: match self.response {
                CatalogEvidenceDto::Complete => CatalogResponseEvidence::Complete,
                CatalogEvidenceDto::Partial => CatalogResponseEvidence::Partial,
                CatalogEvidenceDto::Unsupported => CatalogResponseEvidence::Unsupported,
                CatalogEvidenceDto::Malformed => CatalogResponseEvidence::Malformed,
                CatalogEvidenceDto::Timeout => CatalogResponseEvidence::Timeout,
                CatalogEvidenceDto::Disconnected => CatalogResponseEvidence::Disconnected,
            },
        })
    }
}
