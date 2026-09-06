//! Main-zone AVR status snapshots and synchronous TCP transport.

use crate::avr::{parse_line, AvrCommand, AvrLine};
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

pub struct TcpAvrTransport {
    stream: TcpStream,
}

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
                Ok(AvrLine::Raw(value)) if value.starts_with(family) => return Ok(value),
                Ok(AvrLine::Raw(_)) => continue,
                Ok(_) => return Err("unexpected AVR line".to_owned()),
                Err(error) => return Err(error.to_string()),
            }
        }
    }
}

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

fn parse_power(response: &str) -> Result<String, String> {
    match response {
        "PWON" | "ZMON" => Ok("on".to_owned()),
        "PWSTANDBY" | "ZMSTANDBY" => Ok("standby".to_owned()),
        value if value.starts_with("PW") || value.starts_with("ZM") => {
            Err(format!("unsupported power response {value}"))
        }
        value => Err(format!("unexpected power response {value}")),
    }
}

fn parse_input(response: &str) -> Result<String, String> {
    response
        .strip_prefix("SI")
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("unexpected input response {response}"))
}

fn parse_surround_mode(response: &str) -> Result<String, String> {
    response
        .strip_prefix("MS")
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("unexpected surround response {response}"))
}

fn parse_volume(response: &str) -> Result<String, String> {
    let code = response
        .strip_prefix("MV")
        .ok_or_else(|| format!("unexpected volume response {response}"))?;
    if code.is_empty() {
        return Err("volume response is empty".to_owned());
    }
    let half_step = code.ends_with('5');
    let base_text = if half_step {
        &code[..code.len() - 1]
    } else {
        code
    };
    let base = base_text
        .parse::<i16>()
        .map_err(|_| format!("invalid volume code {code}"))?;
    if !(0..=98).contains(&base)
        || (half_step && code.len() != 3)
        || (!half_step && code.len() != 2)
    {
        return Err(format!("invalid volume code {code}"));
    }
    let db_tenths = (base - 80) * 10 + if half_step { 5 } else { 0 };
    let db = db_tenths as f32 / 10.0;
    Ok(format!("code {code} ({db:.1} dB)"))
}

fn parse_mute(response: &str) -> Result<String, String> {
    match response {
        "MUON" => Ok("on".to_owned()),
        "MUOFF" => Ok("off".to_owned()),
        value => Err(format!("unexpected mute response {value}")),
    }
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
            stream.write_all(b"SIHDMI1\rMV805\r").unwrap();
        });

        let mut transport =
            TcpAvrTransport::connect_addr(address, Duration::from_secs(1), Duration::from_secs(1))
                .unwrap();
        assert_eq!(transport.query("MV?").unwrap(), "MV805");
        server.join().unwrap();
    }
}
