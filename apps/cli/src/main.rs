//! Short-lived composition for the canonical Phase 5 receiver service.

use denon_avr_application::ports::{ConfigRepository, ReceiverDiscovery};
use denon_avr_application::{CanonicalReceiverSession, OperationRequest};
use denon_avr_domain::{
    ConfiguredReceivers, DiscoveredReceiver, MasterVolume, MuteState, OperationId, ReceiverId,
    ReceiverIdentity, ReceiverIntent, ReceiverState, SoundModeIntent, ZonePower,
};
use denon_avr_infrastructure::discovery_ssdp::{SsdpDiscoveryAdapter, DEFAULT_DISCOVERY_TIMEOUT};
use denon_avr_infrastructure::{AvrSessionConfig, X3800hSession, YamlConfigRepository};
use std::collections::BTreeMap;

const VERSION: &str = env!("CARGO_PKG_VERSION");

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
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "receivers" => Ok(Self::Receivers),
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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
        dry_run: bool,
    },
}

fn print_usage() {
    println!("Usage:\n  denon-avr-remote get <resource> [selector]\n  denon-avr-remote set <operation> <value> [--dry-run] [selector]\n\nResources: receivers, status, power, input, volume, mute, surround\nSelectors: --host HOST | --receiver N\n\nMain Zone power uses ZM. Volume uses exact dB values from -79.5 through +18.0 in 0.5 dB steps, or 'min'.\nThe old --resource-version option is removed: a local snapshot counter cannot provide receiver-side compare-and-set.");
}

fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Command, String> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Ok(Command::Help);
    };
    match command.as_str() {
        "help" | "--help" | "-h" if args.next().is_none() => Ok(Command::Help),
        "version" | "--version" | "-V" if args.next().is_none() => Ok(Command::Version),
        "get" => {
            let resource = Resource::parse(&args.next().ok_or("get requires a resource")?)?;
            let selection = parse_selection(&mut args)?;
            if resource == Resource::Receivers
                && (selection.host.is_some() || selection.receiver.is_some())
            {
                Err("get receivers does not accept a selector".into())
            } else {
                Ok(Command::Get(resource, selection))
            }
        }
        "set" => {
            let operation = Operation::parse(&args.next().ok_or("set requires an operation")?)?;
            let value = args.next().ok_or("set requires a value")?;
            let mut selection = Selection::default();
            let mut dry_run = false;
            while let Some(arg) = args.next() {
                match arg.as_str() {
                "--dry-run" if !dry_run => dry_run = true, "--dry-run" => return Err("--dry-run may be specified only once".into()),
                "--host" => parse_host(&mut selection, &mut args)?, "--receiver" => parse_receiver(&mut selection, &mut args)?,
                "--resource-version" => return Err("--resource-version was removed; receiver state has no compare-and-set token".into()),
                _ => return Err(format!("unexpected argument '{arg}'")),
            }
            }
            Ok(Command::Set {
                operation,
                value,
                selection,
                dry_run,
            })
        }
        _ => Err(format!("unknown command '{command}'; expected get or set")),
    }
}
fn parse_selection(args: &mut impl Iterator<Item = String>) -> Result<Selection, String> {
    let mut selection = Selection::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--host" => parse_host(&mut selection, args)?,
            "--receiver" => parse_receiver(&mut selection, args)?,
            _ => return Err(format!("unexpected argument '{arg}'")),
        }
    }
    Ok(selection)
}
fn parse_host(
    selection: &mut Selection,
    args: &mut impl Iterator<Item = String>,
) -> Result<(), String> {
    let host = args
        .next()
        .filter(|host| !host.trim().is_empty())
        .ok_or("--host requires a non-empty address")?;
    if selection.host.replace(host).is_some() {
        return Err("--host may be specified only once".into());
    }
    if selection.receiver.is_some() {
        return Err("--host and --receiver cannot be combined".into());
    }
    Ok(())
}
fn parse_receiver(
    selection: &mut Selection,
    args: &mut impl Iterator<Item = String>,
) -> Result<(), String> {
    let index = args
        .next()
        .and_then(|raw| raw.parse::<usize>().ok())
        .filter(|index| *index > 0)
        .ok_or("--receiver requires a positive number")?
        - 1;
    if selection.receiver.replace(index).is_some() {
        return Err("--receiver may be specified only once".into());
    }
    if selection.host.is_some() {
        return Err("--host and --receiver cannot be combined".into());
    }
    Ok(())
}

fn resolve_identity(selection: &Selection) -> Result<ReceiverIdentity, String> {
    if let Some(host) = &selection.host {
        return Ok(ReceiverIdentity::ad_hoc(host));
    }
    if let Some(index) = selection.receiver {
        return SsdpDiscoveryAdapter
            .discover(DEFAULT_DISCOVERY_TIMEOUT)
            .map_err(|error| error.to_string())?
            .get(index)
            .map(DiscoveredReceiver::identity)
            .ok_or("receiver selection is out of range".into());
    }
    YamlConfigRepository::default()
        .load()
        .map_err(|error| error.to_string())?
        .current()
        .map(|(_, identity)| identity.clone())
        .ok_or("a saved receiver or explicit selector is required".into())
}
async fn session(identity: &ReceiverIdentity) -> Result<std::sync::Arc<X3800hSession>, String> {
    X3800hSession::connect(
        ReceiverId::new(identity.host.clone()).map_err(str::to_owned)?,
        &identity.host,
        AvrSessionConfig::default(),
    )
    .await
    .map_err(|error| error.to_string())
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
            ..ConfiguredReceivers::default()
        })
        .map_err(|error| error.to_string())
}

async fn query(resource: Resource, selection: Selection) -> Result<(), String> {
    let identity = resolve_identity(&selection)?;
    let service = session(&identity).await?;
    let readiness = service
        .synchronize()
        .await
        .map_err(|error| error.to_string())?;
    print_target(&identity);
    render_resource(resource, &service.current_state());
    println!(
        "Readiness: {}{}",
        if readiness.ready { "ready" } else { "degraded" },
        if readiness.degraded {
            format!(" ({})", readiness.detail)
        } else {
            String::new()
        }
    );
    service.close().await.map_err(|error| error.to_string())?;
    save_identity(&identity)
}
async fn set(
    operation: Operation,
    raw: String,
    selection: Selection,
    dry_run: bool,
) -> Result<(), String> {
    let intent = parse_intent(operation, &raw)?;
    if dry_run {
        println!("Dry run: would submit {intent:?}");
        return Ok(());
    }
    let identity = resolve_identity(&selection)?;
    let service = session(&identity).await?;
    let _ = service
        .synchronize()
        .await
        .map_err(|error| error.to_string())?;
    let outcome = service
        .operate(OperationRequest {
            id: OperationId(1),
            intent,
        })
        .await;
    print_target(&identity);
    println!("Outcome: {outcome:?}");
    service.close().await.map_err(|error| error.to_string())?;
    save_identity(&identity)
}
fn parse_intent(operation: Operation, value: &str) -> Result<ReceiverIntent, String> {
    match operation {
        Operation::Power => match value {
            "on" => Ok(ReceiverIntent::MainZonePower(ZonePower::On)),
            "off" => Ok(ReceiverIntent::MainZonePower(ZonePower::Off)),
            _ => Err("power must be on or off".into()),
        },
        Operation::Input => Ok(ReceiverIntent::Source(
            denon_avr_domain::SourceId::new(value).map_err(str::to_owned)?,
        )),
        Operation::Volume => Ok(ReceiverIntent::Volume(parse_volume(value)?)),
        Operation::Mute => match value {
            "on" => Ok(ReceiverIntent::Mute(MuteState::On)),
            "off" => Ok(ReceiverIntent::Mute(MuteState::Off)),
            _ => Err("mute must be on or off".into()),
        },
        Operation::Surround => Ok(ReceiverIntent::SoundMode(SoundModeIntent::Select(
            value.into(),
        ))),
    }
}
fn parse_volume(raw: &str) -> Result<MasterVolume, String> {
    if raw.eq_ignore_ascii_case("min") {
        return Ok(MasterVolume::Minimum);
    }
    let negative = raw.starts_with('-');
    let digits = raw.strip_prefix(['-', '+']).unwrap_or(raw);
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, "0"));
    if whole.is_empty()
        || !whole.chars().all(|c| c.is_ascii_digit())
        || !matches!(fraction, "0" | "5")
    {
        return Err("volume must be min or an exact 0.5 dB value from -79.5 through +18.0".into());
    }
    let half = whole
        .parse::<i16>()
        .ok()
        .and_then(|value| value.checked_mul(2))
        .and_then(|value| value.checked_add(if fraction == "5" { 1 } else { 0 }))
        .ok_or("invalid volume")?;
    MasterVolume::db_half_steps(if negative { -half } else { half }).map_err(str::to_owned)
}
fn print_target(identity: &ReceiverIdentity) {
    println!(
        "Receiver: {} ({})",
        identity
            .friendly_name
            .as_deref()
            .or(identity.model.as_deref())
            .unwrap_or("receiver"),
        identity.host
    );
}
fn render_resource(resource: Resource, state: &ReceiverState) {
    match resource {
        Resource::Status => println!("{state:#?}"),
        Resource::Power => println!("Main Zone power: {:?}", state.main_zone.power),
        Resource::Input => println!("Source: {:?}", state.main_zone.source),
        Resource::Volume => println!("Volume: {:?}", state.main_zone.volume),
        Resource::Mute => println!("Mute: {:?}", state.main_zone.mute),
        Resource::Surround => println!("Sound mode: {:?}", state.main_zone.sound_mode),
        Resource::Receivers => unreachable!(),
    }
}
fn print_receivers(receivers: &[DiscoveredReceiver]) {
    if receivers.is_empty() {
        println!("No receivers found.");
    }
    for (index, receiver) in receivers.iter().enumerate() {
        println!(
            "{}. {} ({})",
            index + 1,
            receiver
                .model
                .as_deref()
                .or(receiver.server.as_deref())
                .unwrap_or("Denon receiver"),
            receiver.address.host
        );
    }
}
async fn execute(command: Command) -> Result<(), String> {
    match command {
        Command::Help => print_usage(),
        Command::Version => println!("denon-avr-remote {VERSION}"),
        Command::Get(Resource::Receivers, _) => print_receivers(
            &SsdpDiscoveryAdapter
                .discover(DEFAULT_DISCOVERY_TIMEOUT)
                .map_err(|error| error.to_string())?,
        ),
        Command::Get(resource, selection) => query(resource, selection).await?,
        Command::Set {
            operation,
            value,
            selection,
            dry_run,
        } => set(operation, value, selection, dry_run).await?,
    };
    Ok(())
}
fn main() {
    let result = parse_args(std::env::args().skip(1)).and_then(|command| {
        tokio::runtime::Runtime::new()
            .map_err(|error| error.to_string())?
            .block_on(execute(command))
    });
    if let Err(error) = result {
        eprintln!("error: {error}");
        print_usage();
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_exact_volume() {
        assert_eq!(
            parse_volume("-79.5").unwrap(),
            MasterVolume::db_half_steps(-159).unwrap()
        );
        assert!(parse_volume("98.5").is_err());
    }
    #[test]
    fn removes_local_cas_option() {
        assert!(
            parse_args(["set", "volume", "-20", "--resource-version", "1"].map(str::to_owned))
                .is_err()
        );
    }
    #[test]
    fn power_is_main_zone_intent() {
        assert!(matches!(
            parse_intent(Operation::Power, "on"),
            Ok(ReceiverIntent::MainZonePower(ZonePower::On))
        ));
    }
}
