//! Persistent asynchronous AVR TCP sessions.

use denon_avr_core::protocol::{
    command_family, parse_event, parse_main_zone_response, query_command, response_matches,
};
use denon_avr_core::AvrCommand;
use denon_avr_core::{
    AsyncStatusGateway, BoxFuture, ConnectionState, MainZoneEvent, MainZoneField, MainZoneValue,
    OperationError, OperationErrorKind, SessionEvent,
};
use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{lookup_host, TcpStream};
use tokio::sync::{mpsc, oneshot};

const DEFAULT_MAX_LINE_LENGTH: usize = 135;

#[derive(Debug, Clone)]
pub struct AvrSessionConfig {
    pub connect_timeout: Duration,
    pub write_timeout: Duration,
    pub response_timeout: Duration,
    pub reconnect_attempts: usize,
    pub reconnect_delay: Duration,
    pub max_line_length: usize,
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
    response: oneshot::Sender<Result<String, AvrSessionError>>,
}

pub struct AvrSession {
    requests: mpsc::Sender<Request>,
    events: mpsc::Receiver<AvrSessionEvent>,
    generation: Arc<AtomicU64>,
}

impl AvrSession {
    pub async fn connect(host: &str, config: AvrSessionConfig) -> Result<Self, AvrSessionError> {
        let address = tokio::time::timeout(config.connect_timeout, lookup_host((host, 23)))
            .await
            .map_err(|_| AvrSessionError::Timeout("resolving receiver address".to_owned()))?
            .map_err(|error| AvrSessionError::Connection(error.to_string()))?
            .next()
            .ok_or_else(|| AvrSessionError::Connection("host has no address".to_owned()))?;
        Self::connect_addr(address, config).await
    }

    pub async fn connect_addr(
        address: SocketAddr,
        config: AvrSessionConfig,
    ) -> Result<Self, AvrSessionError> {
        let reader = connect_socket(address, &config).await?;
        let (requests, request_rx) = mpsc::channel(16);
        let (event_tx, events) = mpsc::channel(32);
        let generation = Arc::new(AtomicU64::new(0));
        tokio::spawn(run_session(
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
        })
    }

    pub async fn request(&self, command: impl Into<String>) -> Result<String, AvrSessionError> {
        let command = AvrCommand::new(command.into())
            .map_err(|error| AvrSessionError::InvalidCommand(error.to_string()))?;
        let (response, result) = oneshot::channel();
        self.requests
            .send(Request { command, response })
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
            Err(AvrSessionError::Disconnected(_)) | Err(AvrSessionError::Timeout(_)) => {
                self.request(command).await
            }
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
                OperationError::new(
                    OperationErrorKind::Malformed,
                    "parsing AVR response",
                    error.to_string(),
                )
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
                Some(AvrSessionEvent::Line(line)) => Ok(match parse_event(&line) {
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
) {
    let _ = events.send(AvrSessionEvent::Connected).await;
    loop {
        let request =
            match next_request(&mut reader, &mut requests, &events, config.max_line_length).await {
                Ok(Some(request)) => request,
                Ok(None) => return,
                Err(message) => {
                    let _ = events.send(AvrSessionEvent::Disconnected(message)).await;
                    match reconnect(address, &config, &events).await {
                        Ok(new_reader) => {
                            reader = new_reader;
                            generation.fetch_add(1, Ordering::AcqRel);
                            let _ = events.send(AvrSessionEvent::Reconnected).await;
                            continue;
                        }
                        Err(_) => return,
                    }
                }
            };
        let family = command_family(request.command.as_str());
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
            let _ = request.response.send(Err(error));
            let _ = events.send(AvrSessionEvent::Disconnected(message)).await;
            match reconnect(address, &config, &events).await {
                Ok(new_reader) => {
                    reader = new_reader;
                    generation.fetch_add(1, Ordering::AcqRel);
                    let _ = events.send(AvrSessionEvent::Reconnected).await;
                }
                Err(_) => return,
            }
            continue;
        }

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
                let _ = request.response.send(Ok(response));
            }
            Err(error @ AvrSessionError::Timeout(_))
            | Err(error @ AvrSessionError::Disconnected(_)) => {
                let message = error.to_string();
                let _ = request.response.send(Err(error));
                let _ = events.send(AvrSessionEvent::Disconnected(message)).await;
                match reconnect(address, &config, &events).await {
                    Ok(new_reader) => {
                        reader = new_reader;
                        generation.fetch_add(1, Ordering::AcqRel);
                        let _ = events.send(AvrSessionEvent::Reconnected).await;
                    }
                    Err(_) => return,
                }
            }
            Err(error) => {
                let _ = request.response.send(Err(error));
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
                        let _ = events.send(AvrSessionEvent::Line(text)).await;
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
    loop {
        let line = tokio::time::timeout(timeout, read_frame(reader, max_line_length))
            .await
            .map_err(|_| AvrSessionError::Timeout(format!("waiting for {family} response")))??;
        let text = frame_text(&line, max_line_length)?;
        if response_matches(family, &text) {
            return Ok(text);
        }
        events
            .send(AvrSessionEvent::Line(text))
            .await
            .map_err(|_| AvrSessionError::SessionStopped)?;
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
                let _ = events
                    .send(AvrSessionEvent::Disconnected(last_error.clone().unwrap()))
                    .await;
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
    fn keeps_numeric_query_suffixes_in_the_response_family() {
        assert_eq!(command_family("Z2?"), "Z2");
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
