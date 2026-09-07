use crate::application::{AsyncStatusGateway, OperationError, OperationErrorKind, StatusGateway};
use crate::domain::{FieldError, FieldErrorKind, MainZoneField, MainZoneSnapshot, StateAuthority};

pub fn query_main_zone(gateway: &mut impl StatusGateway) -> MainZoneSnapshot {
    let generation = gateway.connection_generation();
    let snapshot = query_main_zone_once(gateway);
    if gateway.connection_generation() != generation {
        return query_main_zone_once(gateway);
    }
    snapshot
}

fn query_main_zone_once(gateway: &mut impl StatusGateway) -> MainZoneSnapshot {
    let mut snapshot = MainZoneSnapshot::default();
    for field in MainZoneField::ALL {
        match gateway.query_field(field) {
            Ok(value) => snapshot.set_value(value, StateAuthority::Authoritative),
            Err(error) => snapshot.set_error(field, field_error(error)),
        }
    }
    snapshot
}

pub async fn query_main_zone_async(gateway: &mut impl AsyncStatusGateway) -> MainZoneSnapshot {
    let generation = gateway.connection_generation();
    let snapshot = query_main_zone_async_once(gateway).await;
    if gateway.connection_generation() != generation {
        return query_main_zone_async_once(gateway).await;
    }
    snapshot
}

async fn query_main_zone_async_once(gateway: &mut impl AsyncStatusGateway) -> MainZoneSnapshot {
    let mut snapshot = MainZoneSnapshot::default();
    for field in MainZoneField::ALL {
        match gateway.query_field(field).await {
            Ok(value) => snapshot.set_value(value, StateAuthority::Authoritative),
            Err(error) => snapshot.set_error(field, field_error(error)),
        }
    }
    snapshot
}

fn field_error(error: OperationError) -> FieldError {
    let kind = match error.kind {
        OperationErrorKind::Timeout => FieldErrorKind::Timeout,
        OperationErrorKind::Disconnected | OperationErrorKind::Connection => {
            FieldErrorKind::Disconnected
        }
        OperationErrorKind::Malformed => FieldErrorKind::Malformed,
        OperationErrorKind::Unsupported => FieldErrorKind::Unsupported,
        _ => FieldErrorKind::Unavailable,
    };
    FieldError {
        kind,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::SessionEvent;
    use crate::domain::{Input, MainZoneValue, MuteState, PowerState, SurroundMode, Volume};
    use std::collections::VecDeque;
    use std::time::Duration;

    struct Fake {
        values: VecDeque<Result<MainZoneValue, OperationError>>,
    }

    impl StatusGateway for Fake {
        fn query_field(&mut self, _field: MainZoneField) -> Result<MainZoneValue, OperationError> {
            self.values.pop_front().unwrap()
        }

        fn connection_generation(&self) -> u64 {
            0
        }

        fn next_event(
            &mut self,
            _timeout: Option<Duration>,
        ) -> Result<SessionEvent, OperationError> {
            unreachable!()
        }
    }

    #[test]
    fn preserves_independent_field_failures() {
        let mut fake = Fake {
            values: VecDeque::from([
                Ok(MainZoneValue::Power(PowerState::On)),
                Ok(MainZoneValue::Input(Input::new("CD").unwrap())),
                Err(OperationError::new(
                    OperationErrorKind::Timeout,
                    "querying volume",
                    "timed out",
                )),
                Ok(MainZoneValue::Mute(MuteState::Off)),
                Ok(MainZoneValue::SurroundMode(
                    SurroundMode::new("STEREO").unwrap(),
                )),
            ]),
        };
        let status = query_main_zone(&mut fake);
        assert_eq!(status.power.value(), Some(&PowerState::On));
        assert!(status.volume.value().is_none());
    }

    #[test]
    fn typed_volume_is_displayable() {
        assert_eq!(Volume::from_parts("80", 0).to_string(), "code 80 (0.0 dB)");
    }
}
