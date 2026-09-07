use denon_avr_remote::application::{
    query_main_zone_status, resolve_status_receiver_with_probe, ConfigRepository, ReceiverDiscovery,
};
use denon_avr_remote::domain::{
    ConfiguredReceivers, DiscoveredReceiver, MainZoneControl, MainZoneField, MainZoneSnapshot,
    Model, ModelCapabilities, MuteState, PowerState, ReceiverIdentity, VolumeLevel,
};
use denon_avr_remote::infrastructure::discovery_ssdp::{
    SsdpDiscoveryAdapter, DEFAULT_DISCOVERY_TIMEOUT,
};
use denon_avr_remote::infrastructure::{SyncAvrClient, YamlConfigRepository};
use denon_avr_remote::protocol::avr::encode_control;
use std::collections::BTreeMap;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resource {
    Receivers,
    Capabilities,
    Status,
    Power,
    Input,
    Volume,
    Mute,
    Surround,
}
impl Resource {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "receivers" => Ok(Self::Receivers),
            "capabilities" => Ok(Self::Capabilities),
            "status" => Ok(Self::Status),
            "power" => Ok(Self::Power),
            "input" => Ok(Self::Input),
            "volume" | "level" => Ok(Self::Volume),
            "mute" => Ok(Self::Mute),
            "surround" | "surround-mode" => Ok(Self::Surround),
            _ => Err(format!("unknown resource '{value}'")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operation {
    Power,
    Input,
    Volume,
    Mute,
    Surround,
}
impl Operation {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "power" => Ok(Self::Power),
            "input" => Ok(Self::Input),
            "volume" | "level" => Ok(Self::Volume),
            "mute" => Ok(Self::Mute),
            "surround" | "surround-mode" => Ok(Self::Surround),
            _ => Err(format!("unknown set operation '{value}'")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Selection {
    receiver: Option<usize>,
    host: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Help,
    Version,
    Get(Resource, Selection),
    Set {
        operation: Operation,
        value: String,
        selection: Selection,
        resource_version: Option<u64>,
        dry_run: bool,
    },
}

fn print_usage() {
    println!("Usage:\n  denon-avr-remote get <resource> [selector]\n  denon-avr-remote set <operation> <value> [options] [selector]\n\nCommands:\n  get receivers\n  get capabilities [selector]\n  get status [selector]\n  get power|input|volume|level|mute|surround [selector]\n  set power|input|volume|level|mute|surround <value> [options] [selector]\n\nSelectors:\n  --host HOST           Query or control a receiver directly\n  --receiver N          Use discovered receiver N (one-based)\n\nSet options:\n  --resource-version N  Optional expected snapshot version; current version is used when omitted\n  --dry-run             Validate and show the planned action without dispatching\n\nGlobal options:\n  -h, --help            Show this help\n  -V, --version         Show the package version\n\nVolume values use 0.0–100.0 in 0.5 steps. 'level' aliases 'volume';\n'surround-mode' aliases 'surround'; 'off' aliases power 'standby'.");
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
            return Err("help does not accept arguments".into());
        }
        return Ok(Command::Help);
    }
    if matches!(command.as_str(), "--version" | "-V" | "version") {
        if args.next().is_some() {
            return Err("version does not accept arguments".into());
        }
        return Ok(Command::Version);
    }
    match command.as_str() {
        "get" => parse_get(&mut args),
        "set" => parse_set(&mut args),
        _ => Err(format!("unknown command '{command}'; expected get or set")),
    }
}
fn parse_get(args: &mut impl Iterator<Item = String>) -> Result<Command, String> {
    let resource = Resource::parse(
        &args
            .next()
            .ok_or_else(|| "get requires a resource".to_owned())?,
    )?;
    let selection = parse_selection(args)?;
    if resource == Resource::Receivers && (selection.receiver.is_some() || selection.host.is_some())
    {
        return Err("get receivers does not accept --host or --receiver".into());
    }
    Ok(Command::Get(resource, selection))
}
fn parse_set(args: &mut impl Iterator<Item = String>) -> Result<Command, String> {
    let operation = Operation::parse(
        &args
            .next()
            .ok_or_else(|| "set requires an operation".to_owned())?,
    )?;
    let value = args
        .next()
        .ok_or_else(|| "set requires a value".to_owned())?;
    let mut selection = Selection::default();
    let mut resource_version = None;
    let mut dry_run = false;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--resource-version" => {
                if resource_version.is_some() {
                    return Err("--resource-version may be specified only once".into());
                };
                let raw = args.next().ok_or_else(|| {
                    "--resource-version requires a non-negative number".to_owned()
                })?;
                resource_version =
                    Some(raw.parse::<u64>().map_err(|_| {
                        "--resource-version requires a non-negative number".to_owned()
                    })?);
            }
            "--dry-run" => {
                if dry_run {
                    return Err("--dry-run may be specified only once".into());
                };
                dry_run = true;
            }
            "--host" => parse_host(&mut selection, args)?,
            "--receiver" => parse_receiver(&mut selection, args)?,
            _ => return Err(format!("unexpected argument '{argument}'")),
        }
    }
    Ok(Command::Set {
        operation,
        value,
        selection,
        resource_version,
        dry_run,
    })
}
fn parse_selection(args: &mut impl Iterator<Item = String>) -> Result<Selection, String> {
    let mut selection = Selection::default();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--host" => parse_host(&mut selection, args)?,
            "--receiver" => parse_receiver(&mut selection, args)?,
            _ => return Err(format!("unexpected argument '{argument}'")),
        }
    }
    Ok(selection)
}
fn parse_host(
    selection: &mut Selection,
    args: &mut impl Iterator<Item = String>,
) -> Result<(), String> {
    if selection.host.is_some() {
        return Err("--host may be specified only once".into());
    }
    let value = args
        .next()
        .ok_or_else(|| "--host requires a non-empty address".to_owned())?;
    if value.trim().is_empty() {
        return Err("--host requires a non-empty address".into());
    }
    selection.host = Some(value);
    reject_combined_selector(selection)
}
fn parse_receiver(
    selection: &mut Selection,
    args: &mut impl Iterator<Item = String>,
) -> Result<(), String> {
    if selection.receiver.is_some() {
        return Err("--receiver may be specified only once".into());
    }
    let value = args
        .next()
        .ok_or_else(|| "--receiver requires a positive number".to_owned())?;
    let index = value
        .parse::<usize>()
        .ok()
        .filter(|index| *index > 0)
        .ok_or_else(|| "--receiver requires a positive number".to_owned())?;
    selection.receiver = Some(index - 1);
    reject_combined_selector(selection)
}
fn reject_combined_selector(selection: &Selection) -> Result<(), String> {
    if selection.host.is_some() && selection.receiver.is_some() {
        Err("--host and --receiver cannot be combined".into())
    } else {
        Ok(())
    }
}

fn saved_identity_is_usable(identity: &ReceiverIdentity) -> bool {
    let Ok(mut client) = SyncAvrClient::connect(
        identity.host.as_str(),
        std::time::Duration::from_secs(2),
        std::time::Duration::from_secs(1),
    ) else {
        return false;
    };
    query_main_zone_status(&mut client).probe_succeeded()
}
fn resolve_read_identity(
    selection: &Selection,
) -> Result<denon_avr_remote::application::ResolvedReceiver, String> {
    resolve_status_receiver_with_probe(
        &YamlConfigRepository::default(),
        &SsdpDiscoveryAdapter,
        selection.receiver,
        selection.host.as_deref(),
        DEFAULT_DISCOVERY_TIMEOUT,
        saved_identity_is_usable,
    )
    .map_err(|error| error.to_string())
}
fn resolve_write_identity(
    selection: &Selection,
) -> Result<denon_avr_remote::application::ResolvedReceiver, String> {
    if let Some(host) = selection.host.as_deref() {
        return Ok(denon_avr_remote::application::ResolvedReceiver {
            name: None,
            identity: ReceiverIdentity::ad_hoc(host),
        });
    }
    if let Some(index) = selection.receiver {
        let receivers = SsdpDiscoveryAdapter
            .discover(DEFAULT_DISCOVERY_TIMEOUT)
            .map_err(|error| error.to_string())?;
        let receiver = receivers
            .get(index)
            .ok_or_else(|| "receiver selection is out of range".to_owned())?;
        return Ok(denon_avr_remote::application::ResolvedReceiver {
            name: None,
            identity: receiver.identity(),
        });
    }
    let config = YamlConfigRepository::default()
        .load()
        .map_err(|error| error.to_string())?;
    let (name, identity) = config
        .current()
        .ok_or_else(|| "set requires a saved receiver or an explicit selector".to_owned())?;
    Ok(denon_avr_remote::application::ResolvedReceiver {
        name: Some(name.to_owned()),
        identity: identity.clone(),
    })
}
fn connect(identity: &ReceiverIdentity) -> Result<SyncAvrClient, String> {
    SyncAvrClient::connect(
        &identity.host,
        std::time::Duration::from_secs(2),
        std::time::Duration::from_secs(1),
    )
    .map_err(|error| error.to_string())
}

fn query_resource(resource: Resource, selection: &Selection) -> Result<(), String> {
    let resolved = resolve_read_identity(selection)?;
    let mut client = connect(&resolved.identity)?;
    let snapshot = query_main_zone_status(&mut client);
    print_target(&resolved.identity, snapshot.resource_version());
    match resource {
        Resource::Status => println!("{}", render_status_snapshot(&snapshot)),
        Resource::Capabilities => print_capabilities(&resolved.identity),
        Resource::Power => println!("{}", render_status_field(MainZoneField::Power, &snapshot)),
        Resource::Input => println!("{}", render_status_field(MainZoneField::Input, &snapshot)),
        Resource::Volume => println!("{}", render_status_field(MainZoneField::Volume, &snapshot)),
        Resource::Mute => println!("{}", render_status_field(MainZoneField::Mute, &snapshot)),
        Resource::Surround => println!(
            "{}",
            render_status_field(MainZoneField::SurroundMode, &snapshot)
        ),
        Resource::Receivers => unreachable!(),
    }
    if resource != Resource::Capabilities {
        save_identity(&resolved.identity)?;
    }
    Ok(())
}
fn print_capabilities(identity: &ReceiverIdentity) {
    let model = identity
        .model
        .as_deref()
        .map(Model::from_reported)
        .unwrap_or(Model::Unknown);
    let capabilities = ModelCapabilities::for_model(model);
    println!("Model: {:?}", model);
    if !capabilities.writable {
        println!("Writable controls: none (model is not validated)");
        return;
    }
    println!("Writable controls: power, input, volume, mute, surround");
    println!("Volume level: 0.0–100.0 (step 0.5)");
    println!("Inputs: {}", capabilities.inputs.join(", "));
    println!("Surround modes: {}", capabilities.surround_modes.join(", "));
}
fn save_identity(identity: &ReceiverIdentity) -> Result<(), String> {
    let name = identity
        .friendly_name
        .clone()
        .unwrap_or_else(|| "default".into());
    YamlConfigRepository::default()
        .save(&ConfiguredReceivers {
            current: Some(name.clone()),
            receivers: BTreeMap::from([(name, identity.clone())]),
        })
        .map_err(|error| error.to_string())
}

fn set_resource(
    operation: Operation,
    raw_value: &str,
    selection: &Selection,
    expected_version: Option<u64>,
    dry_run: bool,
) -> Result<(), String> {
    let resolved = resolve_write_identity(selection)?;
    let model = resolved
        .identity
        .model
        .as_deref()
        .map(Model::from_reported)
        .unwrap_or(Model::Unknown);
    let capabilities = ModelCapabilities::for_model(model);
    if !capabilities.writable {
        return Err("selected receiver is not a validated writable X3800H model".into());
    }
    let mut client = connect(&resolved.identity)?;
    let snapshot = query_main_zone_status(&mut client);
    if expected_version.is_some_and(|version| snapshot.resource_version() != version) {
        let expected = expected_version.expect("checked above");
        return Err(format!(
            "stale resource version: expected {expected}, current {}; run get status",
            snapshot.resource_version()
        ));
    }
    let effective_version = snapshot.resource_version();
    let control = parse_control(operation, raw_value, &capabilities)?;
    let field = operation_field(operation);
    if snapshot
        .value(field)
        .is_some_and(|value| control_matches(&control, &value))
    {
        print_target(&resolved.identity, effective_version);
        println!("No-op: {} is already {}", field.name(), raw_value);
        return Ok(());
    }
    let command = encode_control(&control).map_err(|error| error.to_string())?;
    print_target(&resolved.identity, effective_version);
    if dry_run {
        println!("Dry run: would dispatch {}", command.as_str());
        println!("Requested {}: {}", field.name(), raw_value);
        return Ok(());
    }
    client
        .send_once(&command)
        .map_err(|error| error.to_string())?;
    println!("Dispatched once: {}", command.as_str());
    println!("Not confirmed; run get status to verify the receiver state.");
    Ok(())
}
fn parse_control(
    operation: Operation,
    value: &str,
    capabilities: &ModelCapabilities,
) -> Result<MainZoneControl, String> {
    let control = match operation {
        Operation::Power => MainZoneControl::Power(match value {
            "on" => PowerState::On,
            "off" | "standby" => PowerState::Standby,
            _ => return Err("power must be on, off, or standby".into()),
        }),
        Operation::Input => {
            MainZoneControl::Input(capabilities.input(value).map_err(str::to_owned)?)
        }
        Operation::Volume => MainZoneControl::Volume(parse_volume_level(value)?),
        Operation::Mute => MainZoneControl::Mute(match value {
            "on" => MuteState::On,
            "off" => MuteState::Off,
            _ => return Err("mute must be on or off".into()),
        }),
        Operation::Surround => {
            MainZoneControl::SurroundMode(capabilities.surround_mode(value).map_err(str::to_owned)?)
        }
    };
    if capabilities.supports_control(&control) {
        Ok(control)
    } else {
        Err("requested value is not in the validated capability allowlist".into())
    }
}
fn parse_volume_level(value: &str) -> Result<VolumeLevel, String> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, "0"));
    if whole.is_empty()
        || !whole.chars().all(|c| c.is_ascii_digit())
        || fraction.len() > 1
        || !fraction.chars().all(|c| c.is_ascii_digit())
    {
        return Err("volume level must be between 0.0 and 100.0 in 0.5 steps".into());
    }
    let tenths = whole
        .parse::<u16>()
        .ok()
        .and_then(|v| v.checked_mul(10))
        .and_then(|v| v.checked_add(fraction.parse::<u16>().ok()?))
        .ok_or_else(|| "volume level must be between 0.0 and 100.0 in 0.5 steps".to_owned())?;
    VolumeLevel::new(tenths).map_err(str::to_owned)
}
fn operation_field(operation: Operation) -> MainZoneField {
    match operation {
        Operation::Power => MainZoneField::Power,
        Operation::Input => MainZoneField::Input,
        Operation::Volume => MainZoneField::Volume,
        Operation::Mute => MainZoneField::Mute,
        Operation::Surround => MainZoneField::SurroundMode,
    }
}
fn control_matches(
    control: &MainZoneControl,
    value: &denon_avr_remote::domain::MainZoneValue,
) -> bool {
    match (control, value) {
        (MainZoneControl::Power(a), denon_avr_remote::domain::MainZoneValue::Power(b)) => a == b,
        (MainZoneControl::Input(a), denon_avr_remote::domain::MainZoneValue::Input(b)) => a == b,
        (MainZoneControl::Volume(a), denon_avr_remote::domain::MainZoneValue::Volume(b)) => {
            b.level().ok().as_ref() == Some(a)
        }
        (MainZoneControl::Mute(a), denon_avr_remote::domain::MainZoneValue::Mute(b)) => a == b,
        (
            MainZoneControl::SurroundMode(a),
            denon_avr_remote::domain::MainZoneValue::SurroundMode(b),
        ) => a == b,
        _ => false,
    }
}
fn print_discovered_receivers(receivers: &[DiscoveredReceiver]) {
    if receivers.is_empty() {
        println!("No receivers found.");
        eprintln!("Check that the receiver and this computer share a LAN, multicast/SSDP is allowed, and Network Control is enabled on the receiver.");
        return;
    }
    for (index, receiver) in receivers.iter().enumerate() {
        let name = receiver
            .model
            .as_deref()
            .or(receiver.server.as_deref())
            .unwrap_or("Denon receiver");
        println!("{}. {name} ({})", index + 1, receiver.address.host);
    }
}
fn print_target(identity: &ReceiverIdentity, version: u64) {
    println!(
        "Receiver: {} ({})",
        identity
            .friendly_name
            .as_deref()
            .or(identity.model.as_deref())
            .unwrap_or("receiver"),
        identity.host
    );
    println!("Resource version: {version}");
}
fn render_status_field(field: MainZoneField, snapshot: &MainZoneSnapshot) -> String {
    match field {
        MainZoneField::Power => render_status_line("Power", &snapshot.power),
        MainZoneField::Input => render_status_line("Input", &snapshot.input),
        MainZoneField::Volume => snapshot.volume.value().map_or_else(
            || render_status_line("Volume level", &snapshot.volume),
            |volume| {
                format!(
                    "Volume level: {}",
                    volume
                        .level()
                        .map_or_else(|_| "unavailable".into(), |level| level.to_string())
                )
            },
        ),
        MainZoneField::Mute => render_status_line("Mute", &snapshot.mute),
        MainZoneField::SurroundMode => render_status_line("Surround mode", &snapshot.surround_mode),
    }
}
fn render_status_snapshot(snapshot: &MainZoneSnapshot) -> String {
    MainZoneField::ALL
        .into_iter()
        .map(|field| render_status_field(field, snapshot))
        .collect::<Vec<_>>()
        .join("\n")
}
fn render_status_line<T: std::fmt::Display>(
    name: &str,
    field: &denon_avr_remote::domain::FieldStatus<T>,
) -> String {
    match field {
        denon_avr_remote::domain::FieldStatus::Value(value) => format!("{name}: {value}"),
        denon_avr_remote::domain::FieldStatus::Unavailable(error) => {
            format!("{name}: unavailable ({})", error.message)
        }
    }
}
fn execute_command(command: Command) -> Result<(), String> {
    match command {
        Command::Help => print_usage(),
        Command::Version => println!("denon-avr-remote {VERSION}"),
        Command::Get(Resource::Receivers, _) => {
            let receivers = SsdpDiscoveryAdapter
                .discover(DEFAULT_DISCOVERY_TIMEOUT)
                .map_err(|error| error.to_string())?;
            print_discovered_receivers(&receivers);
        }
        Command::Get(resource, selection) => query_resource(resource, &selection)?,
        Command::Set {
            operation,
            value,
            selection,
            resource_version,
            dry_run,
        } => set_resource(operation, &value, &selection, resource_version, dry_run)?,
    }
    Ok(())
}
fn main() {
    if let Err(error) = parse_args(std::env::args().skip(1)).and_then(execute_command) {
        eprintln!("error: {error}");
        print_usage();
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_reads_and_set() {
        assert!(matches!(
            parse_args(["get", "capabilities"].map(str::to_owned)),
            Ok(Command::Get(Resource::Capabilities, _))
        ));
        assert!(matches!(
            parse_args(["set", "volume", "50.5", "--resource-version", "12"].map(str::to_owned)),
            Ok(Command::Set { .. })
        ));
    }
    #[test]
    fn parses_volume_levels() {
        assert_eq!(parse_volume_level("50").unwrap().to_string(), "50.0");
        assert_eq!(parse_volume_level("50.5").unwrap().to_string(), "50.5");
        assert!(parse_volume_level("50.1").is_err());
        assert!(parse_volume_level("100.5").is_err());
    }
    #[test]
    fn accepts_automatic_versioning() {
        assert!(matches!(
            parse_args(["set", "power", "on"].map(str::to_owned)),
            Ok(Command::Set {
                resource_version: None,
                ..
            })
        ));
    }
    #[test]
    fn supports_aliases() {
        assert_eq!(Resource::parse("level").unwrap(), Resource::Volume);
        assert_eq!(
            Operation::parse("surround-mode").unwrap(),
            Operation::Surround
        );
    }
}
