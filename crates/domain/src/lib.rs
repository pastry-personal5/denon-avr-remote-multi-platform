//! Domain types for Denon/Marantz AVR control.
//!
//! This module contains the canonical domain model that is independent of
//! protocol details, transport mechanisms, and infrastructure concerns.

pub mod audio_context;
pub mod capabilities;
pub mod eq_status;
pub mod http_information;
pub mod main_zone;
pub mod quick_select;
pub mod quick_select_names;
pub mod receiver;
pub mod receiver_state;
pub mod source_catalog;
pub mod sync_debt;
pub mod transport_timing;

pub use audio_context::{
    AudioContextSnapshot, Confidence, ConfiguredSpeakerLayout, InputChannelLayout, Observation,
    Observed, OutputChannelLayout, Provenance, RawObservation, SampleRate, SignalCodec,
    SignalFlags, SignalFormat,
};
pub use capabilities::{
    canonical_x3800h_source, supports_x3800h_sound_mode, supports_x3800h_source,
    x3800h_sound_mode_in_category, Model, ModelCapabilities, QuickSelectEqCapabilities,
    SourceCatalogCapabilities,
};
pub use eq_status::{EqEvidence, EqFeature, EqState, EqStatus};
pub use http_information::{
    AudioInformation, AudysseyInformation, ChannelSlot, ChannelSlotState, HttpInformationSnapshot,
    VideoInformation,
};
pub use main_zone::{
    AudioContextField, AudioContextValue, ConnectionState, FieldError, FieldErrorKind, FieldStatus,
    Freshness, Input, MainZoneControl, MainZoneEvent, MainZoneField, MainZoneSnapshot,
    MainZoneValue, MuteState, PowerState, SoundModeCategory, StateAuthority, SurroundMode, Volume,
    VolumeLevel, Zone2Control, Zone2Snapshot,
};
pub use quick_select::{
    QuickSelectName, QuickSelectPreset, QuickSelectRecallConfirmation, QuickSelectRecallOutcome,
    QuickSelectSlot, QuickSelectSnapshot, QuickSelectSummary, Registered,
};
pub use quick_select_names::{QuickSelectNameObservation, QuickSelectNameResponseEvidence};
pub use receiver::{
    ConfiguredReceivers, DiscoveredReceiver, ReceiverEndpoint, ReceiverIdentity, SoundModeFavorite,
};
pub use receiver_state::{
    CoreField, CoreFrame, DispatchCertainty, Epoch, FieldIssue, FieldState as ReceiverFieldState,
    FieldSynchronization, FieldValidity as ReceiverFieldValidity, FrameSeq, MasterVolume,
    MonotonicMillis, Observation as ReceiverObservation, ObservationOrigin, OperationId,
    OperationOutcome, ReceiverId, ReceiverIntent, ReceiverState, SoundModeIntent, SoundModeStatus,
    StaleReason, StateRevision, SyncCause, SyncCycleId, SystemPower, ZonePower,
};
pub use source_catalog::{
    CatalogResponseEvidence, SourceCatalog, SourceCatalogObservation, SourceEntry, SourceId,
    SourceVisibility,
};
pub use sync_debt::SyncDebt;
pub use transport_timing::TransmissionSchedule;
