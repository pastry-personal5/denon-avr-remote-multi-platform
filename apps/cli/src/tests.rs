//! Tests for the CLI: the argument parser, and the commands against a fake
//! control service that records what the CLI asks of it.

use super::args::{parse_args, Command, Operation, Resource, Selection};
use super::commands::{parse_intent, parse_volume, query, run, set, OPERATION_WAIT};
use super::render::{outcome_text, readiness_line, receivers_text, Report};
use denon_avr_application::ports::{BoxFuture, OperationError, OperationErrorKind};
use denon_avr_application::{ControlError, OperationSnapshot, OperationSubmission};
use denon_avr_application::{
    OperationControl, OperationEvents, OperationStatus, OperatorAdmin, ReceiverReads,
    ReceiverSummary, StateSubscription,
};
use denon_avr_domain::{
    ConfiguredReceivers, DiscoveredReceiver, MasterVolume, MuteState, ReceiverId, ReceiverIdentity,
    ReceiverIntent, ReceiverState, ZonePower,
};
use denon_avr_domain::{
    DispatchCertainty, Epoch, FrameSeq, HttpInformationSnapshot, MonotonicMillis,
    ObservationOrigin, OperationId, QuickSelectNameObservation, ReceiverObservation,
    SoundModeStatus, SourceCatalogObservation, SourceId, SystemPower,
};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

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
        parse_args(["set", "volume", "-20", "--resource-version", "1"].map(str::to_owned)).is_err()
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
    fn health(&self) -> BoxFuture<'_, Result<denon_avr_application::ServiceHealth, ControlError>> {
        unexpected("health")
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
    fn dry_run<'a>(
        &'a self,
        _: &'a ReceiverId,
        _: ReceiverIntent,
    ) -> BoxFuture<'a, Result<denon_avr_application::DryRun, ControlError>> {
        unexpected("dry_run")
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
    fn dry_run_as<'a>(
        &'a self,
        _: denon_avr_application::AgentLabel,
        _: &'a ReceiverId,
        _: ReceiverIntent,
    ) -> BoxFuture<'a, Result<denon_avr_application::DryRun, ControlError>> {
        unexpected("dry_run_as")
    }
    fn policy(&self) -> BoxFuture<'_, Result<denon_avr_application::PolicyView, ControlError>> {
        unexpected("policy")
    }
    fn reload_policy(
        &self,
    ) -> BoxFuture<'_, Result<denon_avr_application::PolicyView, ControlError>> {
        unexpected("reload_policy")
    }
    fn audit(
        &self,
        _: denon_avr_application::AuditQuery,
    ) -> BoxFuture<'_, Result<denon_avr_application::AuditPage, ControlError>> {
        unexpected("audit")
    }
    fn issue_token(
        &self,
        _: denon_avr_application::AgentLabel,
    ) -> BoxFuture<'_, Result<denon_avr_application::IssuedToken, ControlError>> {
        unexpected("issue_token")
    }
    fn tokens(
        &self,
    ) -> BoxFuture<'_, Result<Vec<denon_avr_application::TokenRecord>, ControlError>> {
        unexpected("tokens")
    }
    fn revoke_token(
        &self,
        _: denon_avr_application::TokenId,
    ) -> BoxFuture<'_, Result<denon_avr_application::TokenRecord, ControlError>> {
        unexpected("revoke_token")
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
    let report = block_on(run(&fake, Command::Receivers)).unwrap();
    assert_eq!(report.text, "1. AVR-X3800H (192.0.2.77)\n");
    assert_eq!(receivers_text(&[]), "No receivers found.\n");
}

fn parse(args: &[&str]) -> Result<Command, String> {
    parse_args(args.iter().map(|arg| (*arg).to_owned()))
}

fn err(message: &str) -> Result<Command, String> {
    Err(message.to_owned())
}

fn host(host: &str) -> Selection {
    Selection {
        receiver: None,
        host: Some(host.into()),
    }
}

fn receiver(index: usize) -> Selection {
    Selection {
        receiver: Some(index),
        host: None,
    }
}

fn set_command(operation: Operation, value: &str, selection: Selection, dry_run: bool) -> Command {
    Command::Set {
        operation,
        value: value.into(),
        selection,
        dry_run,
    }
}

#[test]
fn parse_no_arguments_is_help() {
    assert_eq!(parse(&[]), Ok(Command::Help));
}

#[test]
fn parse_help_and_version_words() {
    for word in ["help", "--help", "-h"] {
        assert_eq!(parse(&[word]), Ok(Command::Help), "{word}");
        assert_eq!(
            parse(&[word, "extra"]),
            Err(format!("unknown command '{word}'; expected get or set")),
            "{word}"
        );
    }
    for word in ["version", "--version", "-V"] {
        assert_eq!(parse(&[word]), Ok(Command::Version), "{word}");
        assert_eq!(
            parse(&[word, "extra"]),
            Err(format!("unknown command '{word}'; expected get or set")),
            "{word}"
        );
    }
}

#[test]
fn parse_unknown_command() {
    assert_eq!(
        parse(&["frob"]),
        err("unknown command 'frob'; expected get or set")
    );
    assert_eq!(
        parse(&["frob", "status"]),
        err("unknown command 'frob'; expected get or set")
    );
}

#[test]
fn parse_get_resources_and_aliases() {
    assert_eq!(parse(&["get"]), err("get requires a resource"));
    for (word, resource) in [
        ("status", Resource::Status),
        ("power", Resource::Power),
        ("input", Resource::Input),
        ("volume", Resource::Volume),
        ("level", Resource::Volume),
        ("mute", Resource::Mute),
        ("surround", Resource::Surround),
        ("surround-mode", Resource::Surround),
    ] {
        assert_eq!(
            parse(&["get", word]),
            Ok(Command::Get(resource, Selection::default())),
            "{word}"
        );
    }
    assert_eq!(parse(&["get", "bogus"]), err("unknown resource 'bogus'"));
    assert_eq!(parse(&["get", "Status"]), err("unknown resource 'Status'"));
    // The resource word is checked before any selector.
    assert_eq!(
        parse(&["get", "bogus", "--receiver", "0"]),
        err("unknown resource 'bogus'")
    );
    assert_eq!(
        parse(&["get", "status", "extra"]),
        err("unexpected argument 'extra'")
    );
}

#[test]
fn parse_get_receivers() {
    assert_eq!(parse(&["get", "receivers"]), Ok(Command::Receivers));
    assert_eq!(
        parse(&["get", "receivers", "--host", "192.0.2.5"]),
        err("get receivers does not accept a selector")
    );
    assert_eq!(
        parse(&["get", "receivers", "--receiver", "1"]),
        err("get receivers does not accept a selector")
    );
    // Selector options are validated before the receivers check.
    assert_eq!(
        parse(&["get", "receivers", "--receiver", "0"]),
        err("--receiver requires a positive number")
    );
    assert_eq!(
        parse(&["get", "receivers", "--host"]),
        err("--host requires a non-empty address")
    );
    assert_eq!(
        parse(&["get", "receivers", "--host", "a", "--receiver", "1"]),
        err("--host and --receiver cannot be combined")
    );
    assert_eq!(
        parse(&["get", "receivers", "extra"]),
        err("unexpected argument 'extra'")
    );
}

#[test]
fn parse_get_selectors() {
    assert_eq!(
        parse(&["get", "status", "--host", "192.0.2.5"]),
        Ok(Command::Get(Resource::Status, host("192.0.2.5")))
    );
    assert_eq!(
        parse(&["get", "status", "--receiver", "2"]),
        Ok(Command::Get(Resource::Status, receiver(1)))
    );
    assert_eq!(
        parse(&["get", "status", "--host", "h", "--receiver", "2"]),
        err("--host and --receiver cannot be combined")
    );
    assert_eq!(
        parse(&["get", "status", "--receiver", "2", "--host", "h"]),
        err("--host and --receiver cannot be combined")
    );
    assert_eq!(
        parse(&["get", "status", "--host"]),
        err("--host requires a non-empty address")
    );
    assert_eq!(
        parse(&["get", "status", "--host", ""]),
        err("--host requires a non-empty address")
    );
    assert_eq!(
        parse(&["get", "status", "--host", "  "]),
        err("--host requires a non-empty address")
    );
    assert_eq!(
        parse(&["get", "status", "--receiver", "0"]),
        err("--receiver requires a positive number")
    );
    assert_eq!(
        parse(&["get", "status", "--receiver", "x"]),
        err("--receiver requires a positive number")
    );
    assert_eq!(
        parse(&["get", "status", "--receiver"]),
        err("--receiver requires a positive number")
    );
    assert_eq!(
        parse(&["get", "status", "--host", "a", "--host", "b"]),
        err("--host may be specified only once")
    );
    assert_eq!(
        parse(&["get", "status", "--receiver", "1", "--receiver", "2"]),
        err("--receiver may be specified only once")
    );
}

#[test]
fn parse_set_operations_and_aliases() {
    assert_eq!(parse(&["set"]), err("set requires an operation"));
    assert_eq!(
        parse(&["set", "bogus", "1"]),
        err("unknown set operation 'bogus'")
    );
    assert_eq!(
        parse(&["set", "receivers", "1"]),
        err("unknown set operation 'receivers'")
    );
    assert_eq!(
        parse(&["set", "status", "1"]),
        err("unknown set operation 'status'")
    );
    for (word, operation) in [
        ("power", Operation::Power),
        ("input", Operation::Input),
        ("volume", Operation::Volume),
        ("level", Operation::Volume),
        ("mute", Operation::Mute),
        ("surround", Operation::Surround),
        ("surround-mode", Operation::Surround),
    ] {
        assert_eq!(parse(&["set", word]), err("set requires a value"), "{word}");
        assert_eq!(
            parse(&["set", word, "v"]),
            Ok(set_command(operation, "v", Selection::default(), false)),
            "{word}"
        );
    }
    // The value is taken positionally, whatever it looks like.
    assert_eq!(
        parse(&["set", "volume", "--dry-run"]),
        Ok(set_command(
            Operation::Volume,
            "--dry-run",
            Selection::default(),
            false
        ))
    );
}

#[test]
fn parse_set_options() {
    assert_eq!(
        parse(&["set", "volume", "-20", "--dry-run"]),
        Ok(set_command(
            Operation::Volume,
            "-20",
            Selection::default(),
            true
        ))
    );
    assert_eq!(
        parse(&["set", "volume", "-20", "--dry-run", "--dry-run"]),
        err("--dry-run may be specified only once")
    );
    assert_eq!(
        parse(&["set", "volume", "-20", "--resource-version", "1"]),
        err("--resource-version was removed; receiver state has no compare-and-set token")
    );
    assert_eq!(
        parse(&["set", "volume", "-20", "--frob"]),
        err("unexpected argument '--frob'")
    );
    assert_eq!(
        parse(&["set", "mute", "on", "--host", "h", "--dry-run"]),
        Ok(set_command(Operation::Mute, "on", host("h"), true))
    );
    assert_eq!(
        parse(&["set", "power", "on", "--receiver", "3"]),
        Ok(set_command(Operation::Power, "on", receiver(2), false))
    );
    assert_eq!(
        parse(&["set", "power", "on", "--host", "h", "--receiver", "3"]),
        err("--host and --receiver cannot be combined")
    );
    assert_eq!(
        parse(&["set", "power", "on", "--host"]),
        err("--host requires a non-empty address")
    );
    assert_eq!(
        parse(&["set", "power", "on", "--receiver", "0"]),
        err("--receiver requires a positive number")
    );
    assert_eq!(
        parse(&["set", "power", "on", "--host", "a", "--host", "b"]),
        err("--host may be specified only once")
    );
}
