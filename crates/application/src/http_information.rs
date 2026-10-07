//! Policy for optional HTTP information reads.

use crate::ports::OperationError;
use denon_avr_domain::{
    Freshness, HttpInformationSnapshot, MainZoneControl, ModelCapabilities, ReceiverIntent,
    ZonePower,
};

pub fn should_read(capabilities: &ModelCapabilities, saved_receiver: bool) -> bool {
    capabilities.http_information_read || saved_receiver
}

pub fn merge_refresh(
    previous: HttpInformationSnapshot,
    generation: u64,
    result: Result<HttpInformationSnapshot, OperationError>,
) -> (HttpInformationSnapshot, Result<(), OperationError>) {
    match result {
        Ok(mut information) => {
            information.generation = generation;
            (information, Ok(()))
        }
        Err(error) => {
            // Optional HTTP reads never invalidate useful Telnet state from
            // the same connection.
            let mut information = previous;
            information.generation = generation;
            information.freshness = if information.observed_at.is_some() {
                Freshness::Partial
            } else {
                Freshness::Unknown
            };
            information.error = Some(error.to_string());
            (information, Err(error))
        }
    }
}

/// Whether a confirmed control can change what the receiver reports over HTTP.
pub fn intent_may_have_changed(intent: &ReceiverIntent) -> bool {
    matches!(
        intent,
        ReceiverIntent::Source(_)
            | ReceiverIntent::SoundMode(_)
            | ReceiverIntent::MainZonePower(ZonePower::On)
            | ReceiverIntent::SystemPower(denon_avr_domain::SystemPower::On)
    )
}

pub(crate) fn may_have_changed(control: &MainZoneControl) -> bool {
    matches!(
        control,
        MainZoneControl::Input(_)
            | MainZoneControl::SurroundMode(_)
            | MainZoneControl::SelectSoundMode { .. }
            | MainZoneControl::RecallSoundModeCategory(_)
            | MainZoneControl::Power(denon_avr_domain::PowerState::On)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{OperationError, OperationErrorKind};
    use std::time::SystemTime;

    #[test]
    fn failed_refresh_preserves_same_connection_information_as_partial() {
        let previous = HttpInformationSnapshot {
            observed_at: Some(SystemTime::UNIX_EPOCH),
            generation: 7,
            ..HttpInformationSnapshot::default()
        };
        let error = OperationError::new(OperationErrorKind::Timeout, "HTTP information", "late");
        let (information, result) = merge_refresh(previous, 7, Err(error.clone()));
        assert_eq!(information.freshness, Freshness::Partial);
        assert_eq!(information.generation, 7);
        assert_eq!(result, Err(error));
    }
}
