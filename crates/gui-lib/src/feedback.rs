//! Stable user-facing copy for typed application outcomes and observations.

use denon_avr_application::ControlResult;
use denon_avr_domain::{EqFeature, EqStatus, QuickSelectRecallOutcome};

pub fn control_message(result: &ControlResult) -> String {
    match result {
        ControlResult::Dispatched { .. } => {
            "Command sent; waiting for receiver confirmation.".into()
        }
        ControlResult::Confirmed { .. } => "Command confirmed by the receiver.".into(),
        ControlResult::NoOp { .. } => "Receiver already has the requested value.".into(),
        ControlResult::Conflict { .. } => {
            "Receiver state changed before the command; retry.".into()
        }
        ControlResult::Unsupported(reason) => format!("Command is unsupported: {reason}"),
        ControlResult::Rejected(error) => format!("Command rejected: {error}"),
        ControlResult::TransportFailure(error) => format!("Command failed to send: {error}"),
        ControlResult::Unconfirmed(error) => {
            format!("Command sent, but confirmation is unavailable: {error}")
        }
        ControlResult::Cancelled => "Command cancelled.".into(),
    }
}

pub fn quick_select_recall_message(outcome: &QuickSelectRecallOutcome) -> String {
    match outcome {
        QuickSelectRecallOutcome::Pending { slot } => {
            format!("Recalling Main Zone Quick Select {}…", slot.number())
        }
        QuickSelectRecallOutcome::Confirmed { slot } => format!(
            "Main Zone Quick Select {} recalled and confirmed.",
            slot.number()
        ),
        QuickSelectRecallOutcome::Rejected(reason) => {
            format!("Quick Select recall was rejected: {reason}")
        }
        QuickSelectRecallOutcome::Conflict { .. } => {
            "Quick Select state changed; retry the recall.".into()
        }
        QuickSelectRecallOutcome::Unsupported(reason) => {
            format!("Quick Select recall is unsupported: {reason}")
        }
        QuickSelectRecallOutcome::TransportFailure(reason) => {
            format!("Quick Select recall failed: {reason}")
        }
        QuickSelectRecallOutcome::Unconfirmed(reason) => format!(
            "Quick Select may have been recalled, but confirmation is unavailable: {reason}"
        ),
    }
}

pub fn eq_summary(status: &EqStatus) -> String {
    EqFeature::ALL
        .into_iter()
        .map(|feature| {
            format!(
                "{}: {}",
                feature.label(),
                status.state(feature).explanation()
            )
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

pub fn eq_evidence_summary(status: &EqStatus) -> String {
    if status.evidence.is_empty() {
        return "No EQ query evidence recorded.".into();
    }
    status
        .evidence
        .iter()
        .map(|e| {
            format!(
                "{}: {} ms; response={:?}; error={:?}{}",
                e.feature.label(),
                e.elapsed_millis,
                e.response,
                e.error,
                if e.preserved_previous {
                    "; retained previous value"
                } else {
                    ""
                }
            )
        })
        .collect::<Vec<_>>()
        .join(" · ")
}
