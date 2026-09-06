//! Main-zone AVR status snapshots and synchronous TCP transport.

use crate::avr::{parse_line, AvrCommand, AvrLine};
use crate::response::{
    parse_input, parse_mute, parse_power, parse_surround, parse_volume, response_matches,
};
use crate::session::{AvrSession, AvrSessionError};
use std::fmt;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusField {
    pub value: Option<String>,
    pub error: Option<String>,
}

impl StatusField {
    fn value(value: impl Into<String>) -> Self {
        Self {
            value: Some(value.into()),
            error: None,
        }
    }

    fn unavailable(error: impl Into<String>) -> Self {
        Self {
            value: None,
            error: Some(error.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainZoneStatus {
    pub power: StatusField,
    pub input: StatusField,
    pub volume: StatusField,
    pub mute: StatusField,
    pub surround_mode: StatusField,
}

#[derive(Debug)]
pub enum StatusError {
    Connection(String),
    Transport(String),
}

impl fmt::Display for StatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connection(message) => write!(f, "connection failed: {message}"),
            Self::Transport(message) => write!(f, "transport failed: {message}"),
        }
    }
}

impl std::error::Error for StatusError {}

pub trait AvrTransport {
    fn query(&mut self, command: &str) -> Result<String, String>;
}

/// Legacy direct TCP transport. Prefer [`crate::ApplicationService`].
#[deprecated(
    note = "use ApplicationService for receiver operations; this transport remains for compatibility"
)]
pub struct TcpAvrTransport {
    stream: TcpStream,
}

#[allow(deprecated)]
impl TcpAvrTransport {
    pub fn connect(
        host: &str,
        connect_timeout: Duration,
        read_timeout: Duration,
    ) -> Result<Self, StatusError> {
        let address = (host, 23)
            .to_socket_addrs()
            .map_err(|error| StatusError::Connection(error.to_string()))?
            .next()
            .ok_or_else(|| StatusError::Connection("host has no address".to_owned()))?;
        Self::connect_addr(address, connect_timeout, read_timeout)
    }

    pub fn connect_addr(
        address: SocketAddr,
        connect_timeout: Duration,
        read_timeout: Duration,
    ) -> Result<Self, StatusError> {
        let stream = TcpStream::connect_timeout(&address, connect_timeout)
            .map_err(|error| StatusError::Connection(error.to_string()))?;
        stream
            .set_read_timeout(Some(read_timeout))
            .map_err(|error| StatusError::Transport(error.to_string()))?;
        stream
            .set_write_timeout(Some(read_timeout))
            .map_err(|error| StatusError::Transport(error.to_string()))?;
        Ok(Self { stream })
    }
}

#[allow(deprecated)]
impl AvrTransport for TcpAvrTransport {
    fn query(&mut self, command: &str) -> Result<String, String> {
        let command = AvrCommand::new(command).map_err(|error| error.to_string())?;
        self.stream
            .write_all(&command.as_bytes())
            .map_err(|e| e.to_string())?;
        let family =
            &command.as_str()[..command.as_str().find('?').unwrap_or(command.as_str().len())];
        loop {
            let mut line = Vec::new();
            let mut byte = [0u8; 1];
            loop {
                self.stream
                    .read_exact(&mut byte)
                    .map_err(|e| e.to_string())?;
                if byte[0] == b'\r' {
                    break;
                }
                line.push(byte[0]);
                if line.len() > 135 {
                    return Err("AVR response exceeded 135 bytes".to_owned());
                }
            }
            match parse_line(&line) {
                Ok(AvrLine::Raw(value)) if response_matches(family, &value) => return Ok(value),
                Ok(AvrLine::Raw(_)) => continue,
                Ok(_) => return Err("unexpected AVR line".to_owned()),
                Err(error) => return Err(error.to_string()),
            }
        }
    }
}

/// Legacy synchronous query API. Prefer [`crate::ApplicationService`].
#[deprecated(
    note = "use ApplicationService::query_main_zone; direct transports are a compatibility API"
)]
pub fn query_main_zone<T: AvrTransport>(transport: &mut T) -> MainZoneStatus {
    let power = query_field(transport, "PW?", parse_power);
    let input = query_field(transport, "SI?", parse_input);
    let volume = query_field(transport, "MV?", parse_volume);
    let mute = query_field(transport, "MU?", parse_mute);
    let surround_mode = query_field(transport, "MS?", parse_surround_mode);
    MainZoneStatus {
        power,
        input,
        volume,
        mute,
        surround_mode,
    }
}

/// Query the same bounded main-zone snapshot through a persistent async session.
pub async fn query_main_zone_async(session: &AvrSession) -> MainZoneStatus {
    let generation = session.connection_generation();
    let status = query_main_zone_async_once(session).await;
    if session.connection_generation() != generation {
        return query_main_zone_async_once(session).await;
    }
    status
}

async fn query_main_zone_async_once(session: &AvrSession) -> MainZoneStatus {
    let power = query_field_async(session, "PW?", parse_power).await;
    let input = query_field_async(session, "SI?", parse_input).await;
    let volume = query_field_async(session, "MV?", parse_volume).await;
    let mute = query_field_async(session, "MU?", parse_mute).await;
    let surround_mode = query_field_async(session, "MS?", parse_surround_mode).await;
    MainZoneStatus {
        power,
        input,
        volume,
        mute,
        surround_mode,
    }
}

fn query_field<T: AvrTransport>(
    transport: &mut T,
    command: &str,
    parser: fn(&str) -> Result<String, String>,
) -> StatusField {
    match transport
        .query(command)
        .and_then(|response| parser(&response))
    {
        Ok(value) => StatusField::value(value),
        Err(error) => StatusField::unavailable(error),
    }
}

async fn query_field_async(
    session: &AvrSession,
    command: &str,
    parser: fn(&str) -> Result<String, String>,
) -> StatusField {
    let result = match session.request_query(command).await {
        Ok(response) => parser(&response).map_err(AvrSessionError::UnexpectedResponse),
        Err(error) => Err(error),
    };
    match result {
        Ok(value) => StatusField::value(value),
        Err(error) => StatusField::unavailable(error.to_string()),
    }
}

fn parse_surround_mode(response: &str) -> Result<String, String> {
    parse_surround(response)
}

pub fn render(status: &MainZoneStatus) -> String {
    fn line(name: &str, field: &StatusField) -> String {
        match (&field.value, &field.error) {
            (Some(value), _) => format!("{name}: {value}"),
            (None, Some(error)) => format!("{name}: unavailable ({error})"),
            (None, None) => format!("{name}: unavailable"),
        }
    }
    [
        line("Power", &status.power),
        line("Input", &status.input),
        line("Volume", &status.volume),
        line("Mute", &status.mute),
        line("Surround mode", &status.surround_mode),
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        responses: Vec<Result<String, String>>,
        commands: Vec<String>,
    }
    impl AvrTransport for Fake {
        fn query(&mut self, command: &str) -> Result<String, String> {
            self.commands.push(command.to_owned());
            self.responses.remove(0)
        }
    }

    #[test]
    #[allow(deprecated)]
    fn queries_fields_in_order_and_preserves_partial_failures() {
        let mut fake = Fake {
            responses: vec![
                Ok("PWON".into()),
                Ok("SICD".into()),
                Err("timed out".into()),
                Ok("MUOFF".into()),
                Ok("MSSTEREO".into()),
            ],
            commands: Vec::new(),
        };
        let status = query_main_zone(&mut fake);
        assert_eq!(fake.commands, ["PW?", "SI?", "MV?", "MU?", "MS?"]);
        assert_eq!(status.power.value.as_deref(), Some("on"));
        assert_eq!(status.input.value.as_deref(), Some("CD"));
        assert_eq!(status.volume.value, None);
        assert_eq!(status.mute.value.as_deref(), Some("off"));
        assert_eq!(status.surround_mode.value.as_deref(), Some("STEREO"));
    }

    #[test]
    fn renders_unavailable_field() {
        let status = MainZoneStatus {
            power: StatusField::unavailable("timed out"),
            input: StatusField::value("CD"),
            volume: StatusField::value("code 80 (0.0 dB)"),
            mute: StatusField::value("off"),
            surround_mode: StatusField::value("STEREO"),
        };
        assert!(render(&status).contains("Power: unavailable (timed out)"));
    }

    #[test]
    fn decodes_reference_volume_codes_for_humans() {
        assert_eq!(parse_volume("MV80").unwrap(), "code 80 (0.0 dB)");
        assert_eq!(parse_volume("MV795").unwrap(), "code 795 (-0.5 dB)");
        assert!(parse_volume("MV999").is_err());
    }

    #[test]
    #[allow(deprecated)]
    fn tcp_transport_frames_queries_and_ignores_unrelated_events() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = Vec::new();
            reader.read_until(b'\r', &mut line).unwrap();
            assert_eq!(line, b"MV?\r");
            let mut stream = stream;
            stream.write_all(b"SIHDMI1\rMVMAX 615\rMV805\r").unwrap();
        });

        let mut transport =
            TcpAvrTransport::connect_addr(address, Duration::from_secs(1), Duration::from_secs(1))
                .unwrap();
        assert_eq!(transport.query("MV?").unwrap(), "MV805");
        server.join().unwrap();
    }

    #[test]
    fn does_not_correlate_mv_notifications_as_volume_status() {
        assert!(response_matches("MV", "MV805"));
        assert!(!response_matches("MV", "MVMAX 615"));
    }

    #[tokio::test]
    async fn async_status_query_uses_persistent_session() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            for (expected, response) in [
                (b"PW?\r".as_slice(), b"PWON\r".as_slice()),
                (b"SI?\r".as_slice(), b"SICD\r".as_slice()),
                (b"MV?\r".as_slice(), b"MV80\r".as_slice()),
                (b"MU?\r".as_slice(), b"MUOFF\r".as_slice()),
                (b"MS?\r".as_slice(), b"MSSTEREO\r".as_slice()),
            ] {
                let mut command = Vec::new();
                reader.read_until(b'\r', &mut command).await.unwrap();
                assert_eq!(command, expected);
                reader.get_mut().write_all(response).await.unwrap();
            }
        });
        let session = crate::session::AvrSession::connect_addr(
            address,
            crate::session::AvrSessionConfig::default(),
        )
        .await
        .unwrap();
        let status = query_main_zone_async(&session).await;
        assert_eq!(status.power.value.as_deref(), Some("on"));
        assert_eq!(status.input.value.as_deref(), Some("CD"));
        assert_eq!(status.volume.value.as_deref(), Some("code 80 (0.0 dB)"));
        assert_eq!(status.mute.value.as_deref(), Some("off"));
        assert_eq!(status.surround_mode.value.as_deref(), Some("STEREO"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn async_status_query_refreshes_a_query_after_reconnect() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut command = Vec::new();
            reader.read_until(b'\r', &mut command).await.unwrap();
            assert_eq!(command, b"PW?\r");
            drop(reader);

            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            for _ in 0..2 {
                for (expected, response) in [
                    (b"PW?\r".as_slice(), b"PWON\r".as_slice()),
                    (b"SI?\r".as_slice(), b"SICD\r".as_slice()),
                    (b"MV?\r".as_slice(), b"MV80\r".as_slice()),
                    (b"MU?\r".as_slice(), b"MUOFF\r".as_slice()),
                    (b"MS?\r".as_slice(), b"MSSTEREO\r".as_slice()),
                ] {
                    let mut command = Vec::new();
                    reader.read_until(b'\r', &mut command).await.unwrap();
                    assert_eq!(command, expected);
                    reader.get_mut().write_all(response).await.unwrap();
                }
            }
        });
        let config = crate::session::AvrSessionConfig {
            reconnect_delay: Duration::ZERO,
            ..crate::session::AvrSessionConfig::default()
        };
        let session = crate::session::AvrSession::connect_addr(address, config)
            .await
            .unwrap();
        let status = query_main_zone_async(&session).await;
        assert_eq!(status.power.value.as_deref(), Some("on"));
        assert_eq!(status.input.value.as_deref(), Some("CD"));
        server.await.unwrap();
    }
}
