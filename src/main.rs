use denon_avr_remote::config::{self, Config, ReceiverIdentity};
use denon_avr_remote::discovery::{self, DiscoveredReceiver, DEFAULT_DISCOVERY_TIMEOUT};
use denon_avr_remote::status::{query_main_zone, render, TcpAvrTransport};
use denon_avr_remote::{AvrCommand, HeosCommand, VolumeCode};
use std::io::{self, Write};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const READ_TIMEOUT: Duration = Duration::from_secs(1);

fn usage() {
    println!(
        "Usage:\n  denon-avr-remote <command> [value]\n\nCommands:\n  discover             Discover Denon receivers\n  status               Display saved or discovered main-zone status\n  status --receiver N  Display status for discovered receiver N\n  status --host HOST   Query a receiver directly, bypassing discovery\n  avr <command>        Frame an AVR command (development tool)\n  heos <command>       Frame a HEOS command (development tool)\n  volume <db-tenths>   Convert dB tenths to an AVR volume command\n  help                 Show this help"
    );
}

fn print_discovered(receivers: &[DiscoveredReceiver]) {
    if receivers.is_empty() {
        println!("No receivers found.");
        eprintln!("Check that the receiver and this computer share a LAN, multicast/SSDP is allowed, and Network Control is enabled on the receiver.");
        eprintln!("If you know the receiver IP, use: denon-avr-remote status --host <ip>");
        return;
    }
    for (index, receiver) in receivers.iter().enumerate() {
        let name = receiver
            .model
            .as_deref()
            .or(receiver.server.as_deref())
            .unwrap_or("Denon receiver");
        println!("{}. {name} ({})", index + 1, receiver.address.ip());
        if let Some(location) = &receiver.location {
            println!("   location: {location}");
        }
    }
}

fn choose_receiver(
    receivers: &[DiscoveredReceiver],
    requested: Option<usize>,
) -> Result<&DiscoveredReceiver, String> {
    if receivers.is_empty() {
        return Err("no receivers discovered".to_owned());
    }
    let index = match requested {
        Some(index) => index,
        None if receivers.len() == 1 => 0,
        None => {
            print_discovered(receivers);
            print!("Select receiver [1-{}]: ", receivers.len());
            io::stdout().flush().map_err(|e| e.to_string())?;
            let mut input = String::new();
            io::stdin()
                .read_line(&mut input)
                .map_err(|e| e.to_string())?;
            input
                .trim()
                .parse::<usize>()
                .map_err(|_| "receiver selection must be a number".to_owned())?
                .saturating_sub(1)
        }
    };
    receivers
        .get(index)
        .ok_or_else(|| "receiver selection is out of range".to_owned())
}

fn receiver_from_discovery(receiver: &DiscoveredReceiver) -> ReceiverIdentity {
    ReceiverIdentity {
        host: receiver.address.ip().to_string(),
        model: receiver.model.clone(),
        friendly_name: None,
    }
}

fn discover_command() -> Result<(), String> {
    let receivers = discovery::discover(DEFAULT_DISCOVERY_TIMEOUT).map_err(|e| e.to_string())?;
    print_discovered(&receivers);
    if receivers.is_empty() {
        return Err("SSDP discovery returned no receiver responses".to_owned());
    }
    Ok(())
}

fn status_command(requested: Option<usize>, manual_host: Option<String>) -> Result<(), String> {
    if let Some(host) = manual_host {
        return query_identity(ReceiverIdentity {
            host,
            model: None,
            friendly_name: None,
        });
    }
    let path = config::default_path();
    let saved = match config::load(&path) {
        Ok(saved) => saved,
        Err(error) => {
            eprintln!("Saved configuration unavailable ({error}); trying discovery.");
            None
        }
    };
    if let Some(config) = saved.filter(|_| requested.is_none()) {
        match query_identity(config.receiver) {
            Ok(()) => return Ok(()),
            Err(error) => {
                eprintln!("Saved receiver unavailable ({error}); trying discovery.");
            }
        }
    }
    query_identity(discover_identity(requested)?)
}

fn query_identity(identity: ReceiverIdentity) -> Result<(), String> {
    let mut transport = TcpAvrTransport::connect(&identity.host, CONNECT_TIMEOUT, READ_TIMEOUT)
        .map_err(|error| error.to_string())?;
    let status = query_main_zone(&mut transport);
    config::save(
        &config::default_path(),
        &Config {
            receiver: identity.clone(),
        },
    )
    .map_err(|error| error.to_string())?;
    println!(
        "Receiver: {}",
        identity
            .friendly_name
            .as_deref()
            .or(identity.model.as_deref())
            .unwrap_or(&identity.host)
    );
    println!("{}", render(&status));
    Ok(())
}

fn discover_identity(requested: Option<usize>) -> Result<ReceiverIdentity, String> {
    let receivers = discovery::discover(DEFAULT_DISCOVERY_TIMEOUT).map_err(|e| e.to_string())?;
    let receiver = choose_receiver(&receivers, requested)?;
    Ok(receiver_from_discovery(receiver))
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        usage();
        return;
    };
    let result = match command.as_str() {
        "discover" => discover_command(),
        "status" => {
            let mut requested = None;
            let mut manual_host = None;
            while let Some(argument) = args.next() {
                if argument == "--receiver" {
                    let value = args
                        .next()
                        .unwrap_or_else(|| fail("--receiver requires a number"));
                    requested = Some(
                        value
                            .parse::<usize>()
                            .unwrap_or_else(|_| fail("--receiver requires a number")),
                    );
                } else if argument == "--host" {
                    let host = args
                        .next()
                        .unwrap_or_else(|| fail("--host requires an address"));
                    if manual_host.is_some() {
                        fail("--host requires one address");
                    }
                    manual_host = Some(host);
                } else {
                    fail("status accepts --receiver N or --host HOST");
                }
            }
            if manual_host.is_some() && requested.is_some() {
                fail("--host and --receiver cannot be combined");
            }
            status_command(requested.map(|value| value.saturating_sub(1)), manual_host)
        }
        "avr" => args
            .next()
            .ok_or_else(|| "avr requires a command".to_owned())
            .and_then(|value| AvrCommand::new(value).map_err(|e| e.to_string()))
            .map(|command| print!("{}", String::from_utf8_lossy(&command.as_bytes()))),
        "heos" => args
            .next()
            .ok_or_else(|| "heos requires a command".to_owned())
            .and_then(|value| HeosCommand::new(value).map_err(|e| e.to_string()))
            .map(|command| print!("{}", String::from_utf8_lossy(&command.as_bytes()))),
        "volume" => args
            .next()
            .ok_or_else(|| "volume requires dB tenths".to_owned())
            .and_then(|value| {
                value
                    .parse::<i16>()
                    .map_err(|_| "volume must be an integer".to_owned())
            })
            .and_then(|value| VolumeCode::from_db_tenths(value).map_err(|e| e.to_string()))
            .map(|volume| println!("{}", volume.command())),
        "help" | "--help" | "-h" => {
            usage();
            Ok(())
        }
        _ => Err("unknown command".to_owned()),
    };
    if let Err(error) = result {
        fail(&error);
    }
}

fn fail(error: &str) -> ! {
    eprintln!("error: {error}");
    usage();
    std::process::exit(2);
}
