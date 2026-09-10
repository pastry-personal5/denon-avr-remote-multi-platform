//! Synchronous TCP AVR adapter.

use denon_avr_application::ports::{
    ControlGateway, OperationError, OperationErrorKind, SessionEvent, StatusGateway,
};
use denon_avr_domain::{
    AudioContextSnapshot, Confidence, MainZoneControl, MainZoneEvent, MainZoneField, MainZoneValue,
    Observed, RawObservation, SurroundMode,
};
use denon_avr_domain::{FieldError, FieldErrorKind};
use denon_avr_protocol::avr::{
    encode_control, get_command_family, parse_main_zone_event, parse_main_zone_response,
    query_command, response_matches, AvrCommand,
};
use std::collections::VecDeque;
use std::io::{BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

pub struct SyncAvrClient {
    stream: BufReader<TcpStream>,
    response_timeout: Duration,
    events: VecDeque<SessionEvent>,
}
impl SyncAvrClient {
    pub fn connect(
        host: &str,
        connect_timeout: Duration,
        response_timeout: Duration,
    ) -> Result<Self, OperationError> {
        let address = (host, 23)
            .to_socket_addrs()
            .map_err(|e| connection_error("resolving receiver", e))?
            .next()
            .ok_or_else(|| {
                OperationError::new(
                    OperationErrorKind::Connection,
                    "resolving receiver",
                    "host has no address",
                )
            })?;
        Self::connect_addr(address, connect_timeout, response_timeout)
    }
    pub fn connect_addr(
        address: SocketAddr,
        connect_timeout: Duration,
        response_timeout: Duration,
    ) -> Result<Self, OperationError> {
        let stream = TcpStream::connect_timeout(&address, connect_timeout)
            .map_err(|e| connection_error("connecting to receiver", e))?;
        stream
            .set_read_timeout(Some(response_timeout))
            .map_err(|e| connection_error("setting read timeout", e))?;
        stream
            .set_write_timeout(Some(response_timeout))
            .map_err(|e| connection_error("setting write timeout", e))?;
        Ok(Self {
            stream: BufReader::new(stream),
            response_timeout,
            events: VecDeque::new(),
        })
    }
    pub fn request_raw(&mut self, command: &str) -> Result<String, OperationError> {
        let command = AvrCommand::new(command.to_owned()).map_err(|e| {
            OperationError::new(
                OperationErrorKind::Malformed,
                "validating AVR command",
                e.to_string(),
            )
        })?;
        self.stream
            .get_mut()
            .write_all(&command.as_bytes())
            .map_err(|e| connection_error("writing AVR query", e))?;
        let family = get_command_family(command.as_str());
        let deadline = Instant::now() + self.response_timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(OperationError::new(
                    OperationErrorKind::Timeout,
                    "reading AVR response",
                    "response deadline expired",
                ));
            }
            let line = self.read_line(Some(remaining))?;
            if response_matches(family, &line) {
                return Ok(line);
            }
            self.events.push_back(session_event(&line));
        }
    }

    /// Collects only the documented, read-only diagnostic queries. Each query
    /// is independent so one unsupported or malformed response cannot erase
    /// the other observations.
    pub fn query_audio_context(&mut self) -> AudioContextSnapshot {
        let mut snapshot = AudioContextSnapshot::default();
        for command in ["SI?", "SD?", "DC?", "MS?", "CV?"] {
            let started = Instant::now();
            let result = self.request_raw(command);
            let elapsed_millis = started.elapsed().as_millis();
            let raw = match &result {
                Ok(response) => RawObservation {
                    response: Some(response.clone()),
                    error: None,
                    elapsed_millis,
                },
                Err(error) => RawObservation {
                    response: None,
                    error: Some(error.to_string()),
                    elapsed_millis,
                },
            };
            snapshot.record_raw(command, raw);
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
                        if let Ok(mode) = SurroundMode::new(value) {
                            snapshot.current_mode =
                                Observed::known(mode, "MS?", Confidence::Observed);
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
                ("DC?", Err(error)) => {
                    snapshot.digital_mode = Observed::unavailable(
                        FieldError {
                            kind: FieldErrorKind::Unavailable,
                            message: error.to_string(),
                        },
                        "DC?",
                    );
                }
                (_, Err(error)) => {
                    let field = FieldError {
                        kind: match error.kind {
                            OperationErrorKind::Timeout => FieldErrorKind::Timeout,
                            OperationErrorKind::Disconnected => FieldErrorKind::Disconnected,
                            _ => FieldErrorKind::Unavailable,
                        },
                        message: error.to_string(),
                    };
                    match command {
                        "SI?" => {
                            snapshot.input_selection = Observed::unavailable(field.clone(), command)
                        }
                        "SD?" => {
                            snapshot.digital_mode = Observed::unavailable(field.clone(), command)
                        }
                        "MS?" => {
                            snapshot.current_mode = Observed::unavailable(field.clone(), command)
                        }
                        "CV?" => snapshot.channel_volume = Observed::unavailable(field, command),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        snapshot
    }

    /// Dispatch one already-validated command without waiting for a reply.
    /// This is intended for the one-shot CLI; callers must obtain confirmation
    /// with a later status query.
    pub fn send_once(&mut self, command: &AvrCommand) -> Result<(), OperationError> {
        self.stream
            .get_mut()
            .write_all(&command.as_bytes())
            .map_err(|e| connection_error("dispatching AVR command", e))
    }
    fn read_line(&mut self, timeout: Option<Duration>) -> Result<String, OperationError> {
        self.stream
            .get_mut()
            .set_read_timeout(timeout)
            .map_err(|e| connection_error("setting read timeout", e))?;
        let mut line = Vec::with_capacity(135);
        loop {
            let mut byte = [0u8; 1];
            self.stream.read_exact(&mut byte).map_err(|e| {
                let kind = if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) {
                    OperationErrorKind::Timeout
                } else {
                    OperationErrorKind::Disconnected
                };
                OperationError::new(kind, "reading AVR line", e.to_string())
            })?;
            if byte[0] == b'\r' {
                return String::from_utf8(line).map_err(|e| {
                    OperationError::new(
                        OperationErrorKind::Malformed,
                        "reading AVR line",
                        e.to_string(),
                    )
                });
            }
            if line.len() >= 135 {
                return Err(OperationError::new(
                    OperationErrorKind::Malformed,
                    "reading AVR line",
                    "AVR response exceeded 135 bytes",
                ));
            }
            line.push(byte[0]);
        }
    }
}
impl StatusGateway for SyncAvrClient {
    fn query_field(&mut self, field: MainZoneField) -> Result<MainZoneValue, OperationError> {
        let response = self.request_raw(query_command(field).as_str())?;
        parse_main_zone_response(field, &response).map_err(|e| {
            let kind = if matches!(e, denon_avr_protocol::avr::AvrProtocolError::Unavailable(_)) {
                OperationErrorKind::Unavailable
            } else {
                OperationErrorKind::Malformed
            };
            OperationError::new(kind, "parsing AVR response", e.to_string())
        })
    }
    fn connection_generation(&self) -> u64 {
        0
    }
    fn next_event(&mut self, timeout: Option<Duration>) -> Result<SessionEvent, OperationError> {
        if let Some(event) = self.events.pop_front() {
            return Ok(event);
        }
        let line = self.read_line(timeout)?;
        Ok(match parse_main_zone_event(&line) {
            MainZoneEvent::Unknown(line) => SessionEvent::MainZone(MainZoneEvent::Unknown(line)),
            event => SessionEvent::MainZone(event),
        })
    }

    fn query_audio_context(&mut self) -> AudioContextSnapshot {
        SyncAvrClient::query_audio_context(self)
    }
}

impl ControlGateway for SyncAvrClient {
    fn execute_once(&mut self, control: MainZoneControl) -> Result<(), OperationError> {
        let command = encode_control(&control).map_err(|error| {
            OperationError::new(
                OperationErrorKind::Malformed,
                "encoding control command",
                error.to_string(),
            )
        })?;
        self.request_raw(command.as_str()).map(|_| ())
    }
}
fn session_event(line: &str) -> SessionEvent {
    match parse_main_zone_event(line) {
        MainZoneEvent::Unknown(line) => SessionEvent::MainZone(MainZoneEvent::Unknown(line)),
        event => SessionEvent::MainZone(event),
    }
}

fn connection_error(context: &'static str, error: std::io::Error) -> OperationError {
    OperationError::new(OperationErrorKind::Connection, context, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_unknown_sync_unsolicited_lines() {
        assert_eq!(
            session_event("X9UNKNOWN"),
            SessionEvent::MainZone(MainZoneEvent::Unknown("X9UNKNOWN".into()))
        );
    }
}
