//! Command-line arguments: the commands, resources, operations and selectors
//! the CLI accepts, how they are parsed, and the usage text.

const RESOURCE_VERSION_REMOVED: &str =
    "--resource-version was removed; receiver state has no compare-and-set token";

/// A piece of receiver state that `get` can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Resource {
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
pub(super) enum Operation {
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
pub(super) struct Selection {
    pub(super) receiver: Option<usize>,
    pub(super) host: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Command {
    Help,
    Version,
    /// `get receivers`: list the receivers discovery finds.
    Receivers,
    Get(Resource, Selection),
    Set {
        operation: Operation,
        value: String,
        selection: Selection,
        dry_run: bool,
    },
}

pub(super) fn print_usage() {
    println!("Usage:\n  denon-avr-remote get <resource> [selector]\n  denon-avr-remote set <operation> <value> [--dry-run] [selector]\n\nResources: receivers, status, power, input, volume, mute, surround\nSelectors: --host HOST | --receiver N\n\nMain Zone power uses ZM. Volume uses exact dB values from -79.5 through +18.0 in 0.5 dB steps, or 'min'.\nThe old --resource-version option is removed: a local snapshot counter cannot provide receiver-side compare-and-set.");
}

pub(super) fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Command, String> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Ok(Command::Help);
    };
    match command.as_str() {
        "help" | "--help" | "-h" if args.next().is_none() => Ok(Command::Help),
        "version" | "--version" | "-V" if args.next().is_none() => Ok(Command::Version),
        "get" => {
            let resource = args.next().ok_or("get requires a resource")?;
            if resource == "receivers" {
                let selection = parse_selection(&mut args)?;
                if selection.host.is_some() || selection.receiver.is_some() {
                    Err("get receivers does not accept a selector".into())
                } else {
                    Ok(Command::Receivers)
                }
            } else {
                let resource = Resource::parse(&resource)?;
                let selection = parse_selection(&mut args)?;
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
                    "--dry-run" if !dry_run => dry_run = true,
                    "--dry-run" => return Err("--dry-run may be specified only once".into()),
                    "--host" => parse_host(&mut selection, &mut args)?,
                    "--receiver" => parse_receiver(&mut selection, &mut args)?,
                    "--resource-version" => return Err(RESOURCE_VERSION_REMOVED.into()),
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
