use denon_avr_remote::application::ConfigRepository;
use denon_avr_remote::application::{
    query_main_zone_status, resolve_status_receiver_with_probe, ReceiverDiscovery,
};
use denon_avr_remote::domain::{
    ConfiguredReceivers, DiscoveredReceiver, FieldStatus, MainZoneField, MainZoneSnapshot,
};
use denon_avr_remote::infrastructure::discovery_ssdp::{
    SsdpDiscoveryAdapter, DEFAULT_DISCOVERY_TIMEOUT,
};
use denon_avr_remote::infrastructure::{SyncAvrClient, YamlConfigRepository};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resource {
    Receivers,
    Status,
    Power,
    Input,
    Volume,
    Mute,
    Surround,
}

impl Resource {
    fn parse_resource(value: &str) -> Result<Self, String> {
        match value {
            "receivers" => Ok(Self::Receivers),
            "status" => Ok(Self::Status),
            "power" => Ok(Self::Power),
            "input" => Ok(Self::Input),
            "volume" => Ok(Self::Volume),
            "mute" => Ok(Self::Mute),
            "surround" | "surround-mode" => Ok(Self::Surround),
            _ => Err(format!(
                "unknown resource '{value}'; expected receivers, status, power, input, volume, mute, or surround"
            )),
        }
    }

    fn get_resource_field(self) -> Option<MainZoneField> {
        match self {
            Self::Power => Some(MainZoneField::Power),
            Self::Input => Some(MainZoneField::Input),
            Self::Volume => Some(MainZoneField::Volume),
            Self::Mute => Some(MainZoneField::Mute),
            Self::Surround => Some(MainZoneField::SurroundMode),
            Self::Receivers | Self::Status => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Selection {
    receiver: Option<usize>,
    host: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Help,
    Get(Resource, Selection),
}

fn print_usage() {
    println!(
        "Usage:\n  denon-avr-remote get <resource> [selector]\n\nCommands:\n  denon-avr-remote get receivers\n  denon-avr-remote get status\n  denon-avr-remote get power\n  denon-avr-remote get input\n  denon-avr-remote get volume\n  denon-avr-remote get mute\n  denon-avr-remote get surround\n\nResources:\n  receivers             List Denon and Marantz receivers\n  status                Display complete main-zone status\n  power                 Display main-zone power\n  input                 Display main-zone input\n  volume                Display main-zone volume\n  mute                  Display main-zone mute state\n  surround              Display main-zone surround mode\n\nSelectors (receiver-backed resources only):\n  --host HOST           Query a receiver directly\n  --receiver N          Query discovered receiver N (one-based)\n\nUse 'surround-mode' as an alias for 'surround'."
    );
}

fn parse_args<I>(args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Ok(Command::Help);
    };
    if matches!(command.as_str(), "help" | "--help" | "-h") {
        if args.next().is_some() {
            return Err("help does not accept arguments".to_owned());
        }
        return Ok(Command::Help);
    }
    if command != "get" {
        return Err(format!(
            "unknown command '{command}'; commands must use 'get <resource>'"
        ));
    }

    let resource = Resource::parse_resource(
        &args
            .next()
            .ok_or_else(|| "get requires a resource".to_owned())?,
    )?;
    let mut receiver = None;
    let mut host = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--receiver" => {
                if receiver.is_some() {
                    return Err("--receiver may be specified only once".to_owned());
                }
                let value = args
                    .next()
                    .ok_or_else(|| "--receiver requires a positive number".to_owned())?;
                let index = value
                    .parse::<usize>()
                    .ok()
                    .filter(|index| *index > 0)
                    .ok_or_else(|| "--receiver requires a positive number".to_owned())?;
                receiver = Some(index - 1);
            }
            "--host" => {
                if host.is_some() {
                    return Err("--host may be specified only once".to_owned());
                }
                let value = args
                    .next()
                    .ok_or_else(|| "--host requires an address".to_owned())?;
                if value.trim().is_empty() {
                    return Err("--host requires a non-empty address".to_owned());
                }
                host = Some(value);
            }
            _ => return Err(format!("unexpected argument '{argument}'")),
        }
    }
    if resource == Resource::Receivers && (receiver.is_some() || host.is_some()) {
        return Err("get receivers does not accept --host or --receiver".to_owned());
    }
    if receiver.is_some() && host.is_some() {
        return Err("--host and --receiver cannot be combined".to_owned());
    }
    Ok(Command::Get(resource, Selection { receiver, host }))
}

fn print_discovered_receivers(receivers: &[DiscoveredReceiver]) {
    if receivers.is_empty() {
        println!("No receivers found.");
        eprintln!("Check that the receiver and this computer share a LAN, multicast/SSDP is allowed, and Network Control is enabled on the receiver.");
        eprintln!("If you know the receiver IP, use: denon-avr-remote get status --host <ip>");
        return;
    }
    for (index, receiver) in receivers.iter().enumerate() {
        let name = receiver
            .model
            .as_deref()
            .or(receiver.server.as_deref())
            .unwrap_or("Denon receiver");
        println!("{}. {name} ({})", index + 1, receiver.address.host);
        if let Some(location) = &receiver.location {
            println!("   location: {location}");
        }
    }
}

fn saved_identity_is_usable(identity: &denon_avr_remote::domain::ReceiverIdentity) -> bool {
    let Ok(mut client) = SyncAvrClient::connect(
        &identity.host,
        std::time::Duration::from_secs(2),
        std::time::Duration::from_secs(1),
    ) else {
        return false;
    };
    query_main_zone_status(&mut client).probe_succeeded()
}

fn resolve_receiver_identity(
    selection: &Selection,
) -> Result<denon_avr_remote::domain::ReceiverIdentity, String> {
    let repository = YamlConfigRepository::default();
    let discovery = SsdpDiscoveryAdapter;
    let resolved = resolve_status_receiver_with_probe(
        &repository,
        &discovery,
        selection.receiver,
        selection.host.as_deref(),
        DEFAULT_DISCOVERY_TIMEOUT,
        saved_identity_is_usable,
    )
    .map_err(|error| error.to_string())?;
    Ok(resolved.identity)
}

fn query_resource(resource: Resource, selection: &Selection) -> Result<(), String> {
    let identity = resolve_receiver_identity(selection)?;
    let mut client = SyncAvrClient::connect(
        &identity.host,
        std::time::Duration::from_secs(2),
        std::time::Duration::from_secs(1),
    )
    .map_err(|error| error.to_string())?;
    let snapshot = query_main_zone_status(&mut client);
    YamlConfigRepository::default()
        .save(&ConfiguredReceivers {
            current: Some(
                identity
                    .friendly_name
                    .clone()
                    .unwrap_or_else(|| "default".to_owned()),
            ),
            receivers: BTreeMap::from([(
                identity
                    .friendly_name
                    .clone()
                    .unwrap_or_else(|| "default".to_owned()),
                identity.clone(),
            )]),
        })
        .map_err(|error| error.to_string())?;

    if resource == Resource::Status {
        println!("Receiver: {}", get_receiver_label(&identity));
        println!("{}", render_status_snapshot(&snapshot));
    } else if let Some(field) = resource.get_resource_field() {
        println!("{}", render_status_field(field, &snapshot));
    }
    Ok(())
}

fn get_receiver_label(identity: &denon_avr_remote::domain::ReceiverIdentity) -> &str {
    identity
        .friendly_name
        .as_deref()
        .or(identity.model.as_deref())
        .unwrap_or(&identity.host)
}

fn render_status_field(field: MainZoneField, snapshot: &MainZoneSnapshot) -> String {
    match field {
        MainZoneField::Power => render_status_line("Power", &snapshot.power),
        MainZoneField::Input => render_status_line("Input", &snapshot.input),
        MainZoneField::Volume => render_status_line("Volume", &snapshot.volume),
        MainZoneField::Mute => render_status_line("Mute", &snapshot.mute),
        MainZoneField::SurroundMode => render_status_line("Surround mode", &snapshot.surround_mode),
    }
}

fn render_status_snapshot(snapshot: &MainZoneSnapshot) -> String {
    [
        render_status_field(MainZoneField::Power, snapshot),
        render_status_field(MainZoneField::Input, snapshot),
        render_status_field(MainZoneField::Volume, snapshot),
        render_status_field(MainZoneField::Mute, snapshot),
        render_status_field(MainZoneField::SurroundMode, snapshot),
    ]
    .join("\n")
}

fn render_status_line<T: std::fmt::Display>(name: &str, field: &FieldStatus<T>) -> String {
    match field {
        FieldStatus::Value(value) => format!("{name}: {value}"),
        FieldStatus::Unavailable(error) => format!("{name}: unavailable ({})", error.message),
    }
}

fn execute_command(command: Command) -> Result<(), String> {
    match command {
        Command::Help => {
            print_usage();
            Ok(())
        }
        Command::Get(Resource::Receivers, _) => {
            let discovery = SsdpDiscoveryAdapter;
            let receivers = ReceiverDiscovery::discover(&discovery, DEFAULT_DISCOVERY_TIMEOUT)
                .map_err(|error| error.to_string())?;
            print_discovered_receivers(&receivers);
            Ok(())
        }
        Command::Get(resource, selection) => query_resource(resource, &selection),
    }
}

fn main() {
    if let Err(error) = parse_args(std::env::args().skip(1)).and_then(execute_command) {
        exit_with_error(&error);
    }
}

fn exit_with_error(error: &str) -> ! {
    eprintln!("error: {error}");
    print_usage();
    std::process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_supported_resource() {
        for resource in [
            "receivers",
            "status",
            "power",
            "input",
            "volume",
            "mute",
            "surround",
            "surround-mode",
        ] {
            assert!(
                parse_args(["get", resource].map(str::to_owned)).is_ok(),
                "{resource}"
            );
        }
    }

    #[test]
    fn preserves_one_based_receiver_indexes() {
        let Command::Get(_, selection) =
            parse_args(["get", "status", "--receiver", "1"].map(str::to_owned)).unwrap()
        else {
            panic!("expected get command");
        };
        assert_eq!(selection.receiver, Some(0));
        assert!(parse_args(["get", "status", "--receiver", "0"].map(str::to_owned)).is_err());
    }

    #[test]
    fn rejects_selectors_for_receiver_listing() {
        assert!(
            parse_args(["get", "receivers", "--host", "127.0.0.1"].map(str::to_owned)).is_err()
        );
        assert!(parse_args(["get", "receivers", "--receiver", "1"].map(str::to_owned)).is_err());
    }

    #[test]
    fn rejects_old_top_level_commands() {
        for command in ["discover", "status", "avr", "heos", "volume"] {
            assert!(
                parse_args([command].map(str::to_owned)).is_err(),
                "{command}"
            );
        }
    }
}
