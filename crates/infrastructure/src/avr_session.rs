//! Persistent asynchronous AVR TCP sessions.

use denon_avr_application::ports::{OperationError, OperationErrorKind};
use denon_avr_protocol::avr::AvrCommand;
use denon_avr_protocol::{get_command_family, response_matches};
use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

const DEFAULT_MAX_LINE_LENGTH: usize = 135;

#[derive(Debug, Clone)]
pub struct AvrSessionConfig {
    pub connect_timeout: Duration,
    pub write_timeout: Duration,
    pub response_timeout: Duration,
    pub reconnect_attempts: usize,
    pub reconnect_delay: Duration,
    /// Continue reconnecting until the owning Phase 5 session is explicitly
    /// closed.  Kept opt-in while the legacy controller is still present.
    pub reconnect_indefinitely: bool,
    /// The X3800H requires at least 50 ms between all transmissions.
    pub transmission_interval: Duration,
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
            reconnect_indefinitely: false,
            transmission_interval: Duration::from_millis(50),
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
    /// Generation identifies the connection that failed.  Consumers can
    /// discard a delayed disconnect notification after a newer connection is
    /// already established.
    Disconnected {
        generation: u64,
        message: String,
    },
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
    task_handle: JoinHandle<()>,
}

impl Drop for AvrSession {
    fn drop(&mut self) {
        // A canonical actor may be cancelled while the transport is in
        // reconnect backoff. Dropping the handle must still terminate the
        // socket owner; otherwise an orphaned retry task survives shutdown.
        self.task_handle.abort();
    }
}

impl AvrSession {
    pub async fn connect_addr(
        address: SocketAddr,
        config: AvrSessionConfig,
    ) -> Result<Self, AvrSessionError> {
        let reader = connect_socket(address, &config).await?;
        let (requests, request_rx) = mpsc::channel(16);
        let (event_tx, events) = mpsc::channel(32);
        let generation = Arc::new(AtomicU64::new(0));
        let task_handle = tokio::spawn(run_session(
            address,
            config,
            reader,
            request_rx,
            event_tx,
            Arc::clone(&generation),
        ));
        Ok(Self {
            requests,
            events,
            generation,
            task_handle,
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

    pub async fn next_event(&mut self) -> Option<AvrSessionEvent> {
        self.events.recv().await
    }

    /// Monotonically increasing connection generation. It changes after every
    /// successful automatic reconnect and lets snapshot operations detect
    /// state that may have been missed during the reconnect.
    pub fn connection_generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
}

impl AvrSession {
    /// Stop the transport task and wait for it to end. Closing twice is harmless.
    pub async fn close(&mut self) -> Result<(), OperationError> {
        let task_handle = std::mem::replace(&mut self.task_handle, tokio::task::spawn(async {}));
        task_handle.abort();
        match task_handle.await {
            Ok(()) => Ok(()),
            Err(join_error) => {
                // Treat normal cancellation as successful closure.
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
    }
}

async fn connect_socket(
    address: SocketAddr,
    config: &AvrSessionConfig,
) -> Result<BufReader<TcpStream>, AvrSessionError> {
    debug!(%address, "opening AVR TCP connection");
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
) {
    info!(%address, "AVR session transport started");
    if !publish_event(&events, AvrSessionEvent::Connected).await {
        return;
    }
    let mut last_transmission = None;
    let mut power_on_quiet_until = None;
    loop {
        let mut request =
            match next_request(&mut reader, &mut requests, &events, config.max_line_length).await {
                Ok(Some(request)) => request,
                Ok(None) => return,
                Err(message) => {
                    warn!(%message, "AVR transport failed while waiting for a request");
                    if !publish_event(
                        &events,
                        AvrSessionEvent::Disconnected {
                            generation: generation.load(Ordering::Acquire),
                            message,
                        },
                    )
                    .await
                    {
                        return;
                    }
                    match reconnect(address, &config, &events, &generation).await {
                        Ok(new_reader) => {
                            info!(%address, "AVR transport reconnected");
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
        let now = tokio::time::Instant::now();
        let mut due = power_on_quiet_until.unwrap_or(now);
        if let Some(previous) = last_transmission {
            due = due.max(previous + config.transmission_interval);
        }
        if due > now {
            tokio::time::sleep_until(due).await;
        }
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
        // A completed, timed-out, or failed write attempt occupies the wire
        // schedule. We must not immediately issue another command after an
        // ambiguous transport boundary.
        last_transmission = Some(tokio::time::Instant::now());
        if request.command.as_str() == "PWON" {
            power_on_quiet_until = Some(
                last_transmission.expect("transmission timestamp was just recorded")
                    + Duration::from_secs(1),
            );
        }
        if let Err(error) = write_result {
            let message = error.to_string();
            warn!(command = request.command.as_str(), %message, "AVR command write failed");
            if let Some(response) = request.response.take() {
                let _ = response.send(Err(error.clone()));
            }
            if let Some(dispatched) = request.dispatched.take() {
                let _ = dispatched.send(Err(error));
            }
            if !publish_event(
                &events,
                AvrSessionEvent::Disconnected {
                    generation: generation.load(Ordering::Acquire),
                    message,
                },
            )
            .await
            {
                return;
            }
            match reconnect(address, &config, &events, &generation).await {
                Ok(new_reader) => {
                    info!(%address, "AVR transport reconnected after write failure");
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
        )
        .await;
        match result {
            Ok(response) => {
                let _ = response_sender.send(Ok(response));
            }
            Err(error @ AvrSessionError::Timeout(_))
            | Err(error @ AvrSessionError::Disconnected(_))
            | Err(error @ AvrSessionError::MalformedFrame(_)) => {
                let message = error.to_string();
                warn!(%message, "AVR response stream failed");
                let _ = response_sender.send(Err(error));
                if !publish_event(
                    &events,
                    AvrSessionEvent::Disconnected {
                        generation: generation.load(Ordering::Acquire),
                        message,
                    },
                )
                .await
                {
                    return;
                }
                match reconnect(address, &config, &events, &generation).await {
                    Ok(new_reader) => {
                        info!(%address, "AVR transport reconnected after response failure");
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
) -> Result<Option<Request>, String> {
    loop {
        tokio::select! {
            request = requests.recv() => return Ok(request),
            result = read_frame(reader, max_line_length) => {
                match result {
                    Ok(line) => {
                        let text = frame_text(&line, max_line_length)
                            .map_err(|error| error.to_string())?;
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

async fn publish_event(events: &mpsc::Sender<AvrSessionEvent>, event: AvrSessionEvent) -> bool {
    // The AVR can emit an unsolicited burst (notably a series of MV lines
    // while its volume is being changed). A line the consumer cannot take yet
    // is dropped, because notification delivery must not apply backpressure to
    // the protocol reader or block subsequent commands; the canonical session
    // reconciles by querying. Lifecycle notifications remain lossless and ordered.
    if matches!(event, AvrSessionEvent::Line(_)) {
        return match events.try_send(event) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => true,
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        };
    }
    events.send(event).await.is_ok()
}

async fn reconnect(
    address: SocketAddr,
    config: &AvrSessionConfig,
    events: &mpsc::Sender<AvrSessionEvent>,
    generation: &Arc<AtomicU64>,
) -> Result<BufReader<TcpStream>, AvrSessionError> {
    let mut last_error = None;
    let mut attempt = 0usize;
    loop {
        if !config.reconnect_indefinitely && attempt >= config.reconnect_attempts {
            break;
        }
        if attempt > 0 {
            // Bounded exponential backoff. The Phase 5 owner opts into the
            // indefinite form; legacy callers retain their finite policy.
            let multiplier = 1u32 << attempt.saturating_sub(1).min(7);
            tokio::time::sleep(config.reconnect_delay.saturating_mul(multiplier)).await;
        }
        match connect_socket(address, config).await {
            Ok(reader) => {
                info!(%address, attempt, "AVR reconnect succeeded");
                return Ok(reader);
            }
            Err(error) => {
                attempt = attempt.saturating_add(1);
                last_error = Some(error.to_string());
                warn!(%address, attempt, error = %error, "AVR reconnect attempt failed");
                if !publish_event(
                    events,
                    AvrSessionEvent::Disconnected {
                        generation: generation.load(Ordering::Acquire),
                        message: last_error.clone().unwrap(),
                    },
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
    async fn globally_paces_each_transmission() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut command = Vec::new();
            reader.read_until(b'\r', &mut command).await.unwrap();
            assert_eq!(command, b"MV?\r");
            reader.get_mut().write_all(b"MV80\r").await.unwrap();
            command.clear();
            let first = tokio::time::Instant::now();
            reader.read_until(b'\r', &mut command).await.unwrap();
            let elapsed = first.elapsed();
            assert_eq!(command, b"MU?\r");
            assert!(
                elapsed >= Duration::from_millis(45),
                "second command arrived after {elapsed:?}"
            );
            reader.get_mut().write_all(b"MUOFF\r").await.unwrap();
        });
        let session = AvrSession::connect_addr(address, AvrSessionConfig::default())
            .await
            .unwrap();
        assert_eq!(session.request("MV?").await.unwrap(), "MV80");
        assert_eq!(session.request("MU?").await.unwrap(), "MUOFF");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn enforces_denon_power_on_quiet_period() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut command = Vec::new();
            reader.read_until(b'\r', &mut command).await.unwrap();
            assert_eq!(command, b"PWON\r");
            command.clear();
            let started = tokio::time::Instant::now();
            reader.read_until(b'\r', &mut command).await.unwrap();
            assert!(started.elapsed() >= Duration::from_millis(950));
            assert_eq!(command, b"MV?\r");
            reader.get_mut().write_all(b"MV80\r").await.unwrap();
        });
        let session = AvrSession::connect_addr(
            address,
            AvrSessionConfig {
                transmission_interval: Duration::from_millis(1),
                ..AvrSessionConfig::default()
            },
        )
        .await
        .unwrap();
        session.dispatch("PWON").await.unwrap();
        assert_eq!(session.request("MV?").await.unwrap(), "MV80");
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
            for _ in 0..64 {
                reader.get_mut().write_all(b"MVMAX 615\r").await.unwrap();
            }
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
        let session = AvrSession::connect_addr(address, AvrSessionConfig::default())
            .await
            .unwrap();

        session.dispatch("SIGAME").await.unwrap();

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
