//! Domain types for Denon/Marantz AVR control.
//!
//! This module contains the canonical domain model that is independent of
//! protocol details, transport mechanisms, and infrastructure concerns.

pub mod audio_context;
pub mod capabilities;
pub mod eq_status;
pub mod main_zone;
pub mod quick_select;
pub mod receiver;

pub use audio_context::{
    AudioContextSnapshot, Confidence, ConfiguredSpeakerLayout, InputChannelLayout, Observation,
    Observed, OutputChannelLayout, Provenance, RawObservation, SampleRate, SignalCodec,
    SignalFlags, SignalFormat,
};
pub use capabilities::{Model, ModelCapabilities, QuickSelectEqCapabilities};
pub use eq_status::{EqEvidence, EqFeature, EqState, EqStatus};
pub use main_zone::{
    AudioContextField, AudioContextValue, ConnectionState, FieldError, FieldErrorKind, FieldStatus,
    Freshness, Input, ListeningModeGroup, MainZoneControl, MainZoneEvent, MainZoneField,
    MainZoneSnapshot, MainZoneValue, MuteState, PowerState, StateAuthority, SurroundMode, Volume,
    VolumeLevel,
};
pub use quick_select::{
    QuickSelectName, QuickSelectPreset, QuickSelectRecallConfirmation, QuickSelectRecallOutcome,
    QuickSelectSlot, QuickSelectSnapshot, QuickSelectSummary, Registered,
};
pub use receiver::{ConfiguredReceivers, DiscoveredReceiver, ReceiverEndpoint, ReceiverIdentity};
