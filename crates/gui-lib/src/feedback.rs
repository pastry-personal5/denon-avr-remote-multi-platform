//! Stable user-facing copy for typed control outcomes.

use crate::ControlReport;
use denon_avr_application::OperationStatus;
use denon_avr_domain::DispatchCertainty;

/// The message for how a control ended. The wording follows the outcome: a
/// write that went out but could not be confirmed is not "failed to send".
pub fn control_message(report: &ControlReport) -> String {
    match report {
        ControlReport::Conflict => "Receiver state changed before the command; retry.".into(),
        ControlReport::Failed(error) => format!("Operation failed: {error}"),
        ControlReport::Finished(snapshot) => {
            let reason = || snapshot.reason.clone().unwrap_or_default();
            match &snapshot.status {
                OperationStatus::Completed => "Command confirmed by the receiver.".into(),
                OperationStatus::AlreadyInState => {
                    "Receiver already has the requested value.".into()
                }
                OperationStatus::Rejected | OperationStatus::Denied => {
                    format!("Command rejected: {}", reason())
                }
                OperationStatus::Indeterminate
                    if snapshot.dispatch == DispatchCertainty::NotDispatched =>
                {
                    format!("Command failed to send: {}", reason())
                }
                OperationStatus::Indeterminate => format!(
                    "Command sent, but confirmation is unavailable: {}",
                    reason()
                ),
                OperationStatus::Cancelled | OperationStatus::Superseded { .. } => {
                    "Command cancelled.".into()
                }
                other => format!("Command did not finish ({}).", other.as_str()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use denon_avr_application::OperationSnapshot;
    use denon_avr_domain::{MuteState, OperationId, ReceiverId, ReceiverIntent};

    fn report(status: OperationStatus, dispatch: DispatchCertainty, reason: &str) -> ControlReport {
        ControlReport::Finished(Box::new(OperationSnapshot {
            id: OperationId(1),
            receiver: ReceiverId::new("living-room").unwrap(),
            intent: ReceiverIntent::Mute(MuteState::On),
            status,
            dispatch,
            confirmed: false,
            reason: (!reason.is_empty()).then(|| reason.to_owned()),
            observation: None,
        }))
    }

    #[test]
    fn each_outcome_reads_as_what_happened() {
        use DispatchCertainty::*;
        use OperationStatus::*;
        let cases = [
            (
                report(Completed, CompleteWrite, ""),
                "Command confirmed by the receiver.",
            ),
            (
                report(AlreadyInState, NotDispatched, ""),
                "Receiver already has the requested value.",
            ),
            (
                report(Rejected, NotDispatched, "unsupported"),
                "Command rejected: unsupported",
            ),
            (
                report(Indeterminate, CompleteWrite, "not seen in time"),
                "Command sent, but confirmation is unavailable: not seen in time",
            ),
            (
                report(Indeterminate, Unknown, "ambiguous"),
                "Command sent, but confirmation is unavailable: ambiguous",
            ),
            (
                report(Indeterminate, NotDispatched, "session stopped"),
                "Command failed to send: session stopped",
            ),
            (report(Cancelled, NotDispatched, ""), "Command cancelled."),
            (
                report(Superseded { by: OperationId(2) }, NotDispatched, ""),
                "Command cancelled.",
            ),
            (
                ControlReport::Conflict,
                "Receiver state changed before the command; retry.",
            ),
            (
                ControlReport::Failed("no receiver selected".into()),
                "Operation failed: no receiver selected",
            ),
        ];
        for (report, expected) in cases {
            assert_eq!(control_message(&report), expected, "{report:?}");
        }
    }
}
