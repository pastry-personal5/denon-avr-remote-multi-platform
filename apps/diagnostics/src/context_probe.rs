//! Diagnostic-only probe. The allowlist below is intentionally read-only.
use std::env;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use denon_avr_protocol::response_matches;

// Read-only Telnet queries. Extended families and sample-rate/EQ queries are
// diagnostic candidates; their values are not promoted by this tool.
const COMMANDS: [&str; 16] = [
    "SI?",
    "SD?",
    "DC?",
    "MS?",
    "CV?",
    "SYSDA ?",
    "OPINFINS ?",
    "OPINFASP ?",
    "SYSMI ?",
    "SSINFAISFSV ?",
    // EQ read-only candidates. Quick Select recall is execute-only
    // and is intentionally excluded from this diagnostic probe.
    "PSMULTEQ: ?",
    "PSDYNEQ ?",
    "PSREFLEV ?",
    "PSDYNVOL ?",
    "PSLFC ?",
    "PSDIRAC ?",
];

fn main() -> io::Result<()> {
    let mut args = env::args().skip(1);
    let host = args.next().unwrap_or_else(|| {
        eprintln!("usage: x3800h-context-probe HOST TIMEOUT-MS MODEL FIRMWARE");
        std::process::exit(2)
    });
    let timeout = args
        .next()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            eprintln!("TIMEOUT-MS must be a positive integer");
            std::process::exit(2)
        });
    let model = args.next().unwrap_or_else(|| {
        eprintln!("MODEL is required");
        std::process::exit(2)
    });
    let firmware = args.next().unwrap_or_else(|| {
        eprintln!("FIRMWARE is required");
        std::process::exit(2)
    });
    if args.next().is_some() {
        eprintln!("too many arguments; usage: x3800h-context-probe HOST TIMEOUT-MS MODEL FIRMWARE");
        std::process::exit(2);
    }
    let address = (&*host, 23)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "host has no address"))?;
    let mut stream = connect(&address, timeout)?;
    let mut failed = false;
    for (index, command) in COMMANDS.into_iter().enumerate() {
        let started = Instant::now();
        stream.write_all(command.as_bytes())?;
        stream.write_all(b"\r")?;
        let mut bytes = Vec::new();
        let deadline = started + Duration::from_millis(timeout);
        let result = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break Err("timeout");
            }
            stream.set_read_timeout(Some(remaining))?;
            let mut byte = [0u8; 1];
            match stream.read_exact(&mut byte) {
                Ok(()) if byte[0] == b'\r' => {
                    let response = String::from_utf8_lossy(&bytes).into_owned();
                    if response_is_match(command, &response) {
                        break Ok(response);
                    }
                    println!("receiver={host} model={model} firmware={firmware} command=UNSOLICITED timestamp={} elapsed_ms={} status=ignored response={response}", epoch_seconds(), started.elapsed().as_millis());
                    bytes.clear();
                }
                Ok(()) => {
                    if bytes.len() >= 135 {
                        break Err("line-too-long");
                    }
                    bytes.push(byte[0]);
                }
                Err(e) => {
                    break Err(if e.kind() == io::ErrorKind::TimedOut {
                        "timeout"
                    } else {
                        "read-error"
                    })
                }
            }
        };
        let timestamp = epoch_seconds();
        let command_failed = result.is_err();
        match result {
            Ok(response) => println!("receiver={host} model={model} firmware={firmware} timestamp={timestamp} command={command} elapsed_ms={} status=ok response={response}", started.elapsed().as_millis()),
            Err(status) => {
                // DC? is an optional diagnostic family. AVC-X3800H units may
                // leave it unanswered; retain the evidence and reconnect, but
                // do not fail an otherwise successful read-only probe.
                let optional = is_optional_command(command);
                if !optional {
                    failed = true;
                }
                let outcome = if optional { "unavailable" } else { status };
                println!("receiver={host} model={model} firmware={firmware} timestamp={timestamp} command={command} elapsed_ms={} status={outcome}", started.elapsed().as_millis());
            }
        }
        // A timed-out query can leave a late response in the AVR's TCP stream.
        // Do not let that response become associated with the next query.
        if command_failed && index + 1 < COMMANDS.len() {
            stream = connect(&address, timeout)?;
            eprintln!("reconnected after command={command}");
        }
    }
    if failed {
        std::process::exit(1);
    }
    Ok(())
}

fn connect(address: &std::net::SocketAddr, timeout: u64) -> io::Result<TcpStream> {
    let stream = TcpStream::connect_timeout(address, Duration::from_millis(timeout))?;
    stream.set_read_timeout(Some(Duration::from_millis(timeout)))?;
    stream.set_write_timeout(Some(Duration::from_millis(timeout)))?;
    Ok(stream)
}

fn command_family(command: &str) -> &str {
    command.trim().trim_end_matches('?').trim_end()
}

fn is_optional_command(command: &str) -> bool {
    command_family(command) == "DC"
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn response_is_match(command: &str, response: &str) -> bool {
    let family = command_family(command);
    response != family && response_matches(family, response)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_expected_response_family_matches() {
        assert!(response_matches(command_family("SI?"), "SICD"));
        assert!(!response_matches(command_family("SI?"), "MSSTEREO"));
        assert!(!response_is_match("SI?", "SI"));
        assert!(response_is_match("OPINFINS ?", "OPINFINS 222200000000"));
        assert!(response_is_match("SSINFAISFSV ?", "SSINFAISFSV 48K"));
        assert!(response_is_match("PSMULTEQ: ?", "PSMULTEQ:AUDYSSEY"));
        assert!(response_is_match("PSDYNEQ ?", "PSDYNEQ ON"));
        assert!(!response_is_match("OPINFINS ?", "OPINFASP 2222"));
    }

    #[test]
    fn only_dc_is_optional() {
        assert!(is_optional_command("DC?"));
        assert!(is_optional_command("DC? "));
        assert!(!is_optional_command("SI?"));
        assert!(!is_optional_command("DCX?"));
    }
}
