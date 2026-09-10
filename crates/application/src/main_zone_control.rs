//! Safe, capability-gated Main Zone control use cases.

use crate::ports::{
    AsyncControlGateway, AsyncStatusGateway, ControlGateway, OperationError, OperationErrorKind,
    StatusGateway,
};
use denon_avr_domain::{MainZoneControl, MainZoneSnapshot, ModelCapabilities};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlOutcome {
    Confirmed(MainZoneSnapshot),
    NoOp(MainZoneSnapshot),
    Rejected(OperationError),
    Unconfirmed(OperationError),
    Unsupported(String),
    TransportFailure(OperationError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlAdmission {
    Dispatch,
    NoOp,
    Rejected(OperationError),
    Unsupported(String),
}

/// Evaluates optimistic concurrency, capability support, and idempotence from
/// an already-authoritative snapshot without performing I/O. Delivery layers
/// use this for previews; dispatch use cases apply it immediately before the
/// single permitted write.
pub fn admit_main_zone_control(
    preflight: &MainZoneSnapshot,
    capabilities: &ModelCapabilities,
    control: &MainZoneControl,
    expected_version: Option<u64>,
) -> ControlAdmission {
    if let Some(expected_version) = expected_version {
        if preflight.resource_version() != expected_version {
            return ControlAdmission::Rejected(OperationError::new(
                OperationErrorKind::Conflict,
                "checking resource version",
                format!(
                    "expected {expected_version}, current {}",
                    preflight.resource_version()
                ),
            ));
        }
    }
    if !capabilities.supports_control(control) {
        return ControlAdmission::Unsupported(
            "the selected receiver does not support this validated control".into(),
        );
    }
    if preflight
        .value(control_field(control))
        .is_some_and(|value| control_matches(control, &value, capabilities))
    {
        ControlAdmission::NoOp
    } else {
        ControlAdmission::Dispatch
    }
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
    let preflight = crate::main_zone_status::query_main_zone_status(status);
    match admit_main_zone_control(&preflight, capabilities, &control, Some(expected_version)) {
        ControlAdmission::Dispatch => {}
        ControlAdmission::NoOp => return ControlOutcome::NoOp(preflight),
        ControlAdmission::Rejected(error) => return ControlOutcome::Rejected(error),
        ControlAdmission::Unsupported(message) => return ControlOutcome::Unsupported(message),
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
    let preflight = crate::main_zone_status::query_main_zone_status(status);
    match admit_main_zone_control(&preflight, capabilities, &control, Some(expected_version)) {
        ControlAdmission::Dispatch => {}
        ControlAdmission::NoOp => return ControlOutcome::NoOp(preflight),
        ControlAdmission::Rejected(error) => return ControlOutcome::Rejected(error),
        ControlAdmission::Unsupported(message) => return ControlOutcome::Unsupported(message),
    }
    if let Err(error) = status.execute_once(control.clone()) {
        return ControlOutcome::TransportFailure(error);
    }
    if matches!(
        control,
        MainZoneControl::Power(denon_avr_domain::PowerState::On)
    ) {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    let confirmed = crate::main_zone_status::query_main_zone_status(status);
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
    let preflight = crate::main_zone_status::query_main_zone_status_async(status).await;
    match admit_main_zone_control(&preflight, capabilities, &control, Some(expected_version)) {
        ControlAdmission::Dispatch => {}
        ControlAdmission::NoOp => return ControlOutcome::NoOp(preflight),
        ControlAdmission::Rejected(error) => return ControlOutcome::Rejected(error),
        ControlAdmission::Unsupported(message) => return ControlOutcome::Unsupported(message),
    }
    if let Err(error) = status.execute_once(control.clone()).await {
        return ControlOutcome::TransportFailure(error);
    }
    if matches!(
        control,
        MainZoneControl::Power(denon_avr_domain::PowerState::On)
    ) {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    let confirmed = crate::main_zone_status::query_main_zone_status_async(status).await;
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

/// Maps a control to the authoritative Main Zone field used for preflight and
/// confirmation. Shared by synchronous, asynchronous, and coordinated flows.
pub(crate) fn control_field(control: &MainZoneControl) -> denon_avr_domain::MainZoneField {
    match control {
        MainZoneControl::Power(_) => denon_avr_domain::MainZoneField::Power,
        MainZoneControl::Input(_) => denon_avr_domain::MainZoneField::Input,
        MainZoneControl::Volume(_) => denon_avr_domain::MainZoneField::Volume,
        MainZoneControl::Mute(_) => denon_avr_domain::MainZoneField::Mute,
        MainZoneControl::SurroundMode(_) => denon_avr_domain::MainZoneField::SurroundMode,
        MainZoneControl::ListeningModeGroup(_) => denon_avr_domain::MainZoneField::SurroundMode,
    }
}

pub(crate) fn control_matches(
    control: &MainZoneControl,
    value: &denon_avr_domain::MainZoneValue,
    capabilities: &ModelCapabilities,
) -> bool {
    match (control, value) {
        (MainZoneControl::Power(expected), denon_avr_domain::MainZoneValue::Power(actual)) => {
            expected == actual
        }
        (MainZoneControl::Input(expected), denon_avr_domain::MainZoneValue::Input(actual)) => {
            expected == actual
        }
        (MainZoneControl::Volume(expected), denon_avr_domain::MainZoneValue::Volume(actual)) => {
            actual.level().ok().as_ref() == Some(expected)
        }
        (MainZoneControl::Mute(expected), denon_avr_domain::MainZoneValue::Mute(actual)) => {
            expected == actual
        }
        (
            MainZoneControl::SurroundMode(expected),
            denon_avr_domain::MainZoneValue::SurroundMode(actual),
        ) => expected == actual,
        (
            MainZoneControl::ListeningModeGroup(group),
            denon_avr_domain::MainZoneValue::SurroundMode(actual),
        ) => capabilities
            .listening_modes(*group)
            .contains(&actual.as_str()),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use denon_avr_domain::{
        ListeningModeGroup, MainZoneValue, Model, StateAuthority, SurroundMode,
    };

    #[test]
    fn admission_does_not_treat_every_listening_group_as_a_no_op() {
        let mut snapshot = MainZoneSnapshot::default();
        snapshot.set_value(
            MainZoneValue::SurroundMode(SurroundMode::new("ROCK ARENA").unwrap()),
            StateAuthority::Authoritative,
        );
        let capabilities = ModelCapabilities::for_model(Model::AvrX3800h);

        assert_eq!(
            admit_main_zone_control(
                &snapshot,
                &capabilities,
                &MainZoneControl::ListeningModeGroup(ListeningModeGroup::Music),
                None,
            ),
            ControlAdmission::NoOp
        );
        assert_eq!(
            admit_main_zone_control(
                &snapshot,
                &capabilities,
                &MainZoneControl::ListeningModeGroup(ListeningModeGroup::Movie),
                None,
            ),
            ControlAdmission::Dispatch
        );
    }

    #[test]
    fn admission_rejects_stale_resource_versions_before_dispatch() {
        let snapshot = MainZoneSnapshot::default();
        let capabilities = ModelCapabilities::for_model(Model::AvrX3800h);
        let result = admit_main_zone_control(
            &snapshot,
            &capabilities,
            &MainZoneControl::Power(denon_avr_domain::PowerState::On),
            Some(snapshot.resource_version().saturating_add(1)),
        );
        assert!(matches!(
            result,
            ControlAdmission::Rejected(OperationError {
                kind: OperationErrorKind::Conflict,
                ..
            })
        ));
    }
}
