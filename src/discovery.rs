//! SSDP discovery for Denon/HEOS receivers.

use if_addrs::{get_if_addrs, IfAddr};
use std::collections::BTreeMap;
use std::fmt;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

pub const SSDP_PORT: u16 = 1900;
pub const DENON_DOCUMENTED_SSDP_PORT: u16 = 1800;
pub const SSDP_TARGET: &str = "urn:schemas-denon-com:device:ACT-Denon:1";
pub const AIOS_SSDP_TARGET: &str = "urn:schemas-denon-com:device:AiosDevice:1";
pub const DEFAULT_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);

const SSDP_MULTICAST_ADDRESS: [u8; 4] = [239, 255, 255, 250];
const SSDP_MULTICAST_TTL: u32 = 3;
const SSDP_MX_SECONDS: u8 = 3;
const SSDP_RETRIES: usize = 1;
const DESCRIPTION_PORT: u16 = 60006;
const DESCRIPTION_PATH: &str = "/upnp/desc/aios_device/aios_device.xml";
const SCAN_WORKERS: usize = 32;
const SCAN_CONNECT_TIMEOUT: Duration = Duration::from_millis(100);
const SCAN_READ_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredReceiver {
    pub address: SocketAddr,
    pub location: Option<String>,
    pub server: Option<String>,
    pub model: Option<String>,
    pub search_target: Option<String>,
    pub unique_service_name: Option<String>,
}

impl DiscoveredReceiver {
    fn key(&self) -> (String, Option<String>) {
        (self.address.ip().to_string(), self.location.clone())
    }

    fn is_denon_family(&self) -> bool {
        [
            self.location.as_deref(),
            self.server.as_deref(),
            self.model.as_deref(),
            self.search_target.as_deref(),
            self.unique_service_name.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|value| {
            let value = value.to_ascii_lowercase();
            ["denon", "marantz", "heos", "aios"]
                .iter()
                .any(|marker| value.contains(marker))
        })
    }
}

#[derive(Debug)]
pub enum DiscoveryError {
    Io(std::io::Error),
    InvalidResponse(String),
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "SSDP discovery failed: {error}"),
            Self::InvalidResponse(message) => write!(f, "invalid SSDP response: {message}"),
        }
    }
}

impl std::error::Error for DiscoveryError {}

impl From<std::io::Error> for DiscoveryError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn parse_response(
    packet: &[u8],
    source: SocketAddr,
) -> Result<DiscoveredReceiver, DiscoveryError> {
    let text = std::str::from_utf8(packet)
        .map_err(|_| DiscoveryError::InvalidResponse("response is not UTF-8".to_owned()))?;
    let mut headers = BTreeMap::new();
    for line in text.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }
    if headers.is_empty() {
        return Err(DiscoveryError::InvalidResponse("no headers".to_owned()));
    }
    Ok(DiscoveredReceiver {
        address: source,
        location: headers.get("location").cloned(),
        server: headers.get("server").cloned(),
        model: headers
            .get("model")
            .cloned()
            .or_else(|| headers.get("friendlyname").cloned()),
        search_target: headers.get("st").cloned(),
        unique_service_name: headers.get("usn").cloned(),
    })
}

pub fn discover(timeout: Duration) -> Result<Vec<DiscoveredReceiver>, DiscoveryError> {
    let interface_ips = local_private_ipv4_addresses()?;
    let mut sockets = Vec::new();
    for interface_ip in &interface_ips {
        let socket = UdpSocket::bind((*interface_ip, 0))?;
        socket.set_multicast_ttl_v4(SSDP_MULTICAST_TTL)?;
        socket.set_read_timeout(Some(Duration::from_millis(50)))?;
        send_searches(&socket)?;
        sockets.push(socket);
    }
    if sockets.is_empty() {
        return Err(DiscoveryError::InvalidResponse(
            "no private IPv4 network interface is available".to_owned(),
        ));
    }

    let deadline = Instant::now() + timeout;
    let retry_at = Instant::now() + timeout / 2;
    let mut retries_sent = 0;
    let mut packet = [0u8; 4096];
    let mut receivers = BTreeMap::new();
    while Instant::now() < deadline {
        if retries_sent < SSDP_RETRIES && Instant::now() >= retry_at {
            for socket in &sockets {
                send_searches(socket)?;
            }
            retries_sent += 1;
        }
        for socket in &sockets {
            match socket.recv_from(&mut packet) {
                Ok((length, source)) => {
                    if let Ok(receiver) = parse_response(&packet[..length], source) {
                        if !receiver.is_denon_family() {
                            continue;
                        }
                        receivers.entry(receiver.key()).or_insert(receiver);
                    }
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    if receivers.is_empty() {
        return discover_by_description_scan(&interface_ips);
    }
    Ok(receivers.into_values().map(enrich_receiver).collect())
}

fn send_searches(socket: &UdpSocket) -> Result<(), DiscoveryError> {
    for port in [SSDP_PORT, DENON_DOCUMENTED_SSDP_PORT] {
        let destination = SocketAddr::from((SSDP_MULTICAST_ADDRESS, port));
        for target in [SSDP_TARGET, AIOS_SSDP_TARGET, "ssdp:all"] {
            socket.send_to(discovery_request(port, target).as_bytes(), destination)?;
        }
    }
    Ok(())
}

fn discovery_request(port: u16, target: &str) -> String {
    format!(
        "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:{port}\r\nMAN: \"ssdp:discover\"\r\nMX: {SSDP_MX_SECONDS}\r\nST: {target}\r\nUSER-AGENT: Rust/std UPnP/1.1 denon-avr-remote/0.1\r\n\r\n"
    )
}

fn discover_by_description_scan(
    interface_ips: &[Ipv4Addr],
) -> Result<Vec<DiscoveredReceiver>, DiscoveryError> {
    let mut candidates = Vec::new();
    for local_ip in interface_ips {
        let octets = local_ip.octets();
        for host in 1..=254 {
            let candidate = Ipv4Addr::new(octets[0], octets[1], octets[2], host);
            if candidate != *local_ip && !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
    }
    let candidates = Arc::new(candidates);
    let next_host = Arc::new(AtomicUsize::new(0));
    let (sender, receiver) = mpsc::channel();
    let mut workers = Vec::with_capacity(SCAN_WORKERS);
    for _ in 0..SCAN_WORKERS {
        let next_host = Arc::clone(&next_host);
        let candidates = Arc::clone(&candidates);
        let sender = sender.clone();
        workers.push(thread::spawn(move || loop {
            let index = next_host.fetch_add(1, Ordering::Relaxed);
            let Some(candidate) = candidates.get(index).copied() else {
                break;
            };
            if let Some(receiver) = probe_description(candidate) {
                let _ = sender.send(receiver);
            }
        }));
    }
    drop(sender);

    let mut receivers = BTreeMap::new();
    for discovered in receiver {
        receivers.entry(discovered.key()).or_insert(discovered);
    }
    for worker in workers {
        worker.join().map_err(|_| {
            DiscoveryError::InvalidResponse("description scan worker failed".to_owned())
        })?;
    }
    Ok(receivers.into_values().collect())
}

fn local_private_ipv4_addresses() -> Result<Vec<Ipv4Addr>, DiscoveryError> {
    let mut addresses = get_if_addrs()?
        .into_iter()
        .filter_map(|interface| match interface.addr {
            IfAddr::V4(address) if address.ip.is_private() && !address.ip.is_loopback() => {
                Some(address.ip)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    addresses.sort_unstable();
    addresses.dedup();
    Ok(addresses)
}

fn probe_description(ip: Ipv4Addr) -> Option<DiscoveredReceiver> {
    let address = SocketAddr::from((ip, DESCRIPTION_PORT));
    let mut stream = TcpStream::connect_timeout(&address, SCAN_CONNECT_TIMEOUT).ok()?;
    stream.set_read_timeout(Some(SCAN_READ_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(SCAN_READ_TIMEOUT)).ok()?;
    let request = format!(
        "GET {DESCRIPTION_PATH} HTTP/1.0\r\nHost: {ip}:{DESCRIPTION_PORT}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = Vec::new();
    let mut buffer = [0u8; 4096];
    while response.len() < 65_536 {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(length) => response.extend_from_slice(&buffer[..length]),
            Err(error)
                if !response.is_empty()
                    && matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
            {
                break;
            }
            Err(_) => return None,
        }
    }
    let text = std::str::from_utf8(&response).ok()?;
    let (_, body) = text
        .split_once("\r\n\r\n")
        .or_else(|| text.split_once("\n\n"))?;
    parse_description(body, address)
}

fn enrich_receiver(mut receiver: DiscoveredReceiver) -> DiscoveredReceiver {
    let IpAddr::V4(ip) = receiver.address.ip() else {
        return receiver;
    };
    let Some(description) = probe_description(ip) else {
        return receiver;
    };
    receiver.model = description.model.or(receiver.model);
    receiver.location = description.location.or(receiver.location);
    receiver.search_target = description.search_target.or(receiver.search_target);
    receiver.unique_service_name = description
        .unique_service_name
        .or(receiver.unique_service_name);
    receiver
}

fn parse_description(xml: &str, address: SocketAddr) -> Option<DiscoveredReceiver> {
    let manufacturer = xml_tag(xml, "manufacturer")?;
    if !matches!(
        manufacturer.to_ascii_lowercase().as_str(),
        "denon" | "marantz"
    ) {
        return None;
    }
    let model = xml_tag(xml, "modelName").or_else(|| xml_tag(xml, "friendlyName"));
    Some(DiscoveredReceiver {
        address,
        location: Some(format!(
            "http://{}:{DESCRIPTION_PORT}{DESCRIPTION_PATH}",
            address.ip()
        )),
        server: Some(format!("{manufacturer} UPnP")),
        model,
        search_target: xml_tag(xml, "deviceType"),
        unique_service_name: xml_tag(xml, "UDN"),
    })
}

fn xml_tag(xml: &str, tag: &str) -> Option<String> {
    let start_marker = format!("<{tag}>");
    let end_marker = format!("</{tag}>");
    let start = xml.find(&start_marker)? + start_marker.len();
    let end = xml[start..].find(&end_marker)? + start;
    Some(xml[start..end].trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_headers_are_case_insensitive() {
        let response =
            b"HTTP/1.1 200 OK\r\nLOCATION: http://192.0.2.4:8080/\r\nServer: Denon\r\n\r\n";
        let receiver = parse_response(response, "192.0.2.4:1800".parse().unwrap()).unwrap();
        assert_eq!(receiver.location.as_deref(), Some("http://192.0.2.4:8080/"));
        assert_eq!(receiver.server.as_deref(), Some("Denon"));
    }

    #[test]
    fn duplicate_key_uses_address_and_location() {
        let receiver = DiscoveredReceiver {
            address: "192.0.2.4:1800".parse().unwrap(),
            location: Some("http://192.0.2.4/".to_owned()),
            server: None,
            model: None,
            search_target: Some(SSDP_TARGET.to_owned()),
            unique_service_name: None,
        };
        assert_eq!(
            receiver.key(),
            ("192.0.2.4".to_owned(), receiver.location.clone())
        );
    }

    #[test]
    fn probes_standard_and_denon_documented_ssdp_ports() {
        assert!(discovery_request(SSDP_PORT, SSDP_TARGET).contains("HOST: 239.255.255.250:1900"));
        assert!(discovery_request(DENON_DOCUMENTED_SSDP_PORT, SSDP_TARGET)
            .contains("HOST: 239.255.255.250:1800"));
        assert!(discovery_request(SSDP_PORT, SSDP_TARGET).contains(SSDP_TARGET));
        assert!(discovery_request(SSDP_PORT, AIOS_SSDP_TARGET).contains(AIOS_SSDP_TARGET));
        assert!(discovery_request(SSDP_PORT, "ssdp:all").contains("ST: ssdp:all"));
        assert!(discovery_request(SSDP_PORT, SSDP_TARGET).contains("MX: 3"));
        assert!(discovery_request(SSDP_PORT, SSDP_TARGET).contains("UPnP/1.1"));
    }

    #[test]
    fn broad_search_results_are_filtered_to_denon_family() {
        let receiver = parse_response(
            b"HTTP/1.1 200 OK\r\nST: upnp:rootdevice\r\nUSN: uuid:123\r\nLOCATION: http://192.0.2.4/upnp/desc/aios_device.xml\r\n\r\n",
            "192.0.2.4:1900".parse().unwrap(),
        )
        .unwrap();
        assert!(receiver.is_denon_family());

        let unrelated = parse_response(
            b"HTTP/1.1 200 OK\r\nST: upnp:rootdevice\r\nSERVER: Generic Printer\r\n\r\n",
            "192.0.2.5:1900".parse().unwrap(),
        )
        .unwrap();
        assert!(!unrelated.is_denon_family());
    }

    #[test]
    fn parses_denon_aios_description() {
        let receiver = parse_description(
            "<root><device><deviceType>urn:schemas-denon-com:device:AiosDevice:1</deviceType><friendlyName>Living Room</friendlyName><manufacturer>Denon</manufacturer><modelName>Denon AVC-X3800H</modelName><UDN>uuid:test</UDN></device></root>",
            "192.0.2.8:60006".parse().unwrap(),
        )
        .unwrap();
        assert_eq!(receiver.model.as_deref(), Some("Denon AVC-X3800H"));
        assert_eq!(receiver.unique_service_name.as_deref(), Some("uuid:test"));
        assert!(receiver.is_denon_family());
    }

    #[test]
    fn rejects_non_denon_description() {
        assert!(parse_description(
            "<root><device><manufacturer>Other</manufacturer><modelName>Printer</modelName></device></root>",
            "192.0.2.9:60006".parse().unwrap(),
        )
        .is_none());
    }
}
