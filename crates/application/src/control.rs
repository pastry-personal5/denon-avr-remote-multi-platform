//! The control-service port.
//!
//! GUI, CLI, and MCP servers observe and control receivers only through this
//! port, so the receiver session has one owner. It is three traits split by
//! authority, and a surface takes only the traits it needs:
//!
//! - [`ReceiverReads`]: what an agent may observe.
//! - [`OperationControl`]: submitting operations and managing the caller's own.
//! - [`OperatorAdmin`]: discovery, configuration, and Operator-only reads.
//!
//! One handle implements all three. The caller's [`Principal`] is fixed when the
//! handle is created from a credential and is never a request parameter.

use crate::ports::{BoxFuture, OperationError};
use crate::session_v3::StateSubscription;
use denon_avr_domain::{
    ConfiguredReceivers, DiscoveredReceiver, DispatchCertainty, HttpInformationSnapshot,
    ModelCapabilities, OperationId, QuickSelectNameObservation, ReceiverId, ReceiverIdentity,
    ReceiverIntent, SourceCatalogObservation,
};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

const MAX_LABEL_LEN: usize = 64;
const MAX_KEY_LEN: usize = 128;

fn validate_text(value: &str, max: usize, what: &'static str) -> Result<(), &'static str> {
    if value.trim().is_empty() || value.chars().any(char::is_control) || value.len() > max {
        return Err(what);
    }
    Ok(())
}

/// Who is calling, as established by the server from the presented credential.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Principal {
    /// The user, through the GUI or CLI.
    Operator,
    /// One labelled AI agent.
    Agent(AgentLabel),
}

/// The label an Agent token is registered under. It names the agent in the
/// audit log and in policy, and the agent never chooses it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AgentLabel(String);

impl AgentLabel {
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        validate_text(
            &value,
            MAX_LABEL_LEN,
            "agent label must be 1 to 64 characters with no control characters",
        )?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A client-chosen key that makes a retry of the same request return the
/// existing operation instead of creating another.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        validate_text(
            &value,
            MAX_KEY_LEN,
            "idempotency key must be 1 to 128 characters with no control characters",
        )?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A request to change the receiver. The server allocates the operation id.
///
/// Policy dry runs and an agent-supplied justification join this type with the
/// components that use them.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OperationSubmission {
    pub intent: ReceiverIntent,
    pub idempotency_key: Option<IdempotencyKey>,
}

impl OperationSubmission {
    pub fn new(intent: ReceiverIntent) -> Self {
        Self {
            intent,
            idempotency_key: None,
        }
    }

    pub fn with_idempotency_key(mut self, key: IdempotencyKey) -> Self {
        self.idempotency_key = Some(key);
        self
    }
}

/// Where an operation is in the gate's lifecycle. Evaluation is atomic with
/// admission, so there is no separate evaluated state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationStatus {
    Submitted,
    Allowed,
    AwaitingApproval,
    Approved,
    /// The session is executing it. It can no longer be cancelled.
    InSession,
    /// Ended before the session was called. Nothing was dispatched.
    Denied,
    ApprovalUnavailable,
    ApprovalRejected,
    Expired,
    /// Withdrawn by its owner, or dropped by the session before dispatch.
    Cancelled,
    /// The session observed the requested value.
    Completed,
    /// The receiver already had the requested value, so nothing was written.
    AlreadyInState,
    /// The session refused it before dispatch.
    Rejected,
    /// A newer operation replaced it before dispatch.
    Superseded {
        by: OperationId,
    },
    /// The session could not establish the outcome.
    Indeterminate,
}

impl OperationStatus {
    /// Whether the operation has finished and its snapshot will not change.
    pub fn is_terminal(&self) -> bool {
        !matches!(
            self,
            Self::Submitted
                | Self::Allowed
                | Self::AwaitingApproval
                | Self::Approved
                | Self::InSession
        )
    }

    /// The stable name used in tool results and the wire schema.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Allowed => "allowed",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Approved => "approved",
            Self::InSession => "in_session",
            Self::Denied => "denied",
            Self::ApprovalUnavailable => "approval_unavailable",
            Self::ApprovalRejected => "approval_rejected",
            Self::Expired => "expired",
            Self::Cancelled => "cancelled",
            Self::Completed => "completed",
            Self::AlreadyInState => "already_in_state",
            Self::Rejected => "rejected",
            Self::Superseded { .. } => "superseded",
            Self::Indeterminate => "indeterminate",
        }
    }
}

/// A point-in-time view of one operation.
///
/// `dispatch` is the session's certainty and `NotDispatched` for every status
/// decided before the session. `confirmed` is true only when receiver evidence
/// shows the requested value, never because a write completed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationSnapshot {
    pub id: OperationId,
    pub receiver: ReceiverId,
    pub intent: ReceiverIntent,
    pub status: OperationStatus,
    pub dispatch: DispatchCertainty,
    pub confirmed: bool,
    /// Why the operation was denied, rejected, or left indeterminate, in plain
    /// text for display.
    pub reason: Option<String>,
    /// The receiver observation behind `confirmed`, when one exists.
    pub observation: Option<String>,
}

/// One item from an operation event stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationEvent {
    Update(OperationSnapshot),
    /// Events were dropped because the reader fell behind. Read the operation
    /// itself to learn its current state.
    Missed,
}

/// A stream of updates for the caller's own operations, or for every operation
/// when the caller is the Operator. Unlike state snapshots these are not
/// coalesced. `None` means the stream ended.
pub trait OperationEventSource: Send {
    fn next(&mut self) -> BoxFuture<'_, Option<OperationEvent>>;
}

pub type OperationEvents = Box<dyn OperationEventSource>;

/// Why a port call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlError {
    /// Unknown, or not visible to the caller. The two are not distinguished.
    NotFound(&'static str),
    /// The handle's principal may not do this.
    Forbidden,
    InvalidRequest(String),
    /// The service, or the receiver behind it, cannot be reached.
    Unavailable(String),
    /// A cancel arrived after the operation reached the session. Carries the
    /// operation's actual state.
    TooLate(Box<OperationSnapshot>),
    RateLimited {
        retry_after: Option<Duration>,
    },
    /// The receiver session reported an error.
    Receiver(OperationError),
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(what) => write!(f, "{what} not found"),
            Self::Forbidden => f.write_str("not permitted"),
            Self::InvalidRequest(message) => write!(f, "invalid request: {message}"),
            Self::Unavailable(message) => write!(f, "service unavailable: {message}"),
            Self::TooLate(snapshot) => write!(
                f,
                "too late to cancel: operation is {}",
                snapshot.status.as_str()
            ),
            Self::RateLimited { .. } => f.write_str("rate limited"),
            Self::Receiver(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ControlError {}

impl From<OperationError> for ControlError {
    fn from(error: OperationError) -> Self {
        Self::Receiver(error)
    }
}

/// Whether the server currently holds a session to a receiver. It releases idle
/// receivers, so `Released` is normal and not a fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    Released,
    Connecting,
    Connected,
}

/// What an agent may learn about a receiver's abilities. It is narrower than
/// `ModelCapabilities` and carries no port numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverCapabilities {
    pub writable: bool,
    pub zone2_power: bool,
    pub source_catalog_read: bool,
    pub inputs: &'static [&'static str],
    pub surround_modes: &'static [&'static str],
}

impl From<&ModelCapabilities> for ReceiverCapabilities {
    fn from(capabilities: &ModelCapabilities) -> Self {
        Self {
            writable: capabilities.writable,
            zone2_power: capabilities.zone2_power,
            source_catalog_read: capabilities.source_catalog_read,
            inputs: capabilities.inputs,
            surround_modes: capabilities.surround_modes,
        }
    }
}

/// A saved receiver as an agent sees it. There is deliberately no address field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverSummary {
    pub id: ReceiverId,
    pub model: Option<String>,
    pub capabilities: ReceiverCapabilities,
    pub connection: ConnectionStatus,
}

/// What an agent may observe.
pub trait ReceiverReads: Send + Sync {
    /// Saved receivers only. A receiver chosen by address is never listed.
    fn receivers(&self) -> BoxFuture<'_, Result<Vec<ReceiverSummary>, ControlError>>;

    /// Subscribe to a receiver's complete state, with per-field validity.
    ///
    /// Resolves once the session is connected and has synchronized, so the first
    /// snapshot is complete. Holding the subscription keeps the receiver
    /// connected. Snapshots are coalesced: a slow reader sees the newest one.
    fn state<'a>(
        &'a self,
        receiver: &'a ReceiverId,
    ) -> BoxFuture<'a, Result<StateSubscription, ControlError>>;

    /// The receiver's own source names and visibility.
    fn source_catalog<'a>(
        &'a self,
        receiver: &'a ReceiverId,
    ) -> BoxFuture<'a, Result<SourceCatalogObservation, ControlError>>;
}

/// Submitting operations and managing the caller's own. The Operator sees every
/// operation; an agent sees only its own.
pub trait OperationControl: Send + Sync {
    /// Admit an operation and return promptly with its first snapshot. A
    /// request identical to one of the caller's in-flight operations, or one
    /// repeating an idempotency key, returns that operation instead of a new one.
    fn submit<'a>(
        &'a self,
        receiver: &'a ReceiverId,
        submission: OperationSubmission,
    ) -> BoxFuture<'a, Result<OperationSnapshot, ControlError>>;

    /// The operation's current state. With `wait`, returns early once it is
    /// terminal, and otherwise after the wait elapses or the server's cap
    /// passes, whichever is first.
    fn operation(
        &self,
        id: OperationId,
        wait: Option<Duration>,
    ) -> BoxFuture<'_, Result<OperationSnapshot, ControlError>>;

    /// Withdraw an operation before it reaches the session. Afterwards the
    /// result is [`ControlError::TooLate`] carrying the actual state.
    fn cancel(&self, id: OperationId) -> BoxFuture<'_, Result<OperationSnapshot, ControlError>>;

    fn operation_events(&self) -> BoxFuture<'_, Result<OperationEvents, ControlError>>;
}

/// Operator-only administration. Token, approval, audit, and policy views join
/// this trait with the components that provide them.
pub trait OperatorAdmin: Send + Sync {
    fn discover(
        &self,
        timeout: Duration,
    ) -> BoxFuture<'_, Result<Vec<DiscoveredReceiver>, ControlError>>;

    /// Use a receiver chosen only by address. It gets an ad hoc id, is not
    /// listed to agents, and is released like any other idle receiver.
    fn register_ad_hoc(
        &self,
        identity: ReceiverIdentity,
    ) -> BoxFuture<'_, Result<ReceiverId, ControlError>>;

    fn configuration(&self) -> BoxFuture<'_, Result<ConfiguredReceivers, ControlError>>;

    fn save_configuration<'a>(
        &'a self,
        configuration: &'a ConfiguredReceivers,
    ) -> BoxFuture<'a, Result<(), ControlError>>;

    /// Reads that no agent tool uses, so agents cannot reach them.
    fn quick_select_names<'a>(
        &'a self,
        receiver: &'a ReceiverId,
    ) -> BoxFuture<'a, Result<QuickSelectNameObservation, ControlError>>;

    fn http_information<'a>(
        &'a self,
        receiver: &'a ReceiverId,
    ) -> BoxFuture<'a, Result<HttpInformationSnapshot, ControlError>>;
}

/// What a writing agent is handed: observation and operations, no administration.
pub trait AgentControl: ReceiverReads + OperationControl {}
impl<T: ReceiverReads + OperationControl + ?Sized> AgentControl for T {}

/// What the GUI and CLI are handed: everything.
pub trait OperatorControl: AgentControl + OperatorAdmin {}
impl<T: AgentControl + OperatorAdmin + ?Sized> OperatorControl for T {}

pub type SharedAgentControl = Arc<dyn AgentControl>;
pub type SharedOperatorControl = Arc<dyn OperatorControl>;

#[cfg(test)]
mod tests {
    use super::*;
    use denon_avr_domain::{MuteState, ZonePower};

    fn unavailable<T: Send + 'static>() -> BoxFuture<'static, Result<T, ControlError>> {
        Box::pin(async { Err(ControlError::Unavailable("stub".into())) })
    }

    /// Implements every trait, to prove they are object safe and compose.
    struct Stub;

    impl ReceiverReads for Stub {
        fn receivers(&self) -> BoxFuture<'_, Result<Vec<ReceiverSummary>, ControlError>> {
            unavailable()
        }
        fn state<'a>(
            &'a self,
            _: &'a ReceiverId,
        ) -> BoxFuture<'a, Result<StateSubscription, ControlError>> {
            unavailable()
        }
        fn source_catalog<'a>(
            &'a self,
            _: &'a ReceiverId,
        ) -> BoxFuture<'a, Result<SourceCatalogObservation, ControlError>> {
            unavailable()
        }
    }

    impl OperationControl for Stub {
        fn submit<'a>(
            &'a self,
            _: &'a ReceiverId,
            _: OperationSubmission,
        ) -> BoxFuture<'a, Result<OperationSnapshot, ControlError>> {
            unavailable()
        }
        fn operation(
            &self,
            _: OperationId,
            _: Option<Duration>,
        ) -> BoxFuture<'_, Result<OperationSnapshot, ControlError>> {
            unavailable()
        }
        fn cancel(&self, _: OperationId) -> BoxFuture<'_, Result<OperationSnapshot, ControlError>> {
            unavailable()
        }
        fn operation_events(&self) -> BoxFuture<'_, Result<OperationEvents, ControlError>> {
            unavailable()
        }
    }

    impl OperatorAdmin for Stub {
        fn discover(
            &self,
            _: Duration,
        ) -> BoxFuture<'_, Result<Vec<DiscoveredReceiver>, ControlError>> {
            unavailable()
        }
        fn register_ad_hoc(
            &self,
            _: ReceiverIdentity,
        ) -> BoxFuture<'_, Result<ReceiverId, ControlError>> {
            unavailable()
        }
        fn configuration(&self) -> BoxFuture<'_, Result<ConfiguredReceivers, ControlError>> {
            unavailable()
        }
        fn save_configuration<'a>(
            &'a self,
            _: &'a ConfiguredReceivers,
        ) -> BoxFuture<'a, Result<(), ControlError>> {
            unavailable()
        }
        fn quick_select_names<'a>(
            &'a self,
            _: &'a ReceiverId,
        ) -> BoxFuture<'a, Result<QuickSelectNameObservation, ControlError>> {
            unavailable()
        }
        fn http_information<'a>(
            &'a self,
            _: &'a ReceiverId,
        ) -> BoxFuture<'a, Result<HttpInformationSnapshot, ControlError>> {
            unavailable()
        }
    }

    #[test]
    fn one_handle_narrows_to_each_surface_without_new_objects() {
        let operator: SharedOperatorControl = Arc::new(Stub);
        let agent: SharedAgentControl = operator.clone();
        let reads: Arc<dyn ReceiverReads> = agent.clone();
        let operations: Arc<dyn OperationControl> = agent.clone();
        let admin: Arc<dyn OperatorAdmin> = operator;
        // Four views of one allocation: agent, reads, operations, and admin.
        assert_eq!(Arc::strong_count(&agent), 4);
        drop((reads, operations, admin));
    }

    #[tokio::test]
    async fn the_stub_fails_every_call_with_a_typed_error() {
        let id = ReceiverId::new("living-room").unwrap();
        let error = Stub.receivers().await.unwrap_err();
        assert_eq!(error, ControlError::Unavailable("stub".into()));
        assert_eq!(error.to_string(), "service unavailable: stub");
        assert!(Stub.state(&id).await.is_err());
    }

    #[test]
    fn labels_and_keys_are_bounded_text() {
        assert!(AgentLabel::new("openclaw").is_ok());
        assert!(IdempotencyKey::new("retry-1").is_ok());
        for bad in ["", "  ", "new\nline", &"x".repeat(129)] {
            assert!(AgentLabel::new(bad).is_err(), "{bad:?}");
            assert!(IdempotencyKey::new(bad).is_err(), "{bad:?}");
        }
        assert!(AgentLabel::new("x".repeat(65)).is_err());
        assert!(AgentLabel::new("x".repeat(64)).is_ok());
    }

    #[test]
    fn principals_compare_by_label() {
        let a = Principal::Agent(AgentLabel::new("a").unwrap());
        assert_eq!(a, Principal::Agent(AgentLabel::new("a").unwrap()));
        assert_ne!(a, Principal::Agent(AgentLabel::new("b").unwrap()));
        assert_ne!(a, Principal::Operator);
    }

    #[test]
    fn only_finished_statuses_are_terminal() {
        let in_progress = [
            OperationStatus::Submitted,
            OperationStatus::Allowed,
            OperationStatus::AwaitingApproval,
            OperationStatus::Approved,
            OperationStatus::InSession,
        ];
        let finished = [
            OperationStatus::Denied,
            OperationStatus::ApprovalUnavailable,
            OperationStatus::ApprovalRejected,
            OperationStatus::Expired,
            OperationStatus::Cancelled,
            OperationStatus::Completed,
            OperationStatus::AlreadyInState,
            OperationStatus::Rejected,
            OperationStatus::Superseded { by: OperationId(2) },
            OperationStatus::Indeterminate,
        ];
        assert!(in_progress.iter().all(|status| !status.is_terminal()));
        assert!(finished.iter().all(OperationStatus::is_terminal));
    }

    #[test]
    fn status_names_are_unique_snake_case() {
        let names: Vec<_> = [
            OperationStatus::Submitted,
            OperationStatus::Allowed,
            OperationStatus::AwaitingApproval,
            OperationStatus::Approved,
            OperationStatus::InSession,
            OperationStatus::Denied,
            OperationStatus::ApprovalUnavailable,
            OperationStatus::ApprovalRejected,
            OperationStatus::Expired,
            OperationStatus::Cancelled,
            OperationStatus::Completed,
            OperationStatus::AlreadyInState,
            OperationStatus::Rejected,
            OperationStatus::Superseded { by: OperationId(1) },
            OperationStatus::Indeterminate,
        ]
        .iter()
        .map(OperationStatus::as_str)
        .collect();
        let unique: std::collections::BTreeSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len());
        assert!(names
            .iter()
            .all(|name| name.chars().all(|c| c.is_ascii_lowercase() || c == '_')));
    }

    #[test]
    fn a_too_late_error_reports_the_actual_status() {
        let snapshot = OperationSnapshot {
            id: OperationId(7),
            receiver: ReceiverId::new("living-room").unwrap(),
            intent: ReceiverIntent::Mute(MuteState::On),
            status: OperationStatus::InSession,
            dispatch: DispatchCertainty::NotDispatched,
            confirmed: false,
            reason: None,
            observation: None,
        };
        let error = ControlError::TooLate(Box::new(snapshot));
        assert_eq!(
            error.to_string(),
            "too late to cancel: operation is in_session"
        );
    }

    #[test]
    fn a_submission_carries_its_idempotency_key() {
        let submission = OperationSubmission::new(ReceiverIntent::MainZonePower(ZonePower::Off))
            .with_idempotency_key(IdempotencyKey::new("k1").unwrap());
        assert_eq!(submission.idempotency_key.unwrap().as_str(), "k1");
    }

    #[test]
    fn agent_capabilities_omit_network_details() {
        let capabilities = ReceiverCapabilities::from(&ModelCapabilities::for_model(
            denon_avr_domain::Model::AvrX3800h,
        ));
        assert!(capabilities.writable && capabilities.zone2_power);
        assert!(!capabilities.inputs.is_empty());
    }
}
