//! The get and set commands over the operator port: reading a resource,
//! listing receivers, and submitting an intent and waiting for its outcome.

use crate::args::{Command, Operation, Resource, Selection};
use crate::render::{
    outcome_text, readiness_line, receivers_text, render_resource, target_line, Report,
};
use crate::target::{describe, remember_or_warn, resolve_target};
use denon_avr_application::{OperationSubmission, OperatorControl};
use denon_avr_domain::{MasterVolume, MuteState, ReceiverIntent, SoundModeIntent, ZonePower};
use denon_avr_infrastructure::discovery_ssdp::DEFAULT_DISCOVERY_TIMEOUT;
use std::time::Duration;

/// How long one wait for an operation may last. The service caps it as well, so
/// the loop below asks again until the operation is finished.
pub(super) const OPERATION_WAIT: Duration = Duration::from_secs(30);

pub(super) async fn query(
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

pub(super) async fn set(
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

pub(super) fn parse_intent(operation: Operation, value: &str) -> Result<ReceiverIntent, String> {
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
pub(super) fn parse_volume(raw: &str) -> Result<MasterVolume, String> {
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

pub(super) async fn run(
    operator: &dyn OperatorControl,
    command: Command,
) -> Result<Report, String> {
    match command {
        Command::Help | Command::Version => unreachable!("answered before the service is built"),
        Command::Receivers => Ok(Report {
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
