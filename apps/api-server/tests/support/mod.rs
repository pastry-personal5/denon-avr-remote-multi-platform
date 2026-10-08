//! Shared by the server's tests: a fake receiver, a fixture that runs a real
//! server on a short path under `/tmp` (a socket path is limited to about a
//! hundred bytes, and the system's temporary directory is long on macOS), and a raw
//! HTTP client that writes exactly the bytes it is told to.

#![allow(dead_code)]

use denon_avr_api_contract::EndpointPaths;
use denon_avr_api_server::{
    AgentEndpointConfig, Limits, RunningServer, Server, ServerConfig, StartError,
};
use denon_avr_application::ports::{
    AsyncConfigRepository, AsyncReceiverDiscovery, BoxFuture, OperationError, ReceiverConnector,
};
use denon_avr_application::{
    AgentLabel, AgentLimits, AgentPath, CanonicalReceiverSession, ControlService, OperationRequest,
    Readiness, ServiceConfig, SharedReceiverSession, SharedTokenStore, StateSubscription,
    TokenStore,
};
use denon_avr_domain::{
    ConfiguredReceivers, DiscoveredReceiver, DispatchCertainty, Epoch, FrameSeq, MasterVolume,
    MonotonicMillis, MuteState, ObservationOrigin, OperationOutcome, ReceiverId, ReceiverIdentity,
    ReceiverIntent, ReceiverObservation, ReceiverState, SoundModeStatus, SourceCatalog,
    SourceCatalogObservation, SourceEntry, SourceId, SourceVisibility, SystemPower, WallTime,
    ZonePower,
};
use denon_avr_infrastructure::{FileTokenStore, JsonlAuditLog, SystemClock, YamlPolicySource};
use serde_json::Value;
use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::watch;

pub fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub fn living_room() -> ReceiverId {
    ReceiverId::new("living-room").unwrap()
}

fn observation<T>(value: T) -> ReceiverObservation<T> {
    ReceiverObservation {
        receiver: living_room(),
        epoch: Epoch(1),
        frame_seq: FrameSeq(1),
        observed_at: MonotonicMillis(0),
        origin: ObservationOrigin::ReceiverFrame,
        value,
    }
}

pub fn master(db: f64) -> MasterVolume {
    MasterVolume::db_half_steps((db * 2.0).round() as i16).unwrap()
}

/// A receiver with an established connection and every field current.
pub fn receiver_state(volume_db: f64) -> ReceiverState {
    let mut state = ReceiverState::new(living_room());
    state.establish_epoch(Epoch(1));
    let valid = MonotonicMillis(u64::MAX);
    state
        .system_power
        .observe(observation(SystemPower::On), valid);
    state
        .main_zone
        .power
        .observe(observation(ZonePower::On), valid);
    state
        .main_zone
        .source
        .observe(observation(SourceId::new("BD").unwrap()), valid);
    state
        .main_zone
        .volume
        .observe(observation(master(volume_db)), valid);
    state
        .main_zone
        .mute
        .observe(observation(MuteState::On), valid);
    state.main_zone.sound_mode.observe(
        observation(SoundModeStatus {
            id: "DIRECT".into(),
            raw: "DIRECT".into(),
        }),
        valid,
    );
    state
}

// ---- A fake receiver ----

pub struct FakeSession {
    pub states: watch::Sender<ReceiverState>,
    pub calls: Mutex<Vec<OperationRequest>>,
    pub closed: AtomicBool,
    /// While false, a write waits: an operation that is in the session and has not
    /// finished.
    gate: watch::Receiver<bool>,
}

impl FakeSession {
    /// Change the receiver's state as if someone turned a dial.
    pub fn turn_volume(&self, db: f64) {
        self.states.send_modify(|state| {
            state
                .main_zone
                .volume
                .observe(observation(master(db)), MonotonicMillis(u64::MAX));
            state.revision.0 += 1;
        });
    }
}

fn apply(state: &mut ReceiverState, intent: &ReceiverIntent) {
    let valid = MonotonicMillis(u64::MAX);
    match intent {
        ReceiverIntent::SystemPower(power) => {
            state.system_power.observe(observation(*power), valid)
        }
        ReceiverIntent::MainZonePower(power) => {
            state.main_zone.power.observe(observation(*power), valid)
        }
        ReceiverIntent::Zone2Power(power) => state.zone2_power.observe(observation(*power), valid),
        ReceiverIntent::Source(source) => state
            .main_zone
            .source
            .observe(observation(source.clone()), valid),
        ReceiverIntent::Volume(volume) => {
            state.main_zone.volume.observe(observation(*volume), valid)
        }
        ReceiverIntent::Mute(mute) => state.main_zone.mute.observe(observation(*mute), valid),
        ReceiverIntent::SoundMode(_) => {}
    }
    state.revision.0 += 1;
}

impl CanonicalReceiverSession for FakeSession {
    fn state(&self) -> StateSubscription {
        StateSubscription::new(self.states.subscribe())
    }

    fn synchronize(&self) -> BoxFuture<'_, Result<Readiness, OperationError>> {
        Box::pin(async {
            Ok(Readiness {
                ready: true,
                degraded: false,
                detail: "fake".into(),
            })
        })
    }

    fn operate(&self, request: OperationRequest) -> BoxFuture<'_, OperationOutcome> {
        Box::pin(async move {
            locked(&self.calls).push(request.clone());
            let mut gate = self.gate.clone();
            let _ = gate.wait_for(|open| *open).await;
            if let Some(precondition) = &request.precondition {
                if let Some(mismatch) = precondition.mismatch(&self.states.borrow()) {
                    return OperationOutcome::RejectedBeforeDispatch {
                        operation: request.id,
                        cause: denon_avr_domain::RejectionCause::PreconditionMismatch(mismatch),
                        reason: "the receiver changed since the decision".into(),
                    };
                }
            }
            self.states
                .send_modify(|state| apply(state, &request.intent));
            OperationOutcome::ObservedRequestedValue {
                operation: request.id,
                dispatch: DispatchCertainty::CompleteWrite,
                observation: "applied".into(),
            }
        })
    }

    fn source_catalog(&self) -> BoxFuture<'_, Result<SourceCatalogObservation, OperationError>> {
        Box::pin(async {
            Ok(SourceCatalogObservation {
                catalog: SourceCatalog {
                    entries: vec![SourceEntry {
                        id: SourceId::new("BD").unwrap(),
                        display_name: Some("Blu-ray".into()),
                        visibility: SourceVisibility::Shown,
                    }],
                    generation: 7,
                    error: Some("timed out talking to 192.0.2.10".into()),
                    ..SourceCatalog::default()
                },
                raw_response: "<list host=\"192.0.2.10\"/>".into(),
                response_evidence: denon_avr_domain::CatalogResponseEvidence::Complete,
            })
        })
    }

    fn quick_select_names(
        &self,
    ) -> BoxFuture<'_, Result<denon_avr_domain::QuickSelectNameObservation, OperationError>> {
        Box::pin(async {
            Ok(denon_avr_domain::QuickSelectNameObservation {
                generation: 2,
                ..Default::default()
            })
        })
    }

    fn http_information(
        &self,
    ) -> BoxFuture<'_, Result<denon_avr_domain::HttpInformationSnapshot, OperationError>> {
        Box::pin(async {
            Ok(denon_avr_domain::HttpInformationSnapshot {
                generation: 3,
                ..Default::default()
            })
        })
    }

    fn close(&self) -> BoxFuture<'_, Result<(), OperationError>> {
        Box::pin(async {
            self.closed.store(true, Ordering::SeqCst);
            Ok(())
        })
    }
}

pub struct FakeConnector {
    pub sessions: Mutex<Vec<Arc<FakeSession>>>,
    pub opens: AtomicUsize,
    pub initial: Mutex<ReceiverState>,
    /// When set, a connection fails with this text, as an unreachable receiver does.
    pub fail: Mutex<Option<String>>,
    gate: watch::Sender<bool>,
}

impl FakeConnector {
    pub fn new(volume_db: f64) -> Arc<Self> {
        Arc::new(Self {
            sessions: Mutex::new(Vec::new()),
            opens: AtomicUsize::new(0),
            initial: Mutex::new(receiver_state(volume_db)),
            fail: Mutex::new(None),
            gate: watch::channel(true).0,
        })
    }

    /// From now on a write reaches the receiver and waits there, so an operation
    /// stays in the session until [`FakeConnector::release_operations`].
    pub fn hold_operations(&self) {
        self.gate.send_replace(false);
    }

    pub fn release_operations(&self) {
        self.gate.send_replace(true);
    }

    /// Sessions opened and not yet closed.
    pub fn open_sessions(&self) -> usize {
        locked(&self.sessions)
            .iter()
            .filter(|session| !session.closed.load(Ordering::SeqCst))
            .count()
    }

    pub fn latest(&self) -> Arc<FakeSession> {
        Arc::clone(locked(&self.sessions).last().expect("a session was opened"))
    }

    pub fn operate_calls(&self) -> usize {
        locked(&self.sessions)
            .iter()
            .map(|session| locked(&session.calls).len())
            .sum()
    }
}

impl ReceiverConnector for FakeConnector {
    fn connect<'a>(
        &'a self,
        receiver: &'a ReceiverId,
        _: &'a ReceiverIdentity,
    ) -> BoxFuture<'a, Result<SharedReceiverSession, OperationError>> {
        Box::pin(async move {
            self.opens.fetch_add(1, Ordering::SeqCst);
            if let Some(text) = locked(&self.fail).clone() {
                return Err(OperationError::new(
                    denon_avr_application::ports::OperationErrorKind::Connection,
                    "using AVR session",
                    text,
                ));
            }
            let mut state = locked(&self.initial).clone();
            state.receiver = receiver.clone();
            let session = Arc::new(FakeSession {
                states: watch::channel(state).0,
                calls: Mutex::new(Vec::new()),
                closed: AtomicBool::new(false),
                gate: self.gate.subscribe(),
            });
            locked(&self.sessions).push(Arc::clone(&session));
            Ok(session as SharedReceiverSession)
        })
    }
}

pub struct FakeConfig(pub Mutex<ConfiguredReceivers>);

impl AsyncConfigRepository for FakeConfig {
    fn load(&self) -> BoxFuture<'_, Result<ConfiguredReceivers, OperationError>> {
        Box::pin(async { Ok(locked(&self.0).clone()) })
    }

    fn save<'a>(
        &'a self,
        config: &'a ConfiguredReceivers,
    ) -> BoxFuture<'a, Result<(), OperationError>> {
        Box::pin(async move {
            *locked(&self.0) = config.clone();
            Ok(())
        })
    }
}

pub struct FakeDiscovery(pub Vec<DiscoveredReceiver>, pub Mutex<Option<Duration>>);

impl FakeDiscovery {
    pub fn new(found: Vec<DiscoveredReceiver>) -> Self {
        Self(found, Mutex::new(None))
    }
}

impl AsyncReceiverDiscovery for FakeDiscovery {
    fn discover(
        &self,
        timeout: Duration,
    ) -> BoxFuture<'_, Result<Vec<DiscoveredReceiver>, OperationError>> {
        Box::pin(async move {
            *locked(&self.1) = Some(timeout);
            Ok(self.0.clone())
        })
    }
}

// ---- The fixture ----

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A short, unique directory under `/tmp`, removed when dropped.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        let path = PathBuf::from(format!(
            "/tmp/dar-{name}-{}-{}",
            std::process::id() % 100_000,
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub const OWNERS_POLICY: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/examples/policy.yaml"
));

pub struct Options {
    pub agent_endpoint: bool,
    pub limits: Limits,
    pub volume_db: f64,
    pub policy: &'static str,
    /// Audit files this small rotate, to show what a flood would do.
    pub audit_limits: Option<denon_avr_infrastructure::AuditLimits>,
    /// The longest the service holds a wait for an operation, when not its default.
    pub max_operation_wait: Option<Duration>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            agent_endpoint: true,
            limits: Limits::default(),
            volume_db: -35.0,
            policy: OWNERS_POLICY,
            audit_limits: None,
            max_operation_wait: None,
        }
    }
}

pub struct Fixture {
    pub root: Arc<TempDir>,
    pub paths: EndpointPaths,
    pub service: Arc<ControlService>,
    pub tokens: Arc<FileTokenStore>,
    pub connector: Arc<FakeConnector>,
    pub config: Arc<FakeConfig>,
    pub discovery: Arc<FakeDiscovery>,
    pub audit_directory: PathBuf,
    pub server: Option<RunningServer>,
    pub agent_directory: PathBuf,
}

impl Fixture {
    pub async fn start(options: Options) -> Self {
        let root = Arc::new(TempDir::new("fx"));
        Self::start_in(root, options)
            .await
            .expect("the server starts")
    }

    pub async fn start_in(root: Arc<TempDir>, options: Options) -> Result<Self, StartError> {
        let paths = EndpointPaths::under(&root.0);
        let agent_directory = root.0.join("agent");
        let audit_directory = root.0.join("audit");
        std::fs::write(root.0.join("policy.yaml"), options.policy).unwrap();
        let tokens = Arc::new(FileTokenStore::open(&paths.credentials_directory).unwrap());
        let connector = FakeConnector::new(options.volume_db);
        let discovery = Arc::new(FakeDiscovery::new(Vec::new()));
        let config = Arc::new(FakeConfig(Mutex::new(ConfiguredReceivers {
            current: Some("living-room".into()),
            receivers: BTreeMap::from([(
                "living-room".into(),
                ReceiverIdentity {
                    host: "192.0.2.10".into(),
                    model: Some("AVR-X3800H".into()),
                    friendly_name: None,
                },
            )]),
            ..ConfiguredReceivers::default()
        })));
        let service = Arc::new(
            ControlService::start(
                connector.clone(),
                config.clone(),
                discovery.clone(),
                ServiceConfig {
                    idle_release: Duration::from_millis(200),
                    max_operation_wait: options
                        .max_operation_wait
                        .unwrap_or(ServiceConfig::default().max_operation_wait),
                    ..ServiceConfig::default()
                },
                AgentPath {
                    policy: Arc::new(YamlPolicySource::new(root.0.join("policy.yaml"))),
                    audit: Arc::new(match options.audit_limits {
                        Some(limits) => JsonlAuditLog::new(&audit_directory).with_limits(limits),
                        None => JsonlAuditLog::new(&audit_directory),
                    }),
                    clock: Arc::new(SystemClock),
                    limits: AgentLimits::default(),
                    tokens: Some(tokens.clone()),
                },
            )
            .await,
        );
        let agent = if options.agent_endpoint {
            std::fs::create_dir_all(&agent_directory).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&agent_directory, std::fs::Permissions::from_mode(0o700))
                .unwrap();
            Some(AgentEndpointConfig {
                directory: agent_directory.clone(),
                uids: vec![std::fs::metadata(&root.0).unwrap().uid()],
                mode: 0o600,
            })
        } else {
            None
        };
        let server = Server::start(
            service.clone(),
            tokens.clone() as SharedTokenStore,
            ServerConfig {
                paths: paths.clone(),
                agent,
                limits: options.limits,
            },
        )
        .await?;
        Ok(Self {
            root,
            paths,
            service,
            tokens,
            connector,
            config,
            discovery,
            audit_directory,
            server: Some(server),
            agent_directory,
        })
    }

    pub fn operator_socket(&self) -> &Path {
        self.server.as_ref().unwrap().operator_socket()
    }

    pub fn agent_socket(&self) -> &Path {
        self.server
            .as_ref()
            .unwrap()
            .agent_socket()
            .expect("the agent endpoint is on")
    }

    pub fn operator_token(&self) -> String {
        std::fs::read_to_string(&self.paths.operator_token)
            .unwrap()
            .trim()
            .to_owned()
    }

    /// Issue an Agent token under `label` and return its secret.
    pub async fn agent_token(&self, label: &str) -> String {
        let issued = self
            .tokens
            .issue(AgentLabel::new(label).unwrap(), WallTime(1))
            .await
            .unwrap();
        issued.secret.expose().to_owned()
    }

    pub async fn shutdown(mut self) {
        if let Some(server) = self.server.take() {
            server.shutdown().await;
        }
    }
}

// ---- A client that writes exactly what it is told ----

#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|why| {
            panic!(
                "not JSON ({why}): {:?}",
                String::from_utf8_lossy(&self.body)
            )
        })
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn code(&self) -> Option<String> {
        self.json()["error"]["code"].as_str().map(str::to_owned)
    }
}

pub fn parse_reply(bytes: &[u8]) -> Option<Reply> {
    let split = bytes.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&bytes[..split]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines.next()?.split(' ').nth(1)?.parse().ok()?;
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect();
    Some(Reply {
        status,
        headers,
        body: bytes[split + 4..].to_vec(),
    })
}

/// Send `bytes` as they are and read until the server closes the connection.
pub async fn send_raw(socket: &Path, bytes: &[u8]) -> Vec<u8> {
    let mut stream = UnixStream::connect(socket).await.expect("connect");
    // The server may close before it has read all of an oversized request.
    let _ = stream.write_all(bytes).await;
    let mut reply = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut reply)).await;
    reply
}

pub async fn request(
    socket: &Path,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&[u8]>,
) -> Reply {
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: dar\r\nConnection: close\r\n");
    if let Some(token) = token {
        head.push_str(&format!("Authorization: Bearer {token}\r\n"));
    }
    if let Some(body) = body {
        head.push_str("Content-Type: application/json\r\n");
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    } else if method == "POST" || method == "PUT" {
        head.push_str("Content-Length: 0\r\n");
    }
    head.push_str("\r\n");
    let mut bytes = head.into_bytes();
    if let Some(body) = body {
        bytes.extend_from_slice(body);
    }
    let reply = send_raw(socket, &bytes).await;
    parse_reply(&reply).unwrap_or_else(|| panic!("no reply: {:?}", String::from_utf8_lossy(&reply)))
}

pub async fn get(socket: &Path, path: &str, token: &str) -> Reply {
    request(socket, "GET", path, Some(token), None).await
}

/// The audit file's text, for a test that wants to see what was written.
pub fn audit_text(directory: &Path) -> String {
    std::fs::read_to_string(directory.join("audit.jsonl")).unwrap_or_default()
}

pub async fn wait_for<F: FnMut() -> bool>(what: &str, mut condition: F) {
    for _ in 0..200 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}");
}

/// A path that fits a row's pattern, for the table-driven checks.
pub fn sample_path(pattern: &str) -> String {
    pattern
        .replace("/receivers/{id}", "/receivers/living-room")
        .replace("/operations/{id}", "/operations/1")
        .replace("/tokens/{id}", "/tokens/t-0000ffff")
}

/// Every record in the audit log, newest first, as the Operator reads it.
pub async fn audit_records(fixture: &Fixture) -> Vec<denon_avr_application::AuditRecord> {
    use denon_avr_application::{AuditQuery, OperatorAdmin, Principal};
    let handle = fixture.service.handle(Principal::Operator).unwrap();
    handle
        .audit(AuditQuery::new(500))
        .await
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| entry.record)
        .collect()
}

// ---- A client for the event streams ----

/// Undoes HTTP/1.1 chunked transfer coding, which is how the server writes a body
/// of no known length.
#[derive(Default)]
struct Chunked {
    buffer: Vec<u8>,
    remaining: usize,
    state: ChunkState,
}

#[derive(Default, PartialEq)]
enum ChunkState {
    #[default]
    Size,
    Data,
    DataEnd,
    Done,
}

impl Chunked {
    fn push(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.buffer.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            match self.state {
                ChunkState::Size => {
                    let Some(end) = self.buffer.windows(2).position(|pair| pair == b"\r\n") else {
                        break;
                    };
                    let line = String::from_utf8_lossy(&self.buffer[..end]).into_owned();
                    self.buffer.drain(..end + 2);
                    let size = line.split(';').next().unwrap_or("").trim();
                    let size = usize::from_str_radix(size, 16)
                        .unwrap_or_else(|_| panic!("a chunk size, not {size:?}"));
                    if size == 0 {
                        self.state = ChunkState::Done;
                    } else {
                        self.remaining = size;
                        self.state = ChunkState::Data;
                    }
                }
                ChunkState::Data => {
                    let take = self.remaining.min(self.buffer.len());
                    if take == 0 {
                        break;
                    }
                    out.extend(self.buffer.drain(..take));
                    self.remaining -= take;
                    if self.remaining == 0 {
                        self.state = ChunkState::DataEnd;
                    }
                }
                ChunkState::DataEnd => {
                    if self.buffer.len() < 2 {
                        break;
                    }
                    self.buffer.drain(..2);
                    self.state = ChunkState::Size;
                }
                ChunkState::Done => break,
            }
        }
        out
    }

    fn done(&self) -> bool {
        self.state == ChunkState::Done
    }
}

/// What a stream's client saw when it looked.
#[derive(Debug)]
pub enum Next {
    Event(denon_avr_api_contract::events::Event),
    /// The stream is over: its body ended or the server closed the connection.
    Closed,
    /// Nothing arrived in the time given.
    Quiet,
}

/// A connection reading `text/event-stream`, with the bytes it has read.
pub struct Stream {
    connection: UnixStream,
    chunks: Chunked,
    parser: denon_avr_api_contract::events::Parser,
    queue: std::collections::VecDeque<denon_avr_api_contract::events::Event>,
    /// The body as the server wrote it, with the chunk coding removed.
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
    closed: bool,
}

impl Stream {
    /// Open a stream, or return the error response that was sent instead.
    pub async fn open(socket: &Path, path: &str, token: &str) -> Result<Self, Reply> {
        let mut connection = UnixStream::connect(socket).await.expect("connect");
        let head = format!(
            "GET {path} HTTP/1.1\r\nHost: dar\r\nAuthorization: Bearer {token}\r\n\
             Accept: text/event-stream\r\n\r\n"
        );
        connection
            .write_all(head.as_bytes())
            .await
            .expect("write the request");
        let mut raw = Vec::new();
        let split = loop {
            if let Some(split) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
                break split;
            }
            let mut buffer = [0u8; 4096];
            let read = tokio::time::timeout(Duration::from_secs(5), connection.read(&mut buffer))
                .await
                .expect("a response head within five seconds")
                .expect("read");
            assert!(read > 0, "the connection closed before a response head");
            raw.extend_from_slice(&buffer[..read]);
        };
        let rest = raw.split_off(split + 4);
        let reply = parse_reply(&raw).expect("a response head");
        if reply.status != 200 {
            let length: usize = reply
                .header("content-length")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            let mut body = rest;
            while body.len() < length {
                let mut buffer = [0u8; 4096];
                let read = connection.read(&mut buffer).await.expect("read");
                if read == 0 {
                    break;
                }
                body.extend_from_slice(&buffer[..read]);
            }
            return Err(Reply { body, ..reply });
        }
        let mut stream = Self {
            connection,
            chunks: Chunked::default(),
            parser: denon_avr_api_contract::events::Parser::new(),
            queue: Default::default(),
            body: Vec::new(),
            headers: reply.headers,
            closed: false,
        };
        stream.feed(&rest);
        Ok(stream)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn feed(&mut self, bytes: &[u8]) {
        let decoded = self.chunks.push(bytes);
        self.body.extend_from_slice(&decoded);
        let events = self
            .parser
            .push(&decoded)
            .expect("the server writes events");
        self.queue.extend(events);
        if self.chunks.done() {
            self.closed = true;
        }
    }

    /// The next event, waiting up to `within` for it.
    pub async fn next(&mut self, within: Duration) -> Next {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            if let Some(event) = self.queue.pop_front() {
                return Next::Event(event);
            }
            if self.closed {
                return Next::Closed;
            }
            let mut buffer = [0u8; 4096];
            match tokio::time::timeout_at(deadline, self.connection.read(&mut buffer)).await {
                Err(_) => return Next::Quiet,
                Ok(Ok(0)) | Ok(Err(_)) => self.closed = true,
                Ok(Ok(read)) => self.feed(&buffer[..read]),
            }
        }
    }

    /// The next event, which the test expects to be there soon.
    pub async fn event(&mut self) -> denon_avr_api_contract::events::Event {
        match self.next(Duration::from_secs(5)).await {
            Next::Event(event) => event,
            other => panic!("expected an event, saw {other:?}"),
        }
    }

    /// The next event, which must be `name`.
    pub async fn event_named(
        &mut self,
        name: denon_avr_api_contract::events::EventName,
    ) -> denon_avr_api_contract::events::Event {
        let event = self.event().await;
        assert_eq!(event.name, name, "data: {}", event.data);
        event
    }

    /// Read until the stream ends, and return its last event.
    pub async fn last_event(&mut self) -> Option<denon_avr_api_contract::events::Event> {
        let mut last = None;
        loop {
            match self.next(Duration::from_secs(5)).await {
                Next::Event(event) => last = Some(event),
                Next::Closed => return last,
                Next::Quiet => panic!("the stream did not end"),
            }
        }
    }

    /// Whether the stream is still open and silent for `within`.
    pub async fn stays_quiet(&mut self, within: Duration) -> bool {
        matches!(self.next(within).await, Next::Quiet)
    }

    /// Lines of the body that are comments, which is how a keep-alive is written.
    pub fn comments(&self) -> usize {
        String::from_utf8_lossy(&self.body)
            .lines()
            .filter(|line| line.starts_with(':'))
            .count()
    }
}

/// The reason in an `end` event.
pub fn end_reason(event: &denon_avr_api_contract::events::Event) -> String {
    assert_eq!(event.name, denon_avr_api_contract::events::EventName::End);
    event.decode::<Value>().unwrap()["reason"]
        .as_str()
        .unwrap()
        .to_owned()
}
