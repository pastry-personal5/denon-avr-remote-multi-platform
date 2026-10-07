//! Short-lived composition over the in-process control service.
//!
//! The CLI reaches the receiver only through the control-service port. It
//! composes the service, so it still links infrastructure for now; it
//! constructs no session of its own and allocates no operation ids.

use denon_avr_application::{
    ControlError, ControlService, OperationSnapshot, OperationSubmission, OperatorControl,
    ServiceConfig,
};
use denon_avr_domain::{
    ConfiguredReceivers, CoreField, DiscoveredReceiver, FieldBaseline, MasterVolume, MuteState,
    ReceiverId, ReceiverIdentity, ReceiverIntent, ReceiverState, SoundModeIntent, ZonePower,
};
use denon_avr_infrastructure::discovery_ssdp::DEFAULT_DISCOVERY_TIMEOUT;
use denon_avr_infrastructure::{SsdpDiscoveryAdapter, X3800hConnector, YamlConfigRepository};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// How long one wait for an operation may last. The service caps it as well, so
/// the loop below asks again until the operation is finished.
const OPERATION_WAIT: Duration = Duration::from_secs(30);

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

/// What a command produced. Text goes to standard output and warnings to
/// standard error, so a failure to remember the receiver never hides a result.
#[derive(Debug, Default)]
struct Report {
    text: String,
    warnings: Vec<String>,
}

/// The receiver a command acts on.
struct Target {
    id: ReceiverId,
    identity: ReceiverIdentity,
    /// The configuration entry name, when the receiver is a saved one.
    saved_name: Option<String>,
}

fn describe(error: ControlError) -> String {
    error.to_string()
}

async fn resolve_target(
    operator: &dyn OperatorControl,
    selection: &Selection,
) -> Result<Target, String> {
    let identity = if let Some(host) = &selection.host {
        ReceiverIdentity::ad_hoc(host)
    } else if let Some(index) = selection.receiver {
        operator
            .discover(DEFAULT_DISCOVERY_TIMEOUT)
            .await
            .map_err(describe)?
            .get(index)
            .map(DiscoveredReceiver::identity)
            .ok_or("receiver selection is out of range")?
    } else {
        let configuration = operator.configuration().await.map_err(describe)?;
        let (name, identity) = configuration
            .current()
            .ok_or("a saved receiver or explicit selector is required")?;
        return Ok(Target {
            id: ReceiverId::new(name).map_err(str::to_owned)?,
            identity: identity.clone(),
            saved_name: Some(name.to_owned()),
        });
    };
    let id = operator
        .register_ad_hoc(identity.clone())
        .await
        .map_err(describe)?;
    Ok(Target {
        id,
        identity,
        saved_name: None,
    })
}

/// Remember the receiver a command used as the current one, as every selected
/// read or write has done since version 1.
///
/// A configuration with several receivers was written by hand, and nothing here
/// adds a second one, so it is left alone. A saved receiver keeps its entry
/// name, which is its id.
async fn remember(operator: &dyn OperatorControl, target: &Target) -> Result<(), String> {
    let configuration = operator.configuration().await.map_err(describe)?;
    if configuration.receivers.len() > 1 {
        return Ok(());
    }
    let name = target.saved_name.clone().unwrap_or_else(|| {
        target
            .identity
            .friendly_name
            .clone()
            .unwrap_or_else(|| "default".into())
    });
    let mut sound_mode_favorites = configuration.sound_mode_favorites;
    sound_mode_favorites.retain(|receiver, _| *receiver == name);
    operator
        .save_configuration(&ConfiguredReceivers {
            current: Some(name.clone()),
            receivers: BTreeMap::from([(name, target.identity.clone())]),
            sound_mode_favorites,
        })
        .await
        .map_err(describe)
}

async fn remember_or_warn(operator: &dyn OperatorControl, target: &Target, report: &mut Report) {
    if let Err(error) = remember(operator, target).await {
        report
            .warnings
            .push(format!("could not remember the receiver: {error}"));
    }
}

fn readiness_line(state: &ReceiverState) -> String {
    const CORE: [CoreField; 7] = [
        CoreField::SystemPower,
        CoreField::MainZonePower,
        CoreField::Zone2Power,
        CoreField::Source,
        CoreField::Volume,
        CoreField::Mute,
        CoreField::SoundMode,
    ];
    let usable = CORE.iter().all(|field| {
        matches!(
            FieldBaseline::capture(state, *field),
            FieldBaseline::Value(_)
        )
    });
    if usable {
        "Readiness: ready\n".into()
    } else {
        "Readiness: degraded (one or more core fields could not be observed)\n".into()
    }
}

async fn query(
    operator: &dyn OperatorControl,
    resource: Resource,
    selection: Selection,
) -> Result<Report, String> {
    let target = resolve_target(operator, &selection).await?;
    // Resolves once the receiver is connected and synchronized.
    let subscription = operator.state(&target.id).await.map_err(describe)?;
    let state = subscription.latest();
    let mut report = Report {
        text: target_line(&target.identity) + &render_resource(resource, &state),
        warnings: Vec::new(),
    };
    report.text += &readiness_line(&state);
    drop(subscription);
    remember_or_warn(operator, &target, &mut report).await;
    Ok(report)
}

async fn set(
    operator: &dyn OperatorControl,
    operation: Operation,
    raw: String,
    selection: Selection,
    dry_run: bool,
) -> Result<Report, String> {
    let intent = parse_intent(operation, &raw)?;
    if dry_run {
        return Ok(Report {
            text: format!("Dry run: would submit {intent:?}\n"),
            warnings: Vec::new(),
        });
    }
    let target = resolve_target(operator, &selection).await?;
    let mut snapshot = operator
        .submit(&target.id, OperationSubmission::new(intent))
        .await
        .map_err(describe)?;
    while !snapshot.status.is_terminal() {
        snapshot = operator
            .operation(snapshot.id, Some(OPERATION_WAIT))
            .await
            .map_err(describe)?;
    }
    let mut report = Report {
        text: target_line(&target.identity) + &outcome_text(&snapshot),
        warnings: Vec::new(),
    };
    remember_or_warn(operator, &target, &mut report).await;
    Ok(report)
}

/// An outcome in the design's terms: what the service decided, whether anything
/// was dispatched, and whether receiver evidence confirms the requested value.
/// A completed write is not a confirmation on its own.
fn outcome_text(snapshot: &OperationSnapshot) -> String {
    let mut text = format!("Outcome: {}", snapshot.status.as_str());
    if let denon_avr_application::OperationStatus::Superseded { by } = &snapshot.status {
        text += &format!(" (by operation {})", by.0);
    }
    text += &format!(
        "\nDispatch: {}\nConfirmed: {}\n",
        snapshot.dispatch.as_str(),
        snapshot.confirmed
    );
    if let Some(observation) = &snapshot.observation {
        text += &format!("Observation: {observation}\n");
    }
    if let Some(reason) = &snapshot.reason {
        text += &format!("Reason: {reason}\n");
    }
    text
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
fn target_line(identity: &ReceiverIdentity) -> String {
    format!(
        "Receiver: {} ({})\n",
        identity
            .friendly_name
            .as_deref()
            .or(identity.model.as_deref())
            .unwrap_or("receiver"),
        identity.host
    )
}
fn render_resource(resource: Resource, state: &ReceiverState) -> String {
    match resource {
        Resource::Status => format!("{state:#?}\n"),
        Resource::Power => format!("Main Zone power: {:?}\n", state.main_zone.power),
        Resource::Input => format!("Source: {:?}\n", state.main_zone.source),
        Resource::Volume => format!("Volume: {:?}\n", state.main_zone.volume),
        Resource::Mute => format!("Mute: {:?}\n", state.main_zone.mute),
        Resource::Surround => format!("Sound mode: {:?}\n", state.main_zone.sound_mode),
        Resource::Receivers => unreachable!(),
    }
}
fn receivers_text(receivers: &[DiscoveredReceiver]) -> String {
    if receivers.is_empty() {
        return "No receivers found.\n".into();
    }
    receivers
        .iter()
        .enumerate()
        .map(|(index, receiver)| {
            format!(
                "{}. {} ({})\n",
                index + 1,
                receiver
                    .model
                    .as_deref()
                    .or(receiver.server.as_deref())
                    .unwrap_or("Denon receiver"),
                receiver.address.host
            )
        })
        .collect()
}

async fn run(operator: &dyn OperatorControl, command: Command) -> Result<Report, String> {
    match command {
        Command::Help | Command::Version => unreachable!("answered before the service is built"),
        Command::Get(Resource::Receivers, _) => Ok(Report {
            text: receivers_text(
                &operator
                    .discover(DEFAULT_DISCOVERY_TIMEOUT)
                    .await
                    .map_err(describe)?,
            ),
            warnings: Vec::new(),
        }),
        Command::Get(resource, selection) => query(operator, resource, selection).await,
        Command::Set {
            operation,
            value,
            selection,
            dry_run,
        } => set(operator, operation, value, selection, dry_run).await,
    }
}

async fn execute(command: Command) -> Result<(), String> {
    match command {
        Command::Help => print_usage(),
        Command::Version => println!("denon-avr-remote {VERSION}"),
        command => {
            let service = ControlService::new(
                Arc::new(X3800hConnector::default()),
                Arc::new(YamlConfigRepository::default()),
                Arc::new(SsdpDiscoveryAdapter),
                ServiceConfig::default(),
            );
            let operator = service.operator();
            let result = run(&*operator, command).await;
            // Closes the receiver connection, which frees the receiver's single
            // control connection for other tools.
            service.shutdown().await;
            let report = result?;
            print!("{}", report.text);
            for warning in &report.warnings {
                eprintln!("warning: {warning}");
            }
        }
    }
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
    use denon_avr_application::ports::{BoxFuture, OperationError, OperationErrorKind};
    use denon_avr_application::{
        OperationControl, OperationEvents, OperationStatus, OperatorAdmin, ReceiverReads,
        ReceiverSummary, StateSubscription,
    };
    use denon_avr_domain::{
        DispatchCertainty, Epoch, FrameSeq, HttpInformationSnapshot, MonotonicMillis,
        ObservationOrigin, OperationId, QuickSelectNameObservation, ReceiverObservation,
        SoundModeStatus, SourceCatalogObservation, SourceId, SystemPower,
    };
    use std::sync::Mutex;

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

    fn unexpected<T: Send + 'static>(
        what: &'static str,
    ) -> BoxFuture<'static, Result<T, ControlError>> {
        Box::pin(async move { panic!("the CLI must not call {what}") })
    }

    /// One answer the fake gives when asked about an operation.
    #[derive(Clone)]
    struct Step {
        status: OperationStatus,
        dispatch: DispatchCertainty,
        confirmed: bool,
        reason: Option<&'static str>,
    }

    fn step(
        status: OperationStatus,
        dispatch: DispatchCertainty,
        confirmed: bool,
        reason: Option<&'static str>,
    ) -> Step {
        Step {
            status,
            dispatch,
            confirmed,
            reason,
        }
    }

    /// A control service that records what the CLI asks of it.
    struct Fake {
        configuration: Mutex<ConfiguredReceivers>,
        saved: Mutex<Vec<ConfiguredReceivers>>,
        registered: Mutex<Vec<ReceiverIdentity>>,
        submitted: Mutex<Vec<(ReceiverId, ReceiverIntent)>>,
        waits: Mutex<Vec<Option<Duration>>>,
        /// Statuses `operation` returns, in order; the last one repeats.
        script: Mutex<Vec<Step>>,
        fail_save: bool,
        touched: Mutex<bool>,
    }

    impl Fake {
        fn new(configuration: ConfiguredReceivers) -> Self {
            Self {
                configuration: Mutex::new(configuration),
                saved: Mutex::new(Vec::new()),
                registered: Mutex::new(Vec::new()),
                submitted: Mutex::new(Vec::new()),
                waits: Mutex::new(Vec::new()),
                script: Mutex::new(vec![step(
                    OperationStatus::Completed,
                    DispatchCertainty::CompleteWrite,
                    true,
                    None,
                )]),
                fail_save: false,
                touched: Mutex::new(false),
            }
        }
        fn snapshot(
            &self,
            status: OperationStatus,
            dispatch: DispatchCertainty,
            confirmed: bool,
            reason: Option<&str>,
        ) -> OperationSnapshot {
            let (receiver, intent) = self.submitted.lock().unwrap()[0].clone();
            OperationSnapshot {
                id: OperationId(41),
                receiver,
                intent,
                status,
                dispatch,
                confirmed,
                reason: reason.map(str::to_owned),
                observation: confirmed.then(|| "post-dispatch receiver observation".to_owned()),
            }
        }
    }

    fn living_room_configuration() -> ConfiguredReceivers {
        ConfiguredReceivers {
            current: Some("living-room".into()),
            receivers: BTreeMap::from([(
                "living-room".into(),
                ReceiverIdentity {
                    host: "192.0.2.10".into(),
                    model: Some("AVR-X3800H".into()),
                    friendly_name: Some("Living Room".into()),
                },
            )]),
            ..ConfiguredReceivers::default()
        }
    }

    fn observed<T>(id: &ReceiverId, value: T) -> ReceiverObservation<T> {
        ReceiverObservation {
            receiver: id.clone(),
            epoch: Epoch(1),
            frame_seq: FrameSeq(1),
            observed_at: MonotonicMillis(0),
            origin: ObservationOrigin::ReceiverFrame,
            value,
        }
    }

    fn complete_state(id: &ReceiverId) -> ReceiverState {
        let mut state = ReceiverState::new(id.clone());
        state.establish_epoch(Epoch(1));
        let until = MonotonicMillis(10_000);
        state
            .system_power
            .observe(observed(id, SystemPower::On), until);
        state
            .main_zone
            .power
            .observe(observed(id, ZonePower::On), until);
        state
            .zone2_power
            .observe(observed(id, ZonePower::Off), until);
        state
            .main_zone
            .source
            .observe(observed(id, SourceId::new("CD").unwrap()), until);
        state.main_zone.volume.observe(
            observed(id, MasterVolume::db_half_steps(-78).unwrap()),
            until,
        );
        state
            .main_zone
            .mute
            .observe(observed(id, MuteState::Off), until);
        state.main_zone.sound_mode.observe(
            observed(
                id,
                SoundModeStatus {
                    id: "STEREO".into(),
                    raw: "STEREO".into(),
                },
            ),
            until,
        );
        state
    }

    impl ReceiverReads for Fake {
        fn receivers(&self) -> BoxFuture<'_, Result<Vec<ReceiverSummary>, ControlError>> {
            unexpected("receivers")
        }
        fn state<'a>(
            &'a self,
            receiver: &'a ReceiverId,
        ) -> BoxFuture<'a, Result<StateSubscription, ControlError>> {
            Box::pin(async move {
                *self.touched.lock().unwrap() = true;
                let (_keep, rx) = tokio::sync::watch::channel(complete_state(receiver));
                // The value stays readable after the sender is dropped.
                Ok(StateSubscription::new(rx))
            })
        }
        fn source_catalog<'a>(
            &'a self,
            _: &'a ReceiverId,
        ) -> BoxFuture<'a, Result<SourceCatalogObservation, ControlError>> {
            unexpected("source_catalog")
        }
    }

    impl OperationControl for Fake {
        fn submit<'a>(
            &'a self,
            receiver: &'a ReceiverId,
            submission: OperationSubmission,
        ) -> BoxFuture<'a, Result<OperationSnapshot, ControlError>> {
            Box::pin(async move {
                *self.touched.lock().unwrap() = true;
                self.submitted
                    .lock()
                    .unwrap()
                    .push((receiver.clone(), submission.intent));
                Ok(self.snapshot(
                    OperationStatus::Allowed,
                    DispatchCertainty::NotDispatched,
                    false,
                    None,
                ))
            })
        }
        fn operation(
            &self,
            _: OperationId,
            wait: Option<Duration>,
        ) -> BoxFuture<'_, Result<OperationSnapshot, ControlError>> {
            Box::pin(async move {
                self.waits.lock().unwrap().push(wait);
                let mut script = self.script.lock().unwrap();
                let next = if script.len() > 1 {
                    script.remove(0)
                } else {
                    script[0].clone()
                };
                Ok(self.snapshot(next.status, next.dispatch, next.confirmed, next.reason))
            })
        }
        fn cancel(&self, _: OperationId) -> BoxFuture<'_, Result<OperationSnapshot, ControlError>> {
            unexpected("cancel")
        }
        fn operation_events(&self) -> BoxFuture<'_, Result<OperationEvents, ControlError>> {
            unexpected("operation_events")
        }
    }

    impl OperatorAdmin for Fake {
        fn discover(
            &self,
            _: Duration,
        ) -> BoxFuture<'_, Result<Vec<DiscoveredReceiver>, ControlError>> {
            Box::pin(async move {
                Ok(vec![DiscoveredReceiver {
                    address: denon_avr_domain::ReceiverEndpoint {
                        host: "192.0.2.77".into(),
                        port: 80,
                    },
                    location: None,
                    server: None,
                    model: Some("AVR-X3800H".into()),
                    search_target: None,
                    unique_service_name: None,
                }])
            })
        }
        fn register_ad_hoc(
            &self,
            identity: ReceiverIdentity,
        ) -> BoxFuture<'_, Result<ReceiverId, ControlError>> {
            Box::pin(async move {
                *self.touched.lock().unwrap() = true;
                let id = ReceiverId::ad_hoc(&identity.host).unwrap();
                self.registered.lock().unwrap().push(identity);
                Ok(id)
            })
        }
        fn configuration(&self) -> BoxFuture<'_, Result<ConfiguredReceivers, ControlError>> {
            Box::pin(async move {
                *self.touched.lock().unwrap() = true;
                Ok(self.configuration.lock().unwrap().clone())
            })
        }
        fn save_configuration<'a>(
            &'a self,
            configuration: &'a ConfiguredReceivers,
        ) -> BoxFuture<'a, Result<(), ControlError>> {
            Box::pin(async move {
                if self.fail_save {
                    return Err(ControlError::Receiver(OperationError::new(
                        OperationErrorKind::Configuration,
                        "saving",
                        "disk full",
                    )));
                }
                self.saved.lock().unwrap().push(configuration.clone());
                *self.configuration.lock().unwrap() = configuration.clone();
                Ok(())
            })
        }
        fn quick_select_names<'a>(
            &'a self,
            _: &'a ReceiverId,
        ) -> BoxFuture<'a, Result<QuickSelectNameObservation, ControlError>> {
            unexpected("quick_select_names")
        }
        fn http_information<'a>(
            &'a self,
            _: &'a ReceiverId,
        ) -> BoxFuture<'a, Result<HttpInformationSnapshot, ControlError>> {
            unexpected("http_information")
        }
        fn refresh<'a>(
            &'a self,
            _: &'a ReceiverId,
        ) -> BoxFuture<'a, Result<denon_avr_application::Readiness, ControlError>> {
            unexpected("refresh")
        }
    }

    fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn volume_set(selection: Selection) -> (Operation, String, Selection, bool) {
        (Operation::Volume, "-39".into(), selection, false)
    }

    fn run_set(fake: &Fake, selection: Selection) -> Result<Report, String> {
        let (operation, value, selection, dry_run) = volume_set(selection);
        block_on(set(fake, operation, value, selection, dry_run))
    }

    #[test]
    fn a_set_waits_until_the_operation_is_finished_and_prints_status_dispatch_and_confirmed() {
        let fake = Fake::new(living_room_configuration());
        *fake.script.lock().unwrap() = vec![
            step(
                OperationStatus::InSession,
                DispatchCertainty::NotDispatched,
                false,
                None,
            ),
            step(
                OperationStatus::InSession,
                DispatchCertainty::NotDispatched,
                false,
                None,
            ),
            step(
                OperationStatus::Completed,
                DispatchCertainty::CompleteWrite,
                true,
                None,
            ),
        ];
        let report = run_set(&fake, Selection::default()).unwrap();

        assert!(
            report.text.contains("Outcome: completed\n"),
            "{}",
            report.text
        );
        assert!(report.text.contains("Dispatch: complete_write\n"));
        assert!(report.text.contains("Confirmed: true\n"));
        assert!(report
            .text
            .contains("Observation: post-dispatch receiver observation"));
        // It asked again after each unfinished answer, with a bounded wait.
        let waits = fake.waits.lock().unwrap();
        assert_eq!(waits.len(), 3);
        assert!(waits.iter().all(|wait| *wait == Some(OPERATION_WAIT)));
        // One submission, to the saved receiver's id, with no id of the CLI's own.
        let submitted = fake.submitted.lock().unwrap();
        assert_eq!(submitted.len(), 1);
        assert_eq!(submitted[0].0.as_str(), "living-room");
        assert_eq!(
            submitted[0].1,
            ReceiverIntent::Volume(MasterVolume::db_half_steps(-78).unwrap())
        );
    }

    #[test]
    fn a_rejection_is_reported_as_not_dispatched_and_not_confirmed() {
        let fake = Fake::new(living_room_configuration());
        *fake.script.lock().unwrap() = vec![step(
            OperationStatus::Rejected,
            DispatchCertainty::NotDispatched,
            false,
            Some("preflight failed: timeout"),
        )];
        let report = run_set(&fake, Selection::default()).unwrap();
        assert!(report.text.contains("Outcome: rejected\n"));
        assert!(report.text.contains("Dispatch: not_dispatched\n"));
        assert!(report.text.contains("Confirmed: false\n"));
        assert!(report.text.contains("Reason: preflight failed: timeout\n"));
        assert!(!report.text.contains("Observation:"));
    }

    #[test]
    fn an_ambiguous_write_is_not_called_confirmed() {
        let fake = Fake::new(living_room_configuration());
        *fake.script.lock().unwrap() = vec![step(
            OperationStatus::Indeterminate,
            DispatchCertainty::CompleteWrite,
            false,
            Some("requested value was not observed within the confirmation window"),
        )];
        let report = run_set(&fake, Selection::default()).unwrap();
        assert!(report.text.contains("Outcome: indeterminate\n"));
        assert!(report.text.contains("Dispatch: complete_write\n"));
        assert!(report.text.contains("Confirmed: false\n"));
    }

    #[test]
    fn a_dry_run_never_touches_the_service() {
        let fake = Fake::new(living_room_configuration());
        let report = block_on(set(
            &fake,
            Operation::Volume,
            "-39".into(),
            Selection::default(),
            true,
        ))
        .unwrap();
        assert!(report.text.starts_with("Dry run: would submit"));
        assert!(!*fake.touched.lock().unwrap());
        assert!(fake.submitted.lock().unwrap().is_empty());
    }

    #[test]
    fn a_host_selector_registers_an_ad_hoc_receiver_and_submits_to_it() {
        let fake = Fake::new(ConfiguredReceivers::default());
        run_set(
            &fake,
            Selection {
                host: Some("192.0.2.50".into()),
                receiver: None,
            },
        )
        .unwrap();
        assert_eq!(fake.registered.lock().unwrap()[0].host, "192.0.2.50");
        let submitted = fake.submitted.lock().unwrap();
        assert!(submitted[0].0.is_ad_hoc());
        assert_eq!(submitted[0].0.as_str(), "adhoc:192.0.2.50");
    }

    #[test]
    fn a_numbered_selector_uses_discovery_and_rejects_an_unknown_number() {
        let fake = Fake::new(ConfiguredReceivers::default());
        run_set(
            &fake,
            Selection {
                host: None,
                receiver: Some(0),
            },
        )
        .unwrap();
        assert_eq!(fake.registered.lock().unwrap()[0].host, "192.0.2.77");
        let error = run_set(
            &fake,
            Selection {
                host: None,
                receiver: Some(5),
            },
        )
        .unwrap_err();
        assert_eq!(error, "receiver selection is out of range");
    }

    #[test]
    fn without_a_saved_receiver_or_selector_it_fails_rather_than_guessing() {
        let fake = Fake::new(ConfiguredReceivers::default());
        let error = run_set(&fake, Selection::default()).unwrap_err();
        assert_eq!(error, "a saved receiver or explicit selector is required");
        assert!(fake.submitted.lock().unwrap().is_empty());
    }

    #[test]
    fn a_saved_receiver_keeps_its_entry_name_and_its_favorites() {
        let mut configuration = living_room_configuration();
        configuration
            .toggle_current_sound_mode_favorite("DTS NEURAL:X")
            .unwrap();
        let fake = Fake::new(configuration);
        run_set(&fake, Selection::default()).unwrap();

        let saved = fake.saved.lock().unwrap();
        assert_eq!(saved.len(), 1);
        // The friendly name is "Living Room", but the entry name is the id.
        assert_eq!(saved[0].current.as_deref(), Some("living-room"));
        assert!(saved[0].receivers.contains_key("living-room"));
        assert!(saved[0].is_sound_mode_favorite("DTS NEURAL:X"));
        saved[0].validate().unwrap();
    }

    #[test]
    fn a_configuration_with_several_receivers_is_never_rewritten() {
        let mut configuration = living_room_configuration();
        configuration
            .receivers
            .insert("den".into(), ReceiverIdentity::ad_hoc("192.0.2.11"));
        let fake = Fake::new(configuration);
        run_set(&fake, Selection::default()).unwrap();
        run_set(
            &fake,
            Selection {
                host: Some("192.0.2.50".into()),
                receiver: None,
            },
        )
        .unwrap();
        assert!(fake.saved.lock().unwrap().is_empty());
    }

    #[test]
    fn a_failed_save_is_a_warning_and_never_hides_the_outcome() {
        let mut fake = Fake::new(living_room_configuration());
        fake.fail_save = true;
        let report = run_set(&fake, Selection::default()).unwrap();
        assert!(report.text.contains("Outcome: completed"));
        assert_eq!(report.warnings.len(), 1);
        assert!(
            report.warnings[0].contains("disk full"),
            "{:?}",
            report.warnings
        );
    }

    #[test]
    fn a_read_prints_the_target_the_value_and_readiness() {
        let fake = Fake::new(living_room_configuration());
        let report = block_on(query(&fake, Resource::Volume, Selection::default())).unwrap();
        assert!(report
            .text
            .starts_with("Receiver: Living Room (192.0.2.10)\n"));
        assert!(report.text.contains("Volume:"));
        assert!(
            report.text.ends_with("Readiness: ready\n"),
            "{}",
            report.text
        );
    }

    #[test]
    fn readiness_is_degraded_when_a_core_field_has_no_usable_value() {
        let id = ReceiverId::new("living-room").unwrap();
        let mut state = complete_state(&id);
        assert_eq!(readiness_line(&state), "Readiness: ready\n");
        state
            .main_zone
            .volume
            .stale(denon_avr_domain::StaleReason::QueryFailed, "timeout");
        assert!(readiness_line(&state).starts_with("Readiness: degraded"));
        let empty = ReceiverState::new(id);
        assert!(readiness_line(&empty).starts_with("Readiness: degraded"));
    }

    #[test]
    fn a_superseded_operation_names_the_operation_that_replaced_it() {
        let snapshot = OperationSnapshot {
            id: OperationId(3),
            receiver: ReceiverId::new("living-room").unwrap(),
            intent: ReceiverIntent::Mute(MuteState::On),
            status: OperationStatus::Superseded { by: OperationId(4) },
            dispatch: DispatchCertainty::NotDispatched,
            confirmed: false,
            reason: None,
            observation: None,
        };
        assert!(outcome_text(&snapshot).starts_with("Outcome: superseded (by operation 4)\n"));
    }

    #[test]
    fn receivers_are_numbered_from_one() {
        let fake = Fake::new(ConfiguredReceivers::default());
        let report = block_on(run(
            &fake,
            Command::Get(Resource::Receivers, Selection::default()),
        ))
        .unwrap();
        assert_eq!(report.text, "1. AVR-X3800H (192.0.2.77)\n");
        assert_eq!(receivers_text(&[]), "No receivers found.\n");
    }
}
