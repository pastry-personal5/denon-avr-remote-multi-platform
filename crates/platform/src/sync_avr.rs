use denon_avr_core::protocol::{
    command_family, parse_event, parse_main_zone_response, query_command, response_matches,
};
use denon_avr_core::{
    ConnectionState, MainZoneEvent, MainZoneField, MainZoneValue, OperationError,
    OperationErrorKind, SessionEvent, StatusGateway,
};
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

pub struct SyncAvrClient {
    stream: BufReader<TcpStream>,
    response_timeout: Duration,
}

impl SyncAvrClient {
    pub fn connect(
        host: &str,
        connect_timeout: Duration,
        response_timeout: Duration,
    ) -> Result<Self, OperationError> {
        let address = (host, 23)
            .to_socket_addrs()
            .map_err(|error| connection_error("resolving receiver", error))?
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
            .map_err(|error| connection_error("connecting to receiver", error))?;
        stream
            .set_read_timeout(Some(response_timeout))
            .map_err(|error| connection_error("setting read timeout", error))?;
        stream
            .set_write_timeout(Some(response_timeout))
            .map_err(|error| connection_error("setting write timeout", error))?;
        Ok(Self {
            stream: BufReader::new(stream),
            response_timeout,
        })
    }

    fn request(&mut self, field: MainZoneField) -> Result<MainZoneValue, OperationError> {
        let command = query_command(field);
        self.stream
            .get_mut()
            .write_all(&command.as_bytes())
            .map_err(|error| connection_error("writing AVR query", error))?;
        let family = command_family(command.as_str());
        loop {
            let line = self.read_line(Some(self.response_timeout))?;
            if response_matches(family, &line) {
                return parse_main_zone_response(field, &line).map_err(|error| {
                    OperationError::new(
                        OperationErrorKind::Malformed,
                        "parsing AVR response",
                        error.to_string(),
                    )
                });
            }
        }
    }

    fn read_line(&mut self, timeout: Option<Duration>) -> Result<String, OperationError> {
        self.stream
            .get_mut()
            .set_read_timeout(timeout)
            .map_err(|error| connection_error("setting read timeout", error))?;
        let mut line = Vec::new();
        self.stream.read_until(b'\r', &mut line).map_err(|error| {
            let kind = if matches!(
                error.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) {
                OperationErrorKind::Timeout
            } else {
                OperationErrorKind::Disconnected
            };
            OperationError::new(kind, "reading AVR line", error.to_string())
        })?;
        if line.last() != Some(&b'\r') {
            return Err(OperationError::new(
                OperationErrorKind::Malformed,
                "reading AVR line",
                "line was not terminated by CR",
            ));
        }
        line.pop();
        String::from_utf8(line).map_err(|error| {
            OperationError::new(
                OperationErrorKind::Malformed,
                "reading AVR line",
                error.to_string(),
            )
        })
    }
}

impl StatusGateway for SyncAvrClient {
    fn query_field(&mut self, field: MainZoneField) -> Result<MainZoneValue, OperationError> {
        self.request(field)
    }

    fn connection_generation(&self) -> u64 {
        0
    }

    fn next_event(&mut self, timeout: Option<Duration>) -> Result<SessionEvent, OperationError> {
        let line = self.read_line(timeout)?;
        Ok(match parse_event(&line) {
            MainZoneEvent::Unknown(_) => SessionEvent::Connection(ConnectionState::Connected),
            event => SessionEvent::MainZone(event),
        })
    }
}

fn connection_error(context: &'static str, error: std::io::Error) -> OperationError {
    OperationError::new(OperationErrorKind::Connection, context, error.to_string())
}
