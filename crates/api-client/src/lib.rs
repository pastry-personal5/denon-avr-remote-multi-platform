//! The Control API's client.
//!
//! `ApiClient` is the control-service port over a Unix socket: it implements
//! `ReceiverReads`, `OperationControl`, and `OperatorAdmin` by asking the server,
//! so a caller written against the port runs unchanged against the in-process
//! service or a remote one. It speaks HTTP/1.1 and nothing else: no TLS, no
//! server, no reuse of a connection, so it stays small enough for a process that
//! an agent runs.
//!
//! The server decides what a caller may do. A client built for an agent does not
//! send an Operator request at all, since the answer is known; the port's
//! `Forbidden` is returned without a round trip.

#[cfg(not(unix))]
compile_error!("the Control API client is built for Unix only; macOS is the supported platform");

mod mapping;
mod sse;
mod transport;

pub use transport::Token;

use denon_avr_api_contract::admin::{
    AdHocRequest, AdHocResponse, AuditPageDto, ConfigDto, ConfigRequest, DiscoverRequest,
    DiscoveredListDto, HttpInformationDto, IssuedTokenDto, PolicyDto, QuickSelectNamesDto,
    ReadinessDto, TokenDto, TokenIssueRequest, TokenListDto,
};
use denon_avr_api_contract::paths::request as path;
use denon_avr_api_contract::requests::{
    AgentDryRun, DryRunRequest, OperatorDryRun, OperatorDryRunRequest, SubmitRequest,
};
use denon_avr_api_contract::{
    parse_receiver_id, AgentSourcesView, HealthDto, OperationDto, OperatorSourcesView,
    ReceiverSummaryDto, ReceiversDto, ServerHealthDto, CONTRACT_VERSION,
};
use denon_avr_application::ports::BoxFuture;
use denon_avr_application::{
    AgentLabel, AuditPage, AuditQuery, ControlError, DryRun, IssuedToken, OperationControl,
    OperationEvents, OperationSnapshot, OperationSubmission, OperatorAdmin, PolicyView, Readiness,
    ReceiverReads, ReceiverSummary, ServiceHealth, StateSubscription, TokenId, TokenRecord,
};
use denon_avr_domain::{
    ConfiguredReceivers, DiscoveredReceiver, HttpInformationSnapshot, OperationId,
    QuickSelectNameObservation, ReceiverId, ReceiverIdentity, ReceiverIntent,
    SourceCatalogObservation,
};
use mapping::unreadable;
use sse::{decode_state, ended, EventReader, RemoteOperationEvents};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use transport::{Abort, Call, Transport, REQUEST_TIMEOUT};

/// The longest a state stream may take to send its first state: the server
/// connects the receiver first.
const FIRST_STATE: Duration = Duration::from_secs(30);

/// Which endpoint a client talks to, which decides the views it can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    /// The Operator endpoint: the whole API, and views that carry the receiver's
    /// own text.
    Operator,
    /// The Agent endpoint: the shared resources, and views with no text in them.
    Agent,
}

/// Where the server is and how to prove who is asking.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub socket: PathBuf,
    pub token: Token,
    pub audience: Audience,
}

/// Why a client could not be made. Nothing is sent to build one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectError {
    /// The socket path is longer than the system allows.
    SocketPathTooLong { len: usize, max: usize },
    /// The token is empty or has characters a bearer token cannot have.
    TokenNotWellFormed,
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SocketPathTooLong { len, max } => write!(
                f,
                "the socket path is {len} bytes long and the longest this system allows is {max}"
            ),
            Self::TokenNotWellFormed => f.write_str("the token is not a bearer token"),
        }
    }
}

impl std::error::Error for ConnectError {}

#[cfg(target_os = "macos")]
const SUN_PATH_LEN: usize = 104;
#[cfg(not(target_os = "macos"))]
const SUN_PATH_LEN: usize = 108;

/// The control-service port over a socket.
#[derive(Clone)]
pub struct ApiClient {
    transport: Arc<Transport>,
}

impl std::fmt::Debug for ApiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiClient")
            .field("socket", &self.transport.socket)
            .field("audience", &self.transport.audience)
            .field("token", &self.transport.token)
            .finish()
    }
}

impl ApiClient {
    /// A client for `endpoint`. It opens no connection: each request opens its own.
    pub fn connect(endpoint: Endpoint) -> Result<Self, ConnectError> {
        use std::os::unix::ffi::OsStrExt;
        let len = endpoint.socket.as_os_str().as_bytes().len();
        if len >= SUN_PATH_LEN {
            return Err(ConnectError::SocketPathTooLong {
                len,
                max: SUN_PATH_LEN - 1,
            });
        }
        if !endpoint.token.is_well_formed() {
            return Err(ConnectError::TokenNotWellFormed);
        }
        Ok(Self {
            transport: Arc::new(Transport {
                socket: endpoint.socket,
                token: endpoint.token,
                audience: endpoint.audience,
            }),
        })
    }

    pub fn audience(&self) -> Audience {
        self.transport.audience
    }

    /// What the server says about itself: its endpoints and its token store. Only
    /// the Operator endpoint says, so an agent's client does not ask.
    pub async fn server_health(&self) -> Result<ServerHealthDto, ControlError> {
        self.require_operator()?;
        let health: HealthDto = self.transport.json(Call::get(path::health())).await?;
        health
            .server
            .ok_or_else(|| ControlError::Unavailable("the server did not report on itself".into()))
    }

    fn require_operator(&self) -> Result<(), ControlError> {
        match self.transport.audience {
            Audience::Operator => Ok(()),
            Audience::Agent => Err(ControlError::Forbidden),
        }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: String) -> Result<T, ControlError> {
        self.transport.json(Call::get(path)).await
    }

    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: String,
        body: &impl serde::Serialize,
    ) -> Result<T, ControlError> {
        self.transport.json(Call::post(path, body)?).await
    }
}

fn operation(dto: OperationDto) -> Result<OperationSnapshot, ControlError> {
    OperationSnapshot::try_from(dto).map_err(unreadable)
}

impl ReceiverReads for ApiClient {
    fn receivers(&self) -> BoxFuture<'_, Result<Vec<ReceiverSummary>, ControlError>> {
        Box::pin(async move {
            let list: ReceiversDto = self.get(path::receivers()).await?;
            list.receivers
                .into_iter()
                .map(|dto: ReceiverSummaryDto| ReceiverSummary::try_from(dto).map_err(unreadable))
                .collect()
        })
    }

    fn state<'a>(
        &'a self,
        receiver: &'a ReceiverId,
    ) -> BoxFuture<'a, Result<StateSubscription, ControlError>> {
        Box::pin(async move {
            let audience = self.transport.audience;
            let (response, connection) = self
                .transport
                .open_stream(path::state_events(receiver))
                .await?;
            let mut reader = EventReader::new(response.into_body(), connection);
            let first = tokio::time::timeout(FIRST_STATE, reader.next())
                .await
                .map_err(|_| self.transport.unreachable())?
                .map_err(|_| self.transport.unreachable())?
                .ok_or_else(|| self.transport.unreachable())?;
            if first.name == denon_avr_api_contract::events::EventName::End {
                return Err(ended(&first));
            }
            let state = decode_state(audience, &first)?;

            let (states, latest) = watch::channel(state);
            let (finished, ended_rx) = watch::channel(false);
            let pump = tokio::spawn(async move {
                // Every way this ends is the same to the subscriber: the session is
                // over, and subscribing again reaches the receiver afresh.
                while let Ok(Some(event)) = reader.next().await {
                    match event.name {
                        denon_avr_api_contract::events::EventName::State => {
                            match decode_state(audience, &event) {
                                Ok(state) => {
                                    states.send_replace(state);
                                }
                                Err(_) => break,
                            }
                        }
                        denon_avr_api_contract::events::EventName::End => break,
                        _ => {}
                    }
                }
                finished.send_replace(true);
            });
            Ok(StateSubscription::new(latest)
                .ending_with(ended_rx)
                .holding(Abort::of(&pump)))
        })
    }

    fn source_catalog<'a>(
        &'a self,
        receiver: &'a ReceiverId,
    ) -> BoxFuture<'a, Result<SourceCatalogObservation, ControlError>> {
        Box::pin(async move {
            let url = path::sources(receiver);
            match self.transport.audience {
                Audience::Operator => self
                    .get::<OperatorSourcesView>(url)
                    .await?
                    .into_observation()
                    .map_err(unreadable),
                Audience::Agent => self
                    .get::<AgentSourcesView>(url)
                    .await?
                    .into_observation()
                    .map_err(unreadable),
            }
        })
    }

    fn health(&self) -> BoxFuture<'_, Result<ServiceHealth, ControlError>> {
        Box::pin(async move {
            let health: HealthDto = self.get(path::health()).await?;
            if health.contract != CONTRACT_VERSION {
                return Err(ControlError::Unavailable(format!(
                    "the server speaks version {} of the Control API and this client speaks {}",
                    health.contract, CONTRACT_VERSION
                )));
            }
            Ok(health.service_health())
        })
    }
}

impl OperationControl for ApiClient {
    fn submit<'a>(
        &'a self,
        receiver: &'a ReceiverId,
        submission: OperationSubmission,
    ) -> BoxFuture<'a, Result<OperationSnapshot, ControlError>> {
        Box::pin(async move {
            let dto: OperationDto = self
                .post(path::submit(receiver), &SubmitRequest::from(&submission))
                .await?;
            operation(dto)
        })
    }

    fn operation(
        &self,
        id: OperationId,
        wait: Option<Duration>,
    ) -> BoxFuture<'_, Result<OperationSnapshot, ControlError>> {
        Box::pin(async move {
            let wait_ms = wait.map(|wait| u64::try_from(wait.as_millis()).unwrap_or(u64::MAX));
            let call = Call::get(path::operation(id.0, wait_ms))
                .timeout(REQUEST_TIMEOUT + wait.unwrap_or_default());
            operation(self.transport.json(call).await?)
        })
    }

    fn cancel(&self, id: OperationId) -> BoxFuture<'_, Result<OperationSnapshot, ControlError>> {
        Box::pin(async move {
            let dto: OperationDto = self
                .transport
                .json(Call::post_empty(path::cancel(id.0)))
                .await?;
            operation(dto)
        })
    }

    fn operation_events(&self) -> BoxFuture<'_, Result<OperationEvents, ControlError>> {
        Box::pin(async move {
            let (response, connection) =
                self.transport.open_stream(path::operation_events()).await?;
            Ok(Box::new(RemoteOperationEvents {
                reader: EventReader::new(response.into_body(), connection),
            }) as OperationEvents)
        })
    }

    fn dry_run<'a>(
        &'a self,
        receiver: &'a ReceiverId,
        intent: ReceiverIntent,
    ) -> BoxFuture<'a, Result<DryRun, ControlError>> {
        Box::pin(async move {
            let body = DryRunRequest {
                intent: (&intent).into(),
            };
            let url = path::dry_run(receiver);
            match self.transport.audience {
                Audience::Operator => self
                    .post::<OperatorDryRun>(url, &body)
                    .await?
                    .into_dry_run()
                    .map_err(unreadable),
                Audience::Agent => self
                    .post::<AgentDryRun>(url, &body)
                    .await?
                    .into_dry_run()
                    .map_err(unreadable),
            }
        })
    }
}

impl OperatorAdmin for ApiClient {
    fn discover(
        &self,
        timeout: Duration,
    ) -> BoxFuture<'_, Result<Vec<DiscoveredReceiver>, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let body = DiscoverRequest {
                timeout_ms: Some(u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX)),
            };
            let call = Call::post(path::discover(), &body)?.timeout(REQUEST_TIMEOUT + timeout);
            let found: DiscoveredListDto = self.transport.json(call).await?;
            Ok(found.receivers.into_iter().map(Into::into).collect())
        })
    }

    fn register_ad_hoc(
        &self,
        identity: ReceiverIdentity,
    ) -> BoxFuture<'_, Result<ReceiverId, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let response: AdHocResponse = self
                .post(path::ad_hoc(), &AdHocRequest::from(&identity))
                .await?;
            parse_receiver_id(&response.id).map_err(unreadable)
        })
    }

    fn configuration(&self) -> BoxFuture<'_, Result<ConfiguredReceivers, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let dto: ConfigDto = self.get(path::config()).await?;
            ConfiguredReceivers::try_from(dto).map_err(unreadable)
        })
    }

    fn save_configuration<'a>(
        &'a self,
        configuration: &'a ConfiguredReceivers,
    ) -> BoxFuture<'a, Result<(), ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            // The port replaces the configuration, so this asks for the version in
            // force and writes over it, as the in-process service does. The server
            // still refuses a write over a version it has not shown.
            let current = self.transport.answer(Call::get(path::config())).await?;
            let etag = current.etag.ok_or_else(|| {
                ControlError::Unavailable("the server sent no version of the configuration".into())
            })?;
            let call =
                Call::put(path::config(), &ConfigRequest::from(configuration))?.if_match(etag);
            self.transport.answer(call).await.map(|_| ())
        })
    }

    fn quick_select_names<'a>(
        &'a self,
        receiver: &'a ReceiverId,
    ) -> BoxFuture<'a, Result<QuickSelectNameObservation, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let dto: QuickSelectNamesDto = self.get(path::quick_select_names(receiver)).await?;
            QuickSelectNameObservation::try_from(dto).map_err(unreadable)
        })
    }

    fn http_information<'a>(
        &'a self,
        receiver: &'a ReceiverId,
    ) -> BoxFuture<'a, Result<HttpInformationSnapshot, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let dto: HttpInformationDto = self.get(path::http_information(receiver)).await?;
            Ok(HttpInformationSnapshot::from(dto))
        })
    }

    fn refresh<'a>(
        &'a self,
        receiver: &'a ReceiverId,
    ) -> BoxFuture<'a, Result<Readiness, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let dto: ReadinessDto = self
                .transport
                .json(Call::post_empty(path::refresh(receiver)))
                .await?;
            Ok(Readiness::from(dto))
        })
    }

    fn dry_run_as<'a>(
        &'a self,
        agent: AgentLabel,
        receiver: &'a ReceiverId,
        intent: ReceiverIntent,
    ) -> BoxFuture<'a, Result<DryRun, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let body = OperatorDryRunRequest {
                intent: (&intent).into(),
                as_agent: Some(agent.as_str().to_owned()),
            };
            self.post::<OperatorDryRun>(path::dry_run(receiver), &body)
                .await?
                .into_dry_run()
                .map_err(unreadable)
        })
    }

    fn policy(&self) -> BoxFuture<'_, Result<PolicyView, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let dto: PolicyDto = self.get(path::policy()).await?;
            PolicyView::try_from(dto).map_err(unreadable)
        })
    }

    fn reload_policy(&self) -> BoxFuture<'_, Result<PolicyView, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let dto: PolicyDto = self
                .transport
                .json(Call::post_empty(path::policy_reload()))
                .await?;
            PolicyView::try_from(dto).map_err(unreadable)
        })
    }

    fn audit(&self, query: AuditQuery) -> BoxFuture<'_, Result<AuditPage, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let dto: AuditPageDto = self
                .get(path::audit(
                    query.limit(),
                    query.cursor().map(|cursor| cursor.seq()),
                ))
                .await?;
            AuditPage::try_from(dto).map_err(unreadable)
        })
    }

    fn issue_token(&self, label: AgentLabel) -> BoxFuture<'_, Result<IssuedToken, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let request = TokenIssueRequest {
                label: label.as_str().to_owned(),
            };
            let dto: IssuedTokenDto = self.post(path::tokens(), &request).await?;
            IssuedToken::try_from(dto).map_err(unreadable)
        })
    }

    fn tokens(&self) -> BoxFuture<'_, Result<Vec<TokenRecord>, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let list: TokenListDto = self.get(path::tokens()).await?;
            list.tokens
                .into_iter()
                .map(|dto: TokenDto| TokenRecord::try_from(dto).map_err(unreadable))
                .collect()
        })
    }

    fn revoke_token(&self, id: TokenId) -> BoxFuture<'_, Result<TokenRecord, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let dto: TokenDto = self
                .transport
                .json(Call::delete(path::token(id.as_str())))
                .await?;
            TokenRecord::try_from(dto).map_err(unreadable)
        })
    }
}
