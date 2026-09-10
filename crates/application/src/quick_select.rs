//! Quick Select and EQ state policy, independent of session coordination.

use crate::ports::{OperationError, OperationErrorKind};
use denon_avr_domain::{
    EqStatus, ModelCapabilities, QuickSelectNameObservation, QuickSelectRecallConfirmation,
    QuickSelectRecallOutcome, QuickSelectSlot, QuickSelectSnapshot, SurroundMode,
};

pub(crate) fn ensure_eq_supported(capabilities: &ModelCapabilities) -> Result<(), OperationError> {
    ensure_supported(
        capabilities.eq_status,
        "EQ status",
        "selected receiver has no validated EQ status capability",
    )
}

pub(crate) fn ensure_names_supported(
    capabilities: &ModelCapabilities,
) -> Result<(), OperationError> {
    ensure_supported(
        capabilities.quick_select_names,
        "Quick Select names",
        "selected receiver has no validated Quick Select name capability",
    )
}

pub(crate) fn recall_unsupported(
    capabilities: &ModelCapabilities,
) -> Option<QuickSelectRecallOutcome> {
    (!capabilities.quick_select_recall).then(|| {
        QuickSelectRecallOutcome::Unsupported(
            "selected receiver has no validated Quick Select capability".into(),
        )
    })
}

fn ensure_supported(
    supported: bool,
    context: &'static str,
    message: &'static str,
) -> Result<(), OperationError> {
    supported
        .then_some(())
        .ok_or_else(|| OperationError::new(OperationErrorKind::Unsupported, context, message))
}

pub(crate) fn invalidate(
    quick_select: &mut QuickSelectSnapshot,
    eq_status: &mut EqStatus,
    names: &mut Option<QuickSelectNameObservation>,
    generation: u64,
) {
    quick_select.invalidate();
    eq_status.invalidate();
    *names = None;
    quick_select.generation = generation;
    eq_status.generation = generation;
}

pub(crate) fn merge_eq_status(
    previous: &EqStatus,
    mut refreshed: EqStatus,
    generation: u64,
    current_mode: Option<&SurroundMode>,
) -> EqStatus {
    refreshed.generation = generation;
    refreshed.preserve_failed_observations(previous);
    if let Some(mode) = current_mode {
        refreshed.apply_mode_restrictions(mode.as_str());
    }
    refreshed
}

pub(crate) fn apply_names(
    quick_select: &mut QuickSelectSnapshot,
    mut observation: QuickSelectNameObservation,
    generation: u64,
) -> QuickSelectNameObservation {
    observation.generation = generation;
    quick_select.generation = generation;
    for (index, name) in observation.names.iter().enumerate() {
        if let Some(name) = name {
            let slot = QuickSelectSlot::new(index as u8 + 1)
                .expect("Quick Select name response has four slots");
            quick_select.set_name(slot, name.clone());
        }
    }
    observation
}

pub(crate) fn recall_outcome(
    slot: QuickSelectSlot,
    result: Result<QuickSelectRecallConfirmation, OperationError>,
) -> QuickSelectRecallOutcome {
    match result {
        Ok(QuickSelectRecallConfirmation::Authoritative) => {
            QuickSelectRecallOutcome::Confirmed { slot }
        }
        Ok(QuickSelectRecallConfirmation::Dispatched) => QuickSelectRecallOutcome::Unconfirmed(
            "receiver acknowledged the preset command; resulting state was not authoritative"
                .into(),
        ),
        Err(error) if error.kind == OperationErrorKind::Unsupported => {
            QuickSelectRecallOutcome::Unsupported(error.to_string())
        }
        Err(error)
            if matches!(
                error.kind,
                OperationErrorKind::Timeout
                    | OperationErrorKind::Disconnected
                    | OperationErrorKind::Connection
            ) =>
        {
            QuickSelectRecallOutcome::Unconfirmed(error.to_string())
        }
        Err(error) => QuickSelectRecallOutcome::TransportFailure(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_after_recall_is_unconfirmed_not_safe_to_retry() {
        let slot = QuickSelectSlot::new(1).unwrap();
        let error = OperationError::new(OperationErrorKind::Timeout, "Quick Select", "late");
        assert!(matches!(
            recall_outcome(slot, Err(error)),
            QuickSelectRecallOutcome::Unconfirmed(_)
        ));
    }
}
