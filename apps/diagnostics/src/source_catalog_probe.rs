//! Diagnostic-only source-catalog probe.
//!
//! It sends two *candidate read commands* in one AppCommand request and emits
//! the exact request plus raw HTTP response. It has no product integration and
//! contains no receiver mutation command. The supplied endpoint is explicit so
//! the resulting evidence can record the exact firmware route that answered.
use std::env;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const REQUEST_XML: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?><tx><cmd id=\"1\">GetRenameSource</cmd><cmd id=\"1\">GetDeletedSource</cmd></tx>";
const MAX_RESPONSE_BYTES: u64 = 1_048_576;

fn main() -> io::Result<()> {
    let (endpoint, timeout, model, firmware, scenario) = arguments();
    let target = parse_http_endpoint(&endpoint)?;
    let address = (target.host.as_str(), target.port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "endpoint host has no address"))?;
    let started = Instant::now();
    let mut stream = TcpStream::connect_timeout(&address, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: text/xml; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        target.path, target.host, target.port, REQUEST_XML.len(), REQUEST_XML
    );
    stream.write_all(request.as_bytes())?;
    stream.flush()?;
    let mut response = Vec::new();
    stream.take(MAX_RESPONSE_BYTES).read_to_end(&mut response)?;
    let raw_response = String::from_utf8_lossy(&response);
    let (status, body) = split_http_response(&raw_response);
    let timestamp = epoch_seconds();
    println!("probe=source-catalog scenario={scenario} model={model} firmware={firmware} timestamp={timestamp} endpoint={endpoint} elapsed_ms={}", started.elapsed().as_millis());
    println!("request_xml={REQUEST_XML}");
    println!("http_status={status}");
    println!("raw_response_xml={}", one_line(body));
    if !status.starts_with("HTTP/1.0 2") && !status.starts_with("HTTP/1.1 2") {
        std::process::exit(1);
    }
    Ok(())
}

struct HttpEndpoint {
    host: String,
    port: u16,
    path: String,
}

fn parse_http_endpoint(value: &str) -> io::Result<HttpEndpoint> {
    let remainder = value.strip_prefix("http://").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "endpoint must use explicit http:// scheme",
        )
    })?;
    let (authority, raw_path) = remainder.split_once('/').ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "endpoint must include an AppCommand path",
        )
    })?;
    if authority.is_empty() || raw_path.is_empty() || raw_path.contains('#') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid endpoint",
        ));
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => (
            host.to_owned(),
            port.parse().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "endpoint port is invalid")
            })?,
        ),
        _ => (authority.to_owned(), 80),
    };
    Ok(HttpEndpoint {
        host,
        port,
        path: format!("/{raw_path}"),
    })
}

fn split_http_response(value: &str) -> (&str, &str) {
    let (headers, body) = value.split_once("\r\n\r\n").unwrap_or((value, ""));
    (
        headers.lines().next().unwrap_or("invalid HTTP response"),
        body,
    )
}

fn arguments() -> (String, Duration, String, String, String) {
    let mut args = env::args().skip(1);
    let endpoint = args
        .next()
        .unwrap_or_else(|| usage("APP-COMMAND-ENDPOINT is required"));
    let timeout_ms = args
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| usage("TIMEOUT-MS must be a positive integer"));
    let model = args.next().unwrap_or_else(|| usage("MODEL is required"));
    let firmware = args.next().unwrap_or_else(|| usage("FIRMWARE is required"));
    let scenario = args.next().unwrap_or_else(|| "unspecified".into());
    if args.next().is_some() {
        usage("too many arguments");
    }
    (
        endpoint,
        Duration::from_millis(timeout_ms),
        model,
        firmware,
        scenario,
    )
}

fn usage(message: &str) -> ! {
    eprintln!("{message}\nusage: source-catalog-probe APP-COMMAND-ENDPOINT TIMEOUT-MS MODEL FIRMWARE [SCENARIO]\nexample endpoint: http://192.0.2.10:8080/goform/AppCommand.xml\nThis probe is read-only; record baseline, rename, visibility, reconnect, and restoration scenarios separately.");
    std::process::exit(2)
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn one_line(value: &str) -> String {
    value
        .replace('\r', "\\r")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_an_explicit_app_command_endpoint() {
        let endpoint = parse_http_endpoint("http://192.0.2.10:8080/goform/AppCommand.xml").unwrap();
        assert_eq!(endpoint.host, "192.0.2.10");
        assert_eq!(endpoint.port, 8080);
        assert_eq!(endpoint.path, "/goform/AppCommand.xml");
    }
    #[test]
    fn rejects_an_implicit_or_tls_endpoint() {
        assert!(parse_http_endpoint("192.0.2.10/goform/AppCommand.xml").is_err());
        assert!(parse_http_endpoint("https://192.0.2.10/goform/AppCommand.xml").is_err());
    }
    #[test]
    fn preserves_raw_xml_as_one_record_line() {
        assert_eq!(one_line("<a>\r\n</a>"), "<a>\\r\\n</a>");
    }
}
