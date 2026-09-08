//! Denon/Marantz AVR and HEOS IP control library.
//!
//! The public API is organized into domain, application, protocol, and
//! infrastructure layers.

pub mod application;
pub mod domain;
pub mod gui;
pub mod infrastructure;
pub mod protocol;

pub use application::{
    query_audio_context_async, query_main_zone_status, query_main_zone_status_async,
};
pub use application::{ControllerConfig, ReceiverCommand, ReceiverController, ReceiverEvent};

pub use application::ports::{
    AsyncConfigRepository, AsyncControlGateway, AsyncReceiverDiscovery, AsyncStatusGateway,
    BoxFuture, ConfigRepository, ControlGateway, OperationError, OperationErrorKind,
    ReceiverDiscovery, SessionEvent, StatusGateway,
};
pub use domain::{
    AudioContextField, AudioContextSnapshot, AudioContextValue, Confidence, ConfiguredReceivers,
    ConfiguredSpeakerLayout, ConnectionState, DiscoveredReceiver, EqEvidence, EqFeature, EqState,
    EqStatus, FieldError, FieldErrorKind, FieldStatus, Input, InputChannelLayout,
    ListeningModeGroup, MainZoneControl, MainZoneField, MainZoneSnapshot, MainZoneValue, MuteState,
    Observation, Observed, OutputChannelLayout, PowerState, Provenance, QuickSelectName,
    QuickSelectPreset, QuickSelectRecallConfirmation, QuickSelectRecallOutcome, QuickSelectSlot,
    QuickSelectSnapshot, QuickSelectSummary, RawObservation, ReceiverEndpoint, Registered,
    SampleRate, SignalCodec, SignalFlags, SignalFormat, SurroundMode, ValidatedPhase8Capabilities,
    Volume, VolumeLevel,
};
