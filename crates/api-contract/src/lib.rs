//! The Control API's wire contract, version 1.
//!
//! This crate is the one place a wire shape is defined. Each type converts from
//! the control-service port's value, and, where a client needs the value back,
//! to it, so the server and the client cannot disagree about a field. It depends
//! on the application and domain crates and on `serde`, and on nothing that
//! speaks HTTP or runs a program: the transport is the server's and the client's.
//!
//! Requests are strict, so a mistyped field is refused by name instead of being
//! ignored. Responses are lenient, so the contract can grow: a reader skips the
//! fields it does not know.

pub mod error;
pub mod operations;
pub mod paths;
pub mod receivers;
pub mod routes;
pub mod state;
pub mod values;

pub use error::{ApiError, ErrorBody, ErrorDetail};
pub use operations::{DispatchDto, IntentDto, OperationDto, StatusDto};
pub use paths::{encode_segment, EndpointPaths};
pub use receivers::{
    parse_receiver_id, AgentEndpointDto, ApprovalDto, AuditDto, CapabilitiesDto, ConnectionDto,
    HealthDto, PolicyHealthDto, ReceiverSummaryDto, ServerHealthDto, TokenStoreDto,
};
pub use routes::{Audience, Method, Route, RouteId};
pub use state::{
    AgentSourcesView, AgentStateView, CatalogEvidenceDto, Detail, Diagnostics, Extras, FieldText,
    FieldView, FreshnessDto, MainZoneView, NoText, OperatorSourcesView, OperatorStateView,
    ReasonDto, SoundModeValueDto, SourceEntryDto, SourcesDetail, SourcesText, SourcesView,
    StateView, ValidityDto, ViewError, VisibilityDto,
};
pub use values::{MuteDto, SoundModeDto, SystemPowerDto, VolumeDto, ZonePowerDto};

/// The version of the contract. A client that finds another reports it.
pub const CONTRACT_VERSION: u32 = 1;
