//! The Operator's resources: configuration, discovery, the inspection reads, the
//! policy, the audit log, and tokens.
//!
//! Nothing here is served to an agent, so these types carry whatever the
//! Operator needs, addresses and error text included.

mod audit;
mod config;
mod discovery;
mod inspection;
mod policy;
mod tokens;

pub use audit::{
    AuditDecisionDto, AuditEntryDto, AuditEventDto, AuditPageDto, AuditRecordDto, PrincipalDto,
};
pub use config::{ConfigDto, ConfigRequest, IdentityDto, IdentityRequest};
pub use discovery::{
    AdHocRequest, AdHocResponse, DiscoverRequest, DiscoveredDto, DiscoveredListDto,
};
pub use inspection::{
    AudioDto, AudysseyDto, ChannelSlotDto, FieldErrorKindDto, FieldStatusDto, HttpInformationDto,
    QuickSelectNamesDto, ReadinessDto, SlotStateDto, VideoDto,
};
pub use policy::PolicyDto;
pub use tokens::{IssuedTokenDto, TokenDto, TokenIssueRequest, TokenListDto};
