//! Bounded synchronous HTTP adapter for Denon AppCommand requests.

use denon_avr_domain::ReceiverEndpoint;
use denon_avr_protocol::app_command::{
    AppCommandRequest, AppCommandResponse, APP_COMMAND_0300_PATH,
};
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};
use tracing::{debug, warn};

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawHttpResponse {
    pub status_line: String,
    pub status_code: u16,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppCommandExchange {
    pub http: RawHttpResponse,
    pub response: AppCommandResponse,
}

pub struct AppCommandHttpClient {
    endpoint: ReceiverEndpoint,
    timeout: Duration,
}

impl AppCommandHttpClient {
    pub fn new(endpoint: ReceiverEndpoint, timeout: Duration) -> io::Result<Self> {
        if endpoint.host.contains(['\r', '\n']) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "receiver host contains a line break",
            ));
        }
        if endpoint.host.trim().is_empty() || endpoint.port == 0 || timeout.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "receiver endpoint and timeout must be non-empty",
            ));
        }
        Ok(Self { endpoint, timeout })
    }

    pub fn execute(&self, request: &AppCommandRequest) -> io::Result<RawHttpResponse> {
        let body = request
            .to_xml()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        self.post_xml(APP_COMMAND_0300_PATH, &body)
    }

    pub fn execute_xml_at(&self, path: &str, body: &str) -> io::Result<RawHttpResponse> {
        if !path.starts_with('/') || path.contains(['\r', '\n']) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "HTTP request path is invalid",
            ));
        }
        self.post_xml(path, body)
    }

    pub fn get_audio_information(&self) -> io::Result<AppCommandExchange> {
        self.execute_and_parse(&AppCommandRequest::audio_information())
    }

    pub fn get_input_signal(&self) -> io::Result<AppCommandExchange> {
        self.execute_and_parse(&AppCommandRequest::input_signal())
    }

    pub fn get_active_speaker(&self) -> io::Result<AppCommandExchange> {
        self.execute_and_parse(&AppCommandRequest::active_speaker())
    }

    pub fn get_audio_info(&self) -> io::Result<AppCommandExchange> {
        self.execute_and_parse(&AppCommandRequest::audio_info())
    }

    pub fn execute_and_parse(&self, request: &AppCommandRequest) -> io::Result<AppCommandExchange> {
        let http = self.execute(request)?;
        parse_app_command_exchange(http)
    }

    pub fn execute_and_parse_at(
        &self,
        path: &str,
        request: &AppCommandRequest,
    ) -> io::Result<AppCommandExchange> {
        let body = request
            .to_xml()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let http = self.execute_xml_at(path, &body)?;
        parse_app_command_exchange(http)
    }

    fn post_xml(&self, path: &str, body: &str) -> io::Result<RawHttpResponse> {
        debug!(host = %self.endpoint.host, port = self.endpoint.port, %path, "AppCommand HTTP request");
        let mut stream = self.connect()?;
        let request = build_request(&self.endpoint.host, self.endpoint.port, path, body);
        stream.write_all(request.as_bytes())?;
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let count = stream.read(&mut chunk)?;
            if count == 0 {
                break;
            }
            if bytes
                .len()
                .checked_add(count)
                .is_none_or(|length| length > MAX_RESPONSE_BYTES)
            {
                warn!(host = %self.endpoint.host, %path, "AppCommand HTTP response exceeded size limit");
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "HTTP response exceeds bounded size",
                ));
            }
            bytes.extend_from_slice(&chunk[..count]);
            if let Some(expected_length) = expected_response_length(&bytes)? {
                if expected_length > MAX_RESPONSE_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "HTTP response exceeds bounded size",
                    ));
                }
                if bytes.len() >= expected_length {
                    bytes.truncate(expected_length);
                    break;
                }
            }
        }
        let response = parse_http_response(&bytes)?;
        debug!(host = %self.endpoint.host, %path, status = response.status_code, bytes = bytes.len(), "AppCommand HTTP response received");
        Ok(response)
    }

    fn connect(&self) -> io::Result<TcpStream> {
        let addresses = (&*self.endpoint.host, self.endpoint.port)
            .to_socket_addrs()?
            .collect::<Vec<_>>();
        if addresses.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "host has no address",
            ));
        }
        let deadline = Instant::now() + self.timeout;
        let mut last_error = None;
        for address in addresses {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match TcpStream::connect_timeout(&address, remaining) {
                Ok(stream) => {
                    stream.set_read_timeout(Some(self.timeout))?;
                    stream.set_write_timeout(Some(self.timeout))?;
                    return Ok(stream);
                }
                Err(error) => {
                    debug!(%address, error = %error, "AppCommand HTTP connection attempt failed");
                    last_error = Some(error)
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "receiver connection deadline expired",
            )
        }))
    }
}

fn parse_app_command_exchange(http: RawHttpResponse) -> io::Result<AppCommandExchange> {
    if !(200..300).contains(&http.status_code) {
        return Err(io::Error::other(format!(
            "AppCommand request failed with {}",
            http.status_line
        )));
    }
    let response = denon_avr_protocol::app_command::parse_app_command_response(&http.body)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(AppCommandExchange { http, response })
}

fn build_request(host: &str, port: u16, path: &str, body: &str) -> String {
    let authority = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    format!(
        "POST {path} HTTP/1.1\r\nHost: {authority}\r\nAccept: */*\r\nContent-Type: text/xml; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nUser-Agent: curl/8.0\r\n\r\n{body}",
        body.len()
    )
}

fn parse_http_response(bytes: &[u8]) -> io::Result<RawHttpResponse> {
    let header_end = find_header_end(bytes).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "HTTP response has no header separator",
        )
    })?;
    let headers = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "HTTP headers are not UTF-8"))?;
    let encoded_body = &bytes[header_end + 4..];
    let status_line = headers.lines().next().unwrap_or_default().to_owned();
    if status_line.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "HTTP status is empty",
        ));
    }
    let protocol = status_line.split_whitespace().next().unwrap_or_default();
    if !matches!(protocol, "HTTP/1.0" | "HTTP/1.1") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "HTTP status line has an invalid protocol",
        ));
    }
    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "HTTP status is invalid"))?;
    if !(100..=599).contains(&status_code) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "HTTP status code is out of range",
        ));
    }
    let body = match body_framing(headers)? {
        BodyFraming::ContentLength(content_length) => {
            if encoded_body.len() != content_length {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "HTTP body length does not match Content-Length",
                ));
            }
            encoded_body.to_vec()
        }
        BodyFraming::Chunked => decode_chunked_body(encoded_body)?
            .map(|(body, _)| body)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "chunked HTTP body is incomplete",
                )
            })?,
        BodyFraming::CloseDelimited => encoded_body.to_vec(),
    };
    let body = String::from_utf8(body)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "HTTP body is not UTF-8"))?;
    Ok(RawHttpResponse {
        status_line,
        status_code,
        body,
    })
}

fn expected_response_length(bytes: &[u8]) -> io::Result<Option<usize>> {
    let Some(header_end) = find_header_end(bytes) else {
        return Ok(None);
    };
    let headers = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "HTTP headers are not UTF-8"))?;
    let body_start = header_end.checked_add(4).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "HTTP header length overflows")
    })?;
    match body_framing(headers)? {
        BodyFraming::ContentLength(length) => {
            body_start.checked_add(length).map(Some).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "HTTP body length overflows")
            })
        }
        BodyFraming::Chunked => decode_chunked_body(&bytes[header_end + 4..]).and_then(|decoded| {
            decoded
                .map(|(_, encoded_length)| {
                    body_start.checked_add(encoded_length).ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "chunked HTTP body length overflows",
                        )
                    })
                })
                .transpose()
        }),
        BodyFraming::CloseDelimited => Ok(None),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyFraming {
    ContentLength(usize),
    Chunked,
    CloseDelimited,
}

fn body_framing(headers: &str) -> io::Result<BodyFraming> {
    let mut content_length = None;
    let mut transfer_encoding = None;
    for line in headers.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("transfer-encoding")
            && transfer_encoding.replace(value.trim()).is_some()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP response contains duplicate Transfer-Encoding headers",
            ));
        }
        if name.eq_ignore_ascii_case("content-length") {
            let length = value.trim().parse::<usize>().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "Content-Length is invalid")
            })?;
            if content_length.replace(length).is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "HTTP response contains duplicate Content-Length headers",
                ));
            }
        }
    }
    let chunked = match transfer_encoding {
        Some(value) if value.eq_ignore_ascii_case("chunked") => true,
        Some(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP response uses an unsupported Transfer-Encoding",
            ));
        }
        None => false,
    };
    match (chunked, content_length) {
        (true, Some(_)) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "HTTP response has conflicting body framing",
        )),
        (true, None) => Ok(BodyFraming::Chunked),
        (false, Some(length)) => Ok(BodyFraming::ContentLength(length)),
        (false, None) => Ok(BodyFraming::CloseDelimited),
    }
}

fn decode_chunked_body(encoded: &[u8]) -> io::Result<Option<(Vec<u8>, usize)>> {
    let mut decoded = Vec::new();
    let mut cursor = 0;
    loop {
        let Some(line_length) = encoded[cursor..]
            .windows(2)
            .position(|window| window == b"\r\n")
        else {
            return Ok(None);
        };
        let line_end = cursor + line_length;
        let size_text = std::str::from_utf8(&encoded[cursor..line_end])
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "chunk size is not ASCII"))?;
        let size_text = size_text.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "chunk size is invalid"))?;
        cursor = line_end + 2;
        if size == 0 {
            if encoded.get(cursor..cursor + 2) == Some(b"\r\n") {
                return Ok(Some((decoded, cursor + 2)));
            }
            let Some(trailer_length) = encoded[cursor..]
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
            else {
                return Ok(None);
            };
            return Ok(Some((decoded, cursor + trailer_length + 4)));
        }
        let Some(data_end) = cursor.checked_add(size) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "chunk size overflows",
            ));
        };
        let Some(chunk_end) = data_end.checked_add(2) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "chunk size overflows",
            ));
        };
        if encoded.len() < chunk_end {
            return Ok(None);
        }
        if encoded.get(data_end..chunk_end) != Some(b"\r\n") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "chunk data has no CRLF terminator",
            ));
        }
        if decoded
            .len()
            .checked_add(size)
            .is_none_or(|length| length > MAX_RESPONSE_BYTES)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "decoded HTTP body exceeds bounded size",
            ));
        }
        decoded.extend_from_slice(&encoded[cursor..data_end]);
        cursor = chunk_end;
    }
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_bounded_read_only_request() {
        let request = build_request("receiver", 8080, APP_COMMAND_0300_PATH, "<tx/>");
        assert!(request.starts_with("POST /goform/AppCommand0300.xml HTTP/1.1\r\n"));
        assert!(request.contains("Host: receiver:8080\r\n"));
        assert!(request.contains("Accept: */*\r\n"));
        assert!(request.contains("Content-Type: text/xml; charset=utf-8\r\n"));
        assert!(request.contains("Content-Length: 5\r\n"));
        assert!(request.contains("Connection: close\r\n"));
    }

    #[test]
    fn brackets_ipv6_host_header() {
        let request = build_request("2001:db8::1", 8080, APP_COMMAND_0300_PATH, "<tx/>");
        assert!(request.contains("Host: [2001:db8::1]:8080\r\n"));
    }

    #[test]
    fn parses_status_and_body_without_network() {
        let response =
            parse_http_response(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n<p>ok</p>").unwrap();
        assert_eq!(response.status_code, 200);
        assert_eq!(response.body, "<p>ok</p>");
    }

    #[test]
    fn parses_live_http_1_0_close_delimited_shape() {
        let response = parse_http_response(
            b"HTTP/1.0 200 OK\r\nContent-Type: application/xml\r\n\r\n<rx></rx>",
        )
        .unwrap();
        assert_eq!(response.status_code, 200);
        assert_eq!(response.body, "<rx></rx>");
    }

    #[test]
    fn computes_complete_response_length() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n<p>ok</p>";
        assert_eq!(
            expected_response_length(response).unwrap(),
            Some(response.len())
        );
    }

    #[test]
    fn rejects_incomplete_content_length_body() {
        let incomplete = b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\nshort";
        assert_eq!(
            parse_http_response(incomplete).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn decodes_chunked_response_and_detects_its_end() {
        let response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n<rx>\r\n6\r\nok</rx\r\n1\r\n>\r\n0\r\n\r\nignored";
        let expected_length = response.len() - "ignored".len();
        assert_eq!(
            expected_response_length(response).unwrap(),
            Some(expected_length)
        );
        assert_eq!(
            parse_http_response(&response[..expected_length])
                .unwrap()
                .body,
            "<rx>ok</rx>"
        );
    }

    #[test]
    fn rejects_conflicting_content_length_and_chunked_headers() {
        let response =
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n";
        assert_eq!(
            expected_response_length(response).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn rejects_unsupported_transfer_encoding() {
        let response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\n\r\nbody";
        assert_eq!(
            expected_response_length(response).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn rejects_overflowing_content_length() {
        let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", usize::MAX);
        assert_eq!(
            expected_response_length(response.as_bytes())
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn parsed_exchange_requires_success_status() {
        let error = parse_app_command_exchange(RawHttpResponse {
            status_line: "HTTP/1.0 404 Not Found".to_owned(),
            status_code: 404,
            body: "<rx></rx>".to_owned(),
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
    }

    #[test]
    fn rejects_invalid_http_status() {
        let error = parse_http_response(b"not-http\r\n\r\nbody").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let error = parse_http_response(b"HTTP/2 200 OK\r\n\r\nbody").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
