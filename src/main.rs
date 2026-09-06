use denon_avr_remote::application::ApplicationService;
use denon_avr_remote::discovery::{DiscoveredReceiver, DEFAULT_DISCOVERY_TIMEOUT};
use denon_avr_remote::{AvrCommand, HeosCommand, VolumeCode};

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

fn discover_command() -> Result<(), String> {
    let receivers = ApplicationService::default().discover(DEFAULT_DISCOVERY_TIMEOUT)?;
    print_discovered(&receivers);
    if receivers.is_empty() {
        return Err("SSDP discovery returned no receiver responses".to_owned());
    }
    Ok(())
}

fn status_command(requested: Option<usize>, manual_host: Option<String>) -> Result<(), String> {
    let service = ApplicationService::default();
    let (identity, status) =
        service.query_status(requested, manual_host, DEFAULT_DISCOVERY_TIMEOUT)?;
    println!(
        "Receiver: {}",
        identity
            .friendly_name
            .as_deref()
            .or(identity.model.as_deref())
            .unwrap_or(&identity.host)
    );
    println!("{}", service.render_status(&status));
    Ok(())
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
