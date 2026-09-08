//! Safe, capability-gated Main Zone control use cases.

use crate::application::ports::{
    AsyncControlGateway, AsyncStatusGateway, ControlGateway, OperationError, OperationErrorKind,
    StatusGateway,
};
use crate::domain::{MainZoneControl, MainZoneSnapshot, ModelCapabilities};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlOutcome {
    Confirmed(MainZoneSnapshot),
    NoOp(MainZoneSnapshot),
    Rejected(OperationError),
    Unconfirmed(OperationError),
    Unsupported(String),
    TransportFailure(OperationError),
}

/// One-shot admission policy used by the CLI. It performs the authoritative
/// preflight and emits at most one state-changing dispatch; confirmation is
/// intentionally left to a later status query.
pub fn dispatch_main_zone_control(
    status: &mut (impl StatusGateway + ControlGateway),
    capabilities: &ModelCapabilities,
    control: MainZoneControl,
    expected_version: u64,
) -> ControlOutcome {
    let preflight = crate::application::main_zone_status::query_main_zone_status(status);
    if preflight.resource_version() != expected_version {
        return ControlOutcome::Rejected(OperationError::new(
            OperationErrorKind::Conflict,
            "checking resource version",
            format!(
                "expected {expected_version}, current {}",
                preflight.resource_version()
            ),
        ));
    }
    if !capabilities.supports_control(&control) {
        return ControlOutcome::Unsupported(
            "the selected receiver does not support this validated control".into(),
        );
    }
    if preflight
        .value(control_field(&control))
        .is_some_and(|value| control_matches(&control, &value, capabilities))
    {
        return ControlOutcome::NoOp(preflight);
    }
    match status.execute_once(control) {
        Ok(()) => ControlOutcome::Unconfirmed(OperationError::new(
            OperationErrorKind::Unavailable,
            "confirming control",
            "dispatched once; confirmation is deferred to a later status query",
        )),
        Err(error) => ControlOutcome::TransportFailure(error),
    }
}

pub fn execute_main_zone_control(
    status: &mut (impl StatusGateway + ControlGateway),
    capabilities: &ModelCapabilities,
    control: MainZoneControl,
    expected_version: u64,
) -> ControlOutcome {
    let preflight = crate::application::main_zone_status::query_main_zone_status(status);
    if preflight.resource_version() != expected_version {
        return ControlOutcome::Rejected(OperationError::new(
            OperationErrorKind::Conflict,
            "checking resource version",
            format!(
                "expected {expected_version}, current {}",
                preflight.resource_version()
            ),
        ));
    }
    if !capabilities.supports_control(&control) {
        return ControlOutcome::Unsupported(
            "the selected receiver does not support this validated control".into(),
        );
    }
    if matches!(preflight.value(control_field(&control)), Some(ref value) if control_matches(&control, value, capabilities))
    {
        return ControlOutcome::NoOp(preflight);
    }
    if let Err(error) = status.execute_once(control.clone()) {
        return ControlOutcome::TransportFailure(error);
    }
    if matches!(
        control,
        MainZoneControl::Power(crate::domain::PowerState::On)
    ) {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    let confirmed = crate::application::main_zone_status::query_main_zone_status(status);
    if confirmed
        .value(control_field(&control))
        .is_some_and(|value| control_matches(&control, &value, capabilities))
    {
        ControlOutcome::Confirmed(confirmed)
    } else {
        ControlOutcome::Unconfirmed(OperationError::new(
            OperationErrorKind::Unavailable,
            "confirming control command",
            "receiver did not report the requested value",
        ))
    }
}

pub async fn execute_main_zone_control_async(
    status: &mut (impl AsyncStatusGateway + AsyncControlGateway),
    capabilities: &ModelCapabilities,
    control: MainZoneControl,
    expected_version: u64,
) -> ControlOutcome {
    let preflight =
        crate::application::main_zone_status::query_main_zone_status_async(status).await;
    if preflight.resource_version() != expected_version {
        return ControlOutcome::Rejected(OperationError::new(
            OperationErrorKind::Conflict,
            "checking resource version",
            format!(
                "expected {expected_version}, current {}",
                preflight.resource_version()
            ),
        ));
    }
    if !capabilities.supports_control(&control) {
        return ControlOutcome::Unsupported(
            "the selected receiver does not support this validated control".into(),
        );
    }
    if matches!(preflight.value(control_field(&control)), Some(ref value) if control_matches(&control, value, capabilities))
    {
        return ControlOutcome::NoOp(preflight);
    }
    if let Err(error) = status.execute_once(control.clone()).await {
        return ControlOutcome::TransportFailure(error);
    }
    if matches!(
        control,
        MainZoneControl::Power(crate::domain::PowerState::On)
    ) {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    let confirmed =
        crate::application::main_zone_status::query_main_zone_status_async(status).await;
    if confirmed
        .value(control_field(&control))
        .is_some_and(|value| control_matches(&control, &value, capabilities))
    {
        ControlOutcome::Confirmed(confirmed)
    } else {
        ControlOutcome::Unconfirmed(OperationError::new(
            OperationErrorKind::Unavailable,
            "confirming control command",
            "receiver did not report the requested value",
        ))
    }
}

fn control_field(control: &MainZoneControl) -> crate::domain::MainZoneField {
    match control {
        MainZoneControl::Power(_) => crate::domain::MainZoneField::Power,
        MainZoneControl::Input(_) => crate::domain::MainZoneField::Input,
        MainZoneControl::Volume(_) => crate::domain::MainZoneField::Volume,
        MainZoneControl::Mute(_) => crate::domain::MainZoneField::Mute,
        MainZoneControl::SurroundMode(_) => crate::domain::MainZoneField::SurroundMode,
        MainZoneControl::ListeningModeGroup(_) => crate::domain::MainZoneField::SurroundMode,
    }
}

fn control_matches(
    control: &MainZoneControl,
    value: &crate::domain::MainZoneValue,
    capabilities: &ModelCapabilities,
) -> bool {
    match (control, value) {
        (MainZoneControl::Power(expected), crate::domain::MainZoneValue::Power(actual)) => {
            expected == actual
        }
        (MainZoneControl::Input(expected), crate::domain::MainZoneValue::Input(actual)) => {
            expected == actual
        }
        (MainZoneControl::Volume(expected), crate::domain::MainZoneValue::Volume(actual)) => {
            actual.level().ok().as_ref() == Some(expected)
        }
        (MainZoneControl::Mute(expected), crate::domain::MainZoneValue::Mute(actual)) => {
            expected == actual
        }
        (
            MainZoneControl::SurroundMode(expected),
            crate::domain::MainZoneValue::SurroundMode(actual),
        ) => expected == actual,
        (
            MainZoneControl::ListeningModeGroup(group),
            crate::domain::MainZoneValue::SurroundMode(actual),
        ) => capabilities
            .listening_modes(*group)
            .contains(&actual.as_str()),
        _ => false,
    }
}
