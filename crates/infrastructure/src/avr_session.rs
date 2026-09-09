//! Persistent asynchronous AVR TCP sessions.

use denon_avr_application::controller::{ReceiverSession, SessionEvent as ControllerSessionEvent};
use denon_avr_application::{
    AsyncControlGateway, AsyncStatusGateway, BoxFuture, OperationError, OperationErrorKind,
    SessionEvent, SourceCatalogReader,
};
use denon_avr_domain::{
    AudioContextSnapshot, Confidence, ConnectionState, EqEvidence, EqFeature, EqState, EqStatus,
    FieldError, FieldErrorKind, Freshness, HttpInformationSnapshot, MainZoneEvent, MainZoneField,
    MainZoneSnapshot, MainZoneValue, Observed, QuickSelectRecallConfirmation, QuickSelectSlot,
    RawObservation, StateAuthority, SurroundMode,
};
use denon_avr_protocol::avr::AvrCommand;
use denon_avr_protocol::avr::{eq_status_query, parse_eq_status, quick_select_command};
use denon_avr_protocol::{
    get_command_family, parse_main_zone_event, parse_main_zone_response, query_command,
    response_matches,
};
use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{lookup_host, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

const DEFAULT_MAX_LINE_LENGTH: usize = 135;

#[derive(Debug, Clone)]
pub struct AvrSessionConfig {
    pub connect_timeout: Duration,
    pub write_timeout: Duration,
    pub response_timeout: Duration,
    pub reconnect_attempts: usize,
    pub reconnect_delay: Duration,
    pub max_line_length: usize,
    pub app_command_port: u16,
}

impl Default for AvrSessionConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(2),
            write_timeout: Duration::from_secs(1),
            response_timeout: Duration::from_secs(1),
            reconnect_attempts: 3,
            reconnect_delay: Duration::from_millis(250),
            max_line_length: DEFAULT_MAX_LINE_LENGTH,
            app_command_port: 8080,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AvrSessionEvent {
    Connected,
    Reconnected,
    Line(String),
    Disconnected(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AvrSessionError {
    InvalidCommand(String),
    Connection(String),
    Timeout(String),
    Disconnected(String),
    MalformedFrame(String),
    UnexpectedResponse(String),
    ReceiverError(String),
    SessionStopped,
}

impl fmt::Display for AvrSessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCommand(message) => write!(f, "invalid AVR command: {message}"),
            Self::Connection(message) => write!(f, "AVR connection failed: {message}"),
            Self::Timeout(message) => write!(f, "AVR session timed out: {message}"),
            Self::Disconnected(message) => write!(f, "AVR disconnected: {message}"),
            Self::MalformedFrame(message) => write!(f, "malformed AVR frame: {message}"),
            Self::UnexpectedResponse(message) => write!(f, "unexpected AVR response: {message}"),
            Self::ReceiverError(message) => write!(f, "receiver reported an error: {message}"),
            Self::SessionStopped => f.write_str("AVR session stopped"),
        }
    }
}

impl std::error::Error for AvrSessionError {}

impl From<AvrSessionError> for OperationError {
    fn from(error: AvrSessionError) -> Self {
        let kind = match error {
            AvrSessionError::InvalidCommand(_) | AvrSessionError::UnexpectedResponse(_) => {
                OperationErrorKind::Malformed
            }
            AvrSessionError::Connection(_) => OperationErrorKind::Connection,
            AvrSessionError::Timeout(_) => OperationErrorKind::Timeout,
            AvrSessionError::Disconnected(_) => OperationErrorKind::Disconnected,
            AvrSessionError::MalformedFrame(_) => OperationErrorKind::Malformed,
            AvrSessionError::ReceiverError(_) => OperationErrorKind::Unsupported,
            AvrSessionError::SessionStopped => OperationErrorKind::Stopped,
        };
        OperationError::new(kind, "using AVR session", error.to_string())
    }
}

struct Request {
    command: AvrCommand,
    response: Option<oneshot::Sender<Result<String, AvrSessionError>>>,
    dispatched: Option<oneshot::Sender<Result<(), AvrSessionError>>>,
}

pub struct AvrSession {
    requests: mpsc::Sender<Request>,
    events: mpsc::Receiver<AvrSessionEvent>,
    generation: Arc<AtomicU64>,
    snapshot: Arc<Mutex<MainZoneSnapshot>>,
    task_handle: JoinHandle<()>,
    source_catalog_host: String,
    source_catalog_port: u16,
    source_catalog_timeout: Duration,
}

/// Production session factory used by persistent application clients.
#[derive(Debug, Clone, Default)]
pub struct AvrSessionFactory {
    pub config: AvrSessionConfig,
}

impl denon_avr_application::SessionFactory for AvrSessionFactory {
    fn connect(
        &self,
        identity: denon_avr_domain::ReceiverIdentity,
    ) -> BoxFuture<'_, Result<Box<dyn ReceiverSession>, OperationError>> {
        let config = self.config.clone();
        Box::pin(async move {
            AvrSession::connect(&identity.host, config)
                .await
                .map(|session| Box::new(session) as Box<dyn ReceiverSession>)
                .map_err(OperationError::from)
        })
    }
}

impl AvrSession {
    pub async fn connect(host: &str, config: AvrSessionConfig) -> Result<Self, AvrSessionError> {
        let address = tokio::time::timeout(config.connect_timeout, lookup_host((host, 23)))
            .await
            .map_err(|_| AvrSessionError::Timeout("resolving receiver address".to_owned()))?
            .map_err(|error| AvrSessionError::Connection(error.to_string()))?
            .next()
            .ok_or_else(|| AvrSessionError::Connection("host has no address".to_owned()))?;
        let mut session = Self::connect_addr(address, config).await?;
        session.source_catalog_host = host.to_owned();
        Ok(session)
    }

    pub async fn connect_addr(
        address: SocketAddr,
        config: AvrSessionConfig,
    ) -> Result<Self, AvrSessionError> {
        let source_catalog_port = config.app_command_port;
        let source_catalog_timeout = config.response_timeout;
        let reader = connect_socket(address, &config).await?;
        let (requests, request_rx) = mpsc::channel(16);
        let (event_tx, events) = mpsc::channel(32);
        let generation = Arc::new(AtomicU64::new(0));
        let snapshot = Arc::new(Mutex::new(MainZoneSnapshot::default()));
        let task_handle = tokio::spawn(run_session(
            address,
            config,
            reader,
            request_rx,
            event_tx,
            Arc::clone(&generation),
            Arc::clone(&snapshot),
        ));
        Ok(Self {
            requests,
            events,
            generation,
            snapshot,
            task_handle,
            source_catalog_host: address.ip().to_string(),
            source_catalog_port,
            source_catalog_timeout,
        })
    }

    pub async fn request(&self, command: impl Into<String>) -> Result<String, AvrSessionError> {
        let command = AvrCommand::new(command.into())
            .map_err(|error| AvrSessionError::InvalidCommand(error.to_string()))?;
        let (response, result) = oneshot::channel();
        self.requests
            .send(Request {
                command,
                response: Some(response),
                dispatched: None,
            })
            .await
            .map_err(|_| AvrSessionError::SessionStopped)?;
        result.await.map_err(|_| AvrSessionError::SessionStopped)?
    }

    /// Serialize a state-changing command onto the AVR session without
    /// requiring an echo response. The controller confirms its outcome with
    /// a subsequent authoritative status query.
    pub async fn dispatch(&self, command: impl Into<String>) -> Result<(), AvrSessionError> {
        let command = AvrCommand::new(command.into())
            .map_err(|error| AvrSessionError::InvalidCommand(error.to_string()))?;
        let (dispatched, result) = oneshot::channel();
        self.requests
            .send(Request {
                command,
                response: None,
                dispatched: Some(dispatched),
            })
            .await
            .map_err(|_| AvrSessionError::SessionStopped)?;
        result.await.map_err(|_| AvrSessionError::SessionStopped)?
    }

    /// Send a read-only query again when the connection was lost while it was
    /// being serviced. Repeating actions after reconnect could be unsafe, so
    /// this helper is intentionally separate from [`Self::request`].
    pub async fn request_query(
        &self,
        command: impl Into<String>,
    ) -> Result<String, AvrSessionError> {
        let command = command.into();
        match self.request(command.clone()).await {
            Err(error) if retryable_query_error(&error) => self.request(command).await,
            result => result,
        }
    }

    pub async fn next_event(&mut self) -> Option<AvrSessionEvent> {
        self.events.recv().await
    }

    /// Monotonically increasing connection generation. It changes after every
    /// successful automatic reconnect and lets snapshot operations detect
    /// state that may have been missed during the reconnect.
    pub fn connection_generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub fn snapshot(&self) -> MainZoneSnapshot {
        self.snapshot.lock().expect("session snapshot lock").clone()
    }
}

impl AsyncStatusGateway for AvrSession {
    fn query_field(
        &mut self,
        field: MainZoneField,
    ) -> BoxFuture<'_, Result<MainZoneValue, OperationError>> {
        Box::pin(async move {
            let command = query_command(field);
            let response = self
                .request_query(command.as_str())
                .await
                .map_err(OperationError::from)?;
            parse_main_zone_response(field, &response).map_err(|error| {
                let kind = if matches!(
                    error,
                    denon_avr_protocol::avr::AvrProtocolError::Unavailable(_)
                ) {
                    OperationErrorKind::Unavailable
                } else {
                    OperationErrorKind::Malformed
                };
                OperationError::new(kind, "parsing AVR response", error.to_string())
            })
        })
    }

    fn connection_generation(&self) -> u64 {
        AvrSession::connection_generation(self)
    }

    fn next_event(&mut self) -> BoxFuture<'_, Result<SessionEvent, OperationError>> {
        Box::pin(async move {
            match AvrSession::next_event(self).await {
                Some(AvrSessionEvent::Connected | AvrSessionEvent::Reconnected) => {
                    Ok(SessionEvent::Connection(ConnectionState::Connected))
                }
                Some(AvrSessionEvent::Disconnected(_)) => {
                    Ok(SessionEvent::Connection(ConnectionState::Reconnecting))
                }
                Some(AvrSessionEvent::Line(line)) => Ok(match parse_main_zone_event(&line) {
                    MainZoneEvent::Unknown(_) => {
                        SessionEvent::MainZone(MainZoneEvent::Unknown(line))
                    }
                    event => SessionEvent::MainZone(event),
                }),
                None => Err(OperationError::new(
                    OperationErrorKind::Stopped,
                    "receiving AVR event",
                    "session event stream ended",
                )),
            }
        })
    }

    fn query_audio_context(&mut self) -> BoxFuture<'_, AudioContextSnapshot> {
        Box::pin(async move {
            let mut snapshot = AudioContextSnapshot::default();
            for command in ["SI?", "SD?", "DC?", "MS?", "CV?"] {
                let started = std::time::Instant::now();
                let result = self.request_query(command).await;
                snapshot.record_raw(
                    command,
                    RawObservation {
                        response: result.as_ref().ok().cloned(),
                        error: result.as_ref().err().map(ToString::to_string),
                        elapsed_millis: started.elapsed().as_millis(),
                    },
                );
                match (command, result) {
                    ("SI?", Ok(value)) => {
                        if let Some(value) = value.strip_prefix("SI").filter(|v| !v.is_empty()) {
                            snapshot.input_selection =
                                Observed::known(value.to_owned(), "SI?", Confidence::Observed);
                        } else {
                            snapshot.input_selection = Observed::malformed("SI?");
                        }
                    }
                    ("SD?", Ok(value)) => {
                        if let Some(value) = value.strip_prefix("SD").filter(|v| !v.is_empty()) {
                            snapshot.input_mode =
                                Observed::known(value.to_owned(), "SD?", Confidence::Observed);
                        } else {
                            snapshot.input_mode = Observed::malformed("SD?");
                        }
                    }
                    ("DC?", Ok(value)) => {
                        if let Some(value) = value.strip_prefix("DC").filter(|v| !v.is_empty()) {
                            snapshot.digital_mode =
                                Observed::known(value.to_owned(), "DC?", Confidence::Observed);
                        } else {
                            snapshot.digital_mode = Observed::malformed("DC?");
                        }
                    }
                    ("MS?", Ok(value)) => {
                        if let Some(value) = value.strip_prefix("MS").filter(|v| !v.is_empty()) {
                            if let Ok(value) = SurroundMode::new(value) {
                                snapshot.current_mode =
                                    Observed::known(value, "MS?", Confidence::Observed);
                            } else {
                                snapshot.current_mode = Observed::malformed("MS?");
                            }
                        } else {
                            snapshot.current_mode = Observed::malformed("MS?");
                        }
                    }
                    ("CV?", Ok(value)) => {
                        if let Some(value) = value.strip_prefix("CV").filter(|v| !v.is_empty()) {
                            snapshot.channel_volume =
                                Observed::known(value.to_owned(), "CV?", Confidence::Observed);
                        } else {
                            snapshot.channel_volume = Observed::malformed("CV?");
                        }
                    }
                    (_, Err(error)) => {
                        let field = FieldError {
                            kind: FieldErrorKind::Unavailable,
                            message: error.to_string(),
                        };
                        match command {
                            "SD?" => {
                                snapshot.input_mode = Observed::unavailable(field.clone(), "SD?")
                            }
                            "DC?" => {
                                snapshot.digital_mode = Observed::unavailable(field.clone(), "DC?")
                            }
                            "MS?" => {
                                snapshot.current_mode = Observed::unavailable(field.clone(), "MS?")
                            }
                            "CV?" => {
                                snapshot.channel_volume =
                                    Observed::unavailable(field.clone(), "CV?")
                            }
                            "SI?" => snapshot.input_selection = Observed::unavailable(field, "SI?"),
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            snapshot
        })
    }
}

impl AsyncControlGateway for AvrSession {
    fn execute_once(
        &mut self,
        control: denon_avr_domain::MainZoneControl,
    ) -> BoxFuture<'_, Result<(), OperationError>> {
        Box::pin(async move {
            let command = denon_avr_protocol::avr::encode_control(&control).map_err(|error| {
                OperationError::new(
                    OperationErrorKind::Malformed,
                    "encoding control command",
                    error.to_string(),
                )
            })?;
            self.dispatch(command.as_str())
                .await
                .map_err(OperationError::from)
        })
    }
}

impl SourceCatalogReader for AvrSession {
    fn refresh_source_catalog(
        &mut self,
    ) -> BoxFuture<'_, Result<denon_avr_domain::SourceCatalogObservation, OperationError>> {
        <Self as ReceiverSession>::refresh_source_catalog(self)
    }
}

impl ReceiverSession for AvrSession {
    fn query_field(
        &mut self,
        field: MainZoneField,
    ) -> BoxFuture<'_, Result<MainZoneValue, OperationError>> {
        <Self as AsyncStatusGateway>::query_field(self, field)
    }

    fn execute_once(
        &mut self,
        control: denon_avr_domain::MainZoneControl,
    ) -> BoxFuture<'_, Result<(), OperationError>> {
        <Self as AsyncControlGateway>::execute_once(self, control)
    }

    fn next_event(&mut self) -> BoxFuture<'_, Result<ControllerSessionEvent, OperationError>> {
        Box::pin(async move {
            match <Self as AsyncStatusGateway>::next_event(self).await? {
                SessionEvent::Connection(state) => Ok(ControllerSessionEvent::Connection(state)),
                SessionEvent::MainZone(event) => Ok(ControllerSessionEvent::MainZone(event)),
            }
        })
    }

    fn close(&mut self) -> BoxFuture<'_, Result<(), OperationError>> {
        Box::pin(async {
            let task_handle =
                std::mem::replace(&mut self.task_handle, tokio::task::spawn(async {}));
            task_handle.abort();
            match task_handle.await {
                Ok(()) => Ok(()),
                Err(join_error) => {
                    // Treat normal cancellation as successful closure
                    if join_error.is_cancelled() {
                        Ok(())
                    } else {
                        Err(OperationError::new(
                            OperationErrorKind::Stopped,
                            "session shutdown",
                            format!("unexpected task join error: {}", join_error),
                        ))
                    }
                }
            }
        })
    }

    fn query_audio_context(&mut self) -> BoxFuture<'_, AudioContextSnapshot> {
        <Self as AsyncStatusGateway>::query_audio_context(self)
    }

    fn refresh_http_information(
        &mut self,
    ) -> BoxFuture<'_, Result<HttpInformationSnapshot, OperationError>> {
        let host = self.source_catalog_host.clone();
        let timeout = self.source_catalog_timeout;
        let generation = self.generation.load(Ordering::SeqCst);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                crate::HttpInformationHttpClient::new(host, timeout)?.read(generation)
            })
            .await
            .map_err(|error| {
                OperationError::new(
                    OperationErrorKind::Stopped,
                    "HTTP information",
                    error.to_string(),
                )
            })?
            .map_err(|error| {
                OperationError::new(
                    OperationErrorKind::Connection,
                    "HTTP information",
                    error.to_string(),
                )
            })
        })
    }

    fn recall_quick_select(
        &mut self,
        slot: QuickSelectSlot,
    ) -> BoxFuture<'_, Result<QuickSelectRecallConfirmation, OperationError>> {
        Box::pin(async move {
            self.request(quick_select_command(slot).as_str())
                .await
                .map(|_| QuickSelectRecallConfirmation::Dispatched)
                .map_err(OperationError::from)
        })
    }

    fn query_eq_status(&mut self) -> BoxFuture<'_, Result<EqStatus, OperationError>> {
        Box::pin(async move {
            let mut status = EqStatus::default();
            for feature in EqFeature::ALL {
                let started = std::time::Instant::now();
                let result = self.request_query(eq_status_query(feature).as_str()).await;
                let (state, response, error) = match result {
                    Ok(response) => match parse_eq_status(feature, &response) {
                        Ok(state) => (state, Some(response), None),
                        Err(error) => (EqState::Unknown, Some(response), Some(error.to_string())),
                    },
                    Err(error) => {
                        let message = error.to_string();
                        (EqState::Unavailable(message.clone()), None, Some(message))
                    }
                };
                status.record_evidence(EqEvidence {
                    feature,
                    response,
                    error,
                    elapsed_millis: started.elapsed().as_millis(),
                    preserved_previous: false,
                });
                match feature {
                    EqFeature::MultEqXt32 => status.multeq_xt32 = state,
                    EqFeature::DynamicEq => status.dynamic_eq = state,
                    EqFeature::DynamicEqReferenceLevel => status.dynamic_eq_reference_level = state,
                    EqFeature::DynamicVolume => status.dynamic_volume = state,
                    EqFeature::AudysseyLfc => status.audyssey_lfc = state,
                    EqFeature::DiracLive => status.dirac_live = state,
                }
            }
            status.generation = self.connection_generation();
            status.freshness = if status.evidence.iter().any(|item| item.error.is_some()) {
                Freshness::Partial
            } else {
                Freshness::Live
            };
            Ok(status)
        })
    }

    fn refresh_source_catalog(
        &mut self,
    ) -> BoxFuture<'_, Result<denon_avr_domain::SourceCatalogObservation, OperationError>> {
        let endpoint = denon_avr_domain::ReceiverEndpoint {
            host: self.source_catalog_host.clone(),
            port: self.source_catalog_port,
        };
        let timeout = self.source_catalog_timeout;
        let generation = self.connection_generation();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                crate::SourceCatalogHttpClient::new(endpoint, timeout)
                    .and_then(|client| client.read(generation))
            })
            .await
            .map_err(|error| {
                OperationError::new(
                    OperationErrorKind::Stopped,
                    "reading source catalog",
                    error.to_string(),
                )
            })?
            .map_err(|error| {
                let kind = match error.kind() {
                    std::io::ErrorKind::TimedOut => OperationErrorKind::Timeout,
                    std::io::ErrorKind::InvalidData => OperationErrorKind::Malformed,
                    _ => OperationErrorKind::Connection,
                };
                OperationError::new(kind, "reading source catalog", error.to_string())
            })
        })
    }
}

async fn connect_socket(
    address: SocketAddr,
    config: &AvrSessionConfig,
) -> Result<BufReader<TcpStream>, AvrSessionError> {
    let stream = tokio::time::timeout(config.connect_timeout, TcpStream::connect(address))
        .await
        .map_err(|_| AvrSessionError::Timeout(format!("connecting to {address}")))?
        .map_err(|error| AvrSessionError::Connection(error.to_string()))?;
    Ok(BufReader::new(stream))
}

async fn run_session(
    address: SocketAddr,
    config: AvrSessionConfig,
    mut reader: BufReader<TcpStream>,
    mut requests: mpsc::Receiver<Request>,
    events: mpsc::Sender<AvrSessionEvent>,
    generation: Arc<AtomicU64>,
    snapshot: Arc<Mutex<MainZoneSnapshot>>,
) {
    if !publish_event(&events, AvrSessionEvent::Connected).await {
        return;
    }
    loop {
        let mut request = match next_request(
            &mut reader,
            &mut requests,
            &events,
            config.max_line_length,
            &snapshot,
        )
        .await
        {
            Ok(Some(request)) => request,
            Ok(None) => return,
            Err(message) => {
                invalidate_snapshot(&snapshot);
                if !publish_event(&events, AvrSessionEvent::Disconnected(message)).await {
                    return;
                }
                match reconnect(address, &config, &events).await {
                    Ok(new_reader) => {
                        reader = new_reader;
                        generation.fetch_add(1, Ordering::AcqRel);
                        if !publish_event(&events, AvrSessionEvent::Reconnected).await {
                            return;
                        }
                        continue;
                    }
                    Err(_) => return,
                }
            }
        };
        let family = get_command_family(request.command.as_str());
        let command_bytes = request.command.as_bytes();
        let write_result = tokio::time::timeout(
            config.write_timeout,
            reader.get_mut().write_all(&command_bytes),
        )
        .await
        .map_err(|_| {
            AvrSessionError::Timeout(format!("writing {} command", request.command.as_str()))
        })
        .and_then(|result| {
            result.map_err(|error| AvrSessionError::Disconnected(error.to_string()))
        });
        if let Err(error) = write_result {
            let message = error.to_string();
            if let Some(response) = request.response.take() {
                let _ = response.send(Err(error.clone()));
            }
            if let Some(dispatched) = request.dispatched.take() {
                let _ = dispatched.send(Err(error));
            }
            invalidate_snapshot(&snapshot);
            if !publish_event(&events, AvrSessionEvent::Disconnected(message)).await {
                return;
            }
            match reconnect(address, &config, &events).await {
                Ok(new_reader) => {
                    reader = new_reader;
                    generation.fetch_add(1, Ordering::AcqRel);
                    if !publish_event(&events, AvrSessionEvent::Reconnected).await {
                        return;
                    }
                }
                Err(_) => return,
            }
            continue;
        }

        if let Some(dispatched) = request.dispatched.take() {
            let _ = dispatched.send(Ok(()));
            continue;
        }
        let Some(response_sender) = request.response.take() else {
            continue;
        };

        let result = read_response(
            &mut reader,
            family,
            config.response_timeout,
            config.max_line_length,
            &events,
            &snapshot,
        )
        .await;
        match result {
            Ok(response) => {
                apply_response(&snapshot, family, &response);
                let _ = response_sender.send(Ok(response));
            }
            Err(error @ AvrSessionError::Timeout(_))
            | Err(error @ AvrSessionError::Disconnected(_))
            | Err(error @ AvrSessionError::MalformedFrame(_)) => {
                let message = error.to_string();
                let _ = response_sender.send(Err(error));
                invalidate_snapshot(&snapshot);
                if !publish_event(&events, AvrSessionEvent::Disconnected(message)).await {
                    return;
                }
                match reconnect(address, &config, &events).await {
                    Ok(new_reader) => {
                        reader = new_reader;
                        generation.fetch_add(1, Ordering::AcqRel);
                        if !publish_event(&events, AvrSessionEvent::Reconnected).await {
                            return;
                        }
                    }
                    Err(_) => return,
                }
            }
            Err(error) => {
                let _ = response_sender.send(Err(error));
            }
        }
    }
}

async fn next_request(
    reader: &mut BufReader<TcpStream>,
    requests: &mut mpsc::Receiver<Request>,
    events: &mpsc::Sender<AvrSessionEvent>,
    max_line_length: usize,
    snapshot: &Arc<Mutex<MainZoneSnapshot>>,
) -> Result<Option<Request>, String> {
    loop {
        tokio::select! {
            request = requests.recv() => return Ok(request),
            result = read_frame(reader, max_line_length) => {
                match result {
                    Ok(line) => {
                        let text = frame_text(&line, max_line_length)
                            .map_err(|error| error.to_string())?;
                        apply_line(snapshot, &text, StateAuthority::Event);
                        if !publish_event(events, AvrSessionEvent::Line(text)).await {
                            return Ok(None);
                        }
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
        }
    }
}

async fn read_response(
    reader: &mut BufReader<TcpStream>,
    family: &str,
    timeout: Duration,
    max_line_length: usize,
    events: &mpsc::Sender<AvrSessionEvent>,
    snapshot: &Arc<Mutex<MainZoneSnapshot>>,
) -> Result<String, AvrSessionError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(AvrSessionError::Timeout(format!(
                "waiting for {family} response"
            )));
        }
        let line = tokio::time::timeout(remaining, read_frame(reader, max_line_length))
            .await
            .map_err(|_| AvrSessionError::Timeout(format!("waiting for {family} response")))??;
        let text = frame_text(&line, max_line_length)?;
        if response_matches(family, &text) {
            return Ok(text);
        }
        apply_line(snapshot, &text, StateAuthority::Event);
        if !publish_event(events, AvrSessionEvent::Line(text)).await {
            return Err(AvrSessionError::SessionStopped);
        }
    }
}

async fn read_frame(
    reader: &mut BufReader<TcpStream>,
    max_line_length: usize,
) -> Result<Vec<u8>, AvrSessionError> {
    let mut line = Vec::with_capacity(max_line_length.saturating_add(1));
    loop {
        let byte = match reader.read_u8().await {
            Ok(byte) => byte,
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                if line.is_empty() {
                    return Err(AvrSessionError::Disconnected(
                        "receiver closed the socket".to_owned(),
                    ));
                }
                return Err(AvrSessionError::MalformedFrame(
                    "AVR line was not terminated by CR".to_owned(),
                ));
            }
            Err(error) => return Err(AvrSessionError::Disconnected(error.to_string())),
        };
        line.push(byte);
        if byte == b'\r' {
            return Ok(line);
        }
        if line.len() > max_line_length {
            return Err(AvrSessionError::MalformedFrame(format!(
                "AVR response exceeded {max_line_length} bytes"
            )));
        }
    }
}

fn frame_text(line: &[u8], max_line_length: usize) -> Result<String, AvrSessionError> {
    if line.last() != Some(&b'\r') {
        return Err(AvrSessionError::MalformedFrame(
            "AVR line was not terminated by CR".to_owned(),
        ));
    }
    if line.len() - 1 > max_line_length {
        return Err(AvrSessionError::MalformedFrame(format!(
            "AVR response exceeded {max_line_length} bytes"
        )));
    }
    std::str::from_utf8(&line[..line.len() - 1])
        .map(str::to_owned)
        .map_err(|_| AvrSessionError::MalformedFrame("AVR line is not valid UTF-8".to_owned()))
}

fn apply_line(snapshot: &Arc<Mutex<MainZoneSnapshot>>, line: &str, authority: StateAuthority) {
    if line == "MV---" {
        if let Ok(mut state) = snapshot.lock() {
            state.set_error(
                MainZoneField::Volume,
                FieldError {
                    kind: FieldErrorKind::Unavailable,
                    message: "volume is unavailable".into(),
                },
            );
        }
        return;
    }
    apply_event(snapshot, line, authority);
}

fn apply_response(snapshot: &Arc<Mutex<MainZoneSnapshot>>, family: &str, response: &str) {
    let field = match family {
        "PW" => MainZoneField::Power,
        "SI" => MainZoneField::Input,
        "MV" => MainZoneField::Volume,
        "MU" => MainZoneField::Mute,
        "MS" => MainZoneField::SurroundMode,
        _ => return,
    };
    match denon_avr_protocol::avr::parse_main_zone_response(field, response) {
        Ok(value) => {
            if let Ok(mut state) = snapshot.lock() {
                state.set_value(value, StateAuthority::Authoritative);
            }
        }
        Err(denon_avr_protocol::avr::AvrProtocolError::Unavailable(message)) => {
            if let Ok(mut state) = snapshot.lock() {
                state.set_error(
                    field,
                    FieldError {
                        kind: FieldErrorKind::Unavailable,
                        message: message.into(),
                    },
                );
            }
        }
        Err(_) => {}
    }
}

fn apply_event(snapshot: &Arc<Mutex<MainZoneSnapshot>>, line: &str, authority: StateAuthority) {
    if let Ok(mut state) = snapshot.lock() {
        match authority {
            StateAuthority::Event => {
                state.apply_event(denon_avr_protocol::avr::parse_main_zone_event(line))
            }
            StateAuthority::Authoritative => state
                .apply_authoritative_event(denon_avr_protocol::avr::parse_main_zone_event(line)),
            StateAuthority::Unconfirmed => {}
        }
    }
}

fn retryable_query_error(error: &AvrSessionError) -> bool {
    matches!(
        error,
        AvrSessionError::Disconnected(_) | AvrSessionError::Timeout(_)
    )
}

fn invalidate_snapshot(snapshot: &Arc<Mutex<MainZoneSnapshot>>) {
    if let Ok(mut state) = snapshot.lock() {
        state.invalidate();
    }
}

async fn publish_event(events: &mpsc::Sender<AvrSessionEvent>, event: AvrSessionEvent) -> bool {
    events.send(event).await.is_ok()
}

async fn reconnect(
    address: SocketAddr,
    config: &AvrSessionConfig,
    events: &mpsc::Sender<AvrSessionEvent>,
) -> Result<BufReader<TcpStream>, AvrSessionError> {
    let mut last_error = None;
    for attempt in 0..config.reconnect_attempts {
        if attempt > 0 {
            tokio::time::sleep(config.reconnect_delay).await;
        }
        match connect_socket(address, config).await {
            Ok(reader) => return Ok(reader),
            Err(error) => {
                last_error = Some(error.to_string());
                if !publish_event(
                    events,
                    AvrSessionEvent::Disconnected(last_error.clone().unwrap()),
                )
                .await
                {
                    return Err(AvrSessionError::SessionStopped);
                }
            }
        }
    }
    Err(AvrSessionError::Connection(last_error.unwrap_or_else(
        || "reconnect attempts exhausted".to_owned(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn routes_unsolicited_lines_and_correlates_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut command = Vec::new();
            reader.read_until(b'\r', &mut command).await.unwrap();
            assert_eq!(command, b"MV?\r");
            reader.get_mut().write_all(b"SICD\rMV805\r").await.unwrap();
        });
        let mut session = AvrSession::connect_addr(address, AvrSessionConfig::default())
            .await
            .unwrap();
        assert_eq!(session.request("MV?").await.unwrap(), "MV805");
        assert_eq!(session.next_event().await, Some(AvrSessionEvent::Connected));
        assert_eq!(
            session.next_event().await,
            Some(AvrSessionEvent::Line("SICD".to_owned()))
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_oversized_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut command = Vec::new();
            reader.read_until(b'\r', &mut command).await.unwrap();
            reader
                .get_mut()
                .write_all(&[b'M'; DEFAULT_MAX_LINE_LENGTH + 2])
                .await
                .unwrap();
            reader.get_mut().write_all(b"\r").await.unwrap();
        });
        let config = AvrSessionConfig {
            reconnect_attempts: 0,
            ..AvrSessionConfig::default()
        };
        let session = AvrSession::connect_addr(address, config).await.unwrap();
        assert!(matches!(
            session.request("MV?").await,
            Err(AvrSessionError::MalformedFrame(_))
        ));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn returns_bounded_response_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut command = Vec::new();
            reader.read_until(b'\r', &mut command).await.unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
        });
        let config = AvrSessionConfig {
            response_timeout: Duration::from_millis(10),
            reconnect_attempts: 0,
            ..AvrSessionConfig::default()
        };
        let session = AvrSession::connect_addr(address, config).await.unwrap();
        assert!(matches!(
            session.request("MV?").await,
            Err(AvrSessionError::Timeout(_))
        ));
        server.await.unwrap();
    }

    #[test]
    fn retries_only_read_query_failures() {
        assert!(retryable_query_error(&AvrSessionError::Timeout(
            "timeout".into()
        )));
        assert!(retryable_query_error(&AvrSessionError::Disconnected(
            "closed".into()
        )));
        assert!(!retryable_query_error(&AvrSessionError::MalformedFrame(
            "bad".into()
        )));
        assert!(!retryable_query_error(
            &AvrSessionError::UnexpectedResponse("bad".into())
        ));
    }

    #[test]
    fn malformed_frame_validation_is_bounded() {
        assert!(matches!(
            frame_text(&[], 1),
            Err(AvrSessionError::MalformedFrame(_))
        ));
        assert!(matches!(
            frame_text(b"xx\r", 1),
            Err(AvrSessionError::MalformedFrame(_))
        ));
    }

    #[test]
    fn keeps_numeric_query_suffixes_in_the_response_family() {
        assert_eq!(get_command_family("Z2?"), "Z2");
        assert!(response_matches("MV", "MV805"));
        assert!(!response_matches("MV", "MVMAX 615"));
    }

    #[tokio::test]
    async fn routes_same_family_notifications_before_the_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut command = Vec::new();
            reader.read_until(b'\r', &mut command).await.unwrap();
            assert_eq!(command, b"MV?\r");
            reader
                .get_mut()
                .write_all(b"MVMAX 615\rMV025\r")
                .await
                .unwrap();
        });
        let mut session = AvrSession::connect_addr(address, AvrSessionConfig::default())
            .await
            .unwrap();
        assert_eq!(session.request("MV?").await.unwrap(), "MV025");
        assert_eq!(session.next_event().await, Some(AvrSessionEvent::Connected));
        assert_eq!(
            session.next_event().await,
            Some(AvrSessionEvent::Line("MVMAX 615".to_owned()))
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn dispatches_control_without_waiting_for_an_echo() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut command = Vec::new();
            reader.read_until(b'\r', &mut command).await.unwrap();
            assert_eq!(command, b"SIGAME\r");
        });
        let mut session = AvrSession::connect_addr(address, AvrSessionConfig::default())
            .await
            .unwrap();

        <AvrSession as AsyncControlGateway>::execute_once(
            &mut session,
            denon_avr_domain::MainZoneControl::Input(denon_avr_domain::Input::new("GAME").unwrap()),
        )
        .await
        .unwrap();

        server.await.unwrap();
    }

    #[tokio::test]
    async fn reports_write_disconnect_and_exhausts_reconnects() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            drop(stream);
        });
        let config = AvrSessionConfig {
            reconnect_attempts: 0,
            ..AvrSessionConfig::default()
        };
        let session = AvrSession::connect_addr(address, config).await.unwrap();
        assert!(matches!(
            session.request("MV?").await,
            Err(AvrSessionError::Disconnected(_)) | Err(AvrSessionError::Timeout(_))
        ));
        server.await.unwrap();
        assert!(matches!(
            session.request("MV?").await,
            Err(AvrSessionError::SessionStopped)
        ));
    }

    #[tokio::test]
    async fn reconnects_after_idle_disconnect_before_next_request() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            drop(stream);
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut command = Vec::new();
            reader.read_until(b'\r', &mut command).await.unwrap();
            reader.get_mut().write_all(b"PWON\r").await.unwrap();
        });
        let config = AvrSessionConfig {
            reconnect_delay: Duration::ZERO,
            ..AvrSessionConfig::default()
        };
        let mut session = AvrSession::connect_addr(address, config).await.unwrap();
        while session.next_event().await != Some(AvrSessionEvent::Reconnected) {}
        assert_eq!(session.request("PW?").await.unwrap(), "PWON");
        server.await.unwrap();
    }
}
