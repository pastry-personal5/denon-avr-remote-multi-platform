//! Diagnostic-only Quick Select name discovery probe.
//!
//! This sends a small, fixed set of read-only HTTP GET requests used by
//! established Denon integrations. It neither sends Telnet commands nor posts
//! AppCommand XML, so it cannot recall, save, rename, or otherwise change a
//! receiver setting. Its purpose is to capture model- and firmware-specific
//! evidence for (or against) a Quick Select name field.

use std::env;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CANDIDATE_PATHS: [&str; 4] = [
    "/goform/Deviceinfo.xml",
    "/goform/formMainZone_MainZoneXml.xml",
    "/goform/formMainZone_MainZoneXmlStatus.xml",
    "/goform/formMainZone_MainZoneXmlStatusLite.xml",
];
const QUICK_SELECT_NAME_PATH: &str = "/goform/AppCommand.xml";
const QUICK_SELECT_NAME_REQUEST_XML: &str =
    "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<tx>\n<cmd id=\"1\">GetQuickSelectName</cmd>\n</tx>";
const MAX_RESPONSE_BYTES: u64 = 1_048_576;

fn main() -> io::Result<()> {
    let (endpoint, timeout, model, firmware, scenario) = arguments();
    let target = parse_http_endpoint(&endpoint)?;
    for path in CANDIDATE_PATHS {
        let result = fetch(&target, "GET", path, None, timeout)?;
        println!(
            "probe=quick-select-name scenario={scenario} model={model} firmware={firmware} timestamp={} endpoint={endpoint}{path} elapsed_ms={}",
            epoch_seconds(),
            result.elapsed.as_millis(),
        );
        println!("request_method=GET");
        println!("http_status={}", result.status);
        println!("raw_response_xml={}", one_line(&result.body));
    }
    let result = fetch(
        &target,
        "POST",
        QUICK_SELECT_NAME_PATH,
        Some(QUICK_SELECT_NAME_REQUEST_XML),
        timeout,
    )?;
    println!(
        "probe=quick-select-name scenario={scenario} model={model} firmware={firmware} timestamp={} endpoint={endpoint}{QUICK_SELECT_NAME_PATH} elapsed_ms={}",
        epoch_seconds(),
        result.elapsed.as_millis(),
    );
    println!("request_method=POST");
    println!("request_xml={QUICK_SELECT_NAME_REQUEST_XML}");
    println!("http_status={}", result.status);
    println!("raw_response_xml={}", one_line(&result.body));
    Ok(())
}

struct HttpEndpoint {
    host: String,
    port: u16,
}

struct FetchResult {
    status: String,
    body: String,
    elapsed: Duration,
}

fn fetch(
    target: &HttpEndpoint,
    method: &str,
    path: &str,
    body: Option<&str>,
    timeout: Duration,
) -> io::Result<FetchResult> {
    let address = (target.host.as_str(), target.port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "endpoint host has no address"))?;
    let started = Instant::now();
    let mut stream = TcpStream::connect_timeout(&address, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let request = match body {
        Some(body) => format!(
            "{method} {path} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: text/xml; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            target.host,
            target.port,
            body.len(),
        ),
        None => format!(
            "{method} {path} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\r\n",
            target.host, target.port
        ),
    };
    stream.write_all(request.as_bytes())?;
    stream.flush()?;
    let mut response = Vec::new();
    stream.take(MAX_RESPONSE_BYTES).read_to_end(&mut response)?;
    let raw_response = String::from_utf8_lossy(&response);
    let (status, body) = split_http_response(&raw_response);
    Ok(FetchResult {
        status: status.to_owned(),
        body: body.to_owned(),
        elapsed: started.elapsed(),
    })
}

fn parse_http_endpoint(value: &str) -> io::Result<HttpEndpoint> {
    let authority = value.strip_prefix("http://").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "endpoint must use an explicit http:// scheme",
        )
    })?;
    if authority.is_empty() || authority.contains('/') || authority.contains('#') {
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
    Ok(HttpEndpoint { host, port })
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
        .unwrap_or_else(|| usage("HTTP endpoint is required"));
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
    eprintln!("{message}\nusage: quick-select-name-probe HTTP-ENDPOINT TIMEOUT-MS MODEL FIRMWARE [SCENARIO]\nexample endpoint: http://192.0.2.10:80\nThis probe makes four fixed HTTP GET requests and one AppCommand GetQuickSelectName POST; it never changes receiver state.");
    std::process::exit(2)
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn one_line(value: &str) -> String {
    value.replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_explicit_http_endpoint_with_or_without_port() {
        let default_port = parse_http_endpoint("http://192.0.2.10").unwrap();
        assert_eq!(default_port.host, "192.0.2.10");
        assert_eq!(default_port.port, 80);
        let explicit_port = parse_http_endpoint("http://receiver.local:8080").unwrap();
        assert_eq!(explicit_port.port, 8080);
    }

    #[test]
    fn rejects_paths_and_non_http_schemes_before_network_access() {
        assert!(parse_http_endpoint("https://192.0.2.10").is_err());
        assert!(parse_http_endpoint("http://192.0.2.10/goform/Deviceinfo.xml").is_err());
    }

    #[test]
    fn probe_paths_and_app_command_request_are_fixed_and_read_only() {
        assert_eq!(CANDIDATE_PATHS.len(), 4);
        assert!(CANDIDATE_PATHS
            .iter()
            .all(|path| path.starts_with("/goform/")));
        assert!(CANDIDATE_PATHS.iter().all(|path| path.ends_with(".xml")));
        assert_eq!(QUICK_SELECT_NAME_PATH, "/goform/AppCommand.xml");
        assert!(QUICK_SELECT_NAME_REQUEST_XML.contains("GetQuickSelectName"));
        assert!(!QUICK_SELECT_NAME_REQUEST_XML.contains("SetQuickSelect"));
    }
}
