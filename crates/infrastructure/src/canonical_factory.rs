//! Compatibility composition for the existing GUI controller.
//!
//! The adapter exposes the older application port while delegating all wire
//! ownership, observation, pacing, and confirmation to `X3800hSession`.

use crate::{
    AvrSessionConfig, HttpInformationHttpClient, QuickSelectNamesHttpClient,
    SourceCatalogHttpClient, X3800hSession,
};
use denon_avr_application::ports::{
    BoxFuture, OperationError, OperationErrorKind, ReceiverSession, SessionEvent, SessionFactory,
};
use denon_avr_application::CanonicalReceiverSession;
use denon_avr_domain::{
    MainZoneControl, MainZoneEvent, MainZoneField, MainZoneValue, PowerState, ReceiverId,
    ReceiverIdentity, ReceiverIntent, SoundModeIntent, SurroundMode, Volume, VolumeLevel,
    Zone2Control,
};
use std::sync::Arc;
use tracing::{debug, warn};

#[derive(Debug, Clone, Default)]
pub struct CanonicalSessionFactory {
    pub config: AvrSessionConfig,
}

impl SessionFactory for CanonicalSessionFactory {
    fn connect(
        &self,
        identity: ReceiverIdentity,
    ) -> BoxFuture<'_, Result<Box<dyn ReceiverSession>, OperationError>> {
        let config = self.config.clone();
        Box::pin(async move {
            let receiver = ReceiverId::new(
                identity
                    .friendly_name
                    .clone()
                    .unwrap_or_else(|| identity.host.clone()),
            )
            .map_err(|error| {
                OperationError::new(
                    OperationErrorKind::InvalidSelection,
                    "receiver identity",
                    error,
                )
            })?;
            let host = identity.host.clone();
            let session = X3800hSession::connect(receiver, &host, config.clone()).await?;
            Ok(
                Box::new(CanonicalSessionAdapter::new(session, host, config))
                    as Box<dyn ReceiverSession>,
            )
        })
    }
}

struct CanonicalSessionAdapter {
    session: Arc<X3800hSession>,
    host: String,
    config: AvrSessionConfig,
    states: denon_avr_application::StateSubscription,
    last_state: denon_avr_domain::ReceiverState,
    connected: bool,
    next_operation: u64,
}

impl CanonicalSessionAdapter {
    fn new(session: Arc<X3800hSession>, host: String, config: AvrSessionConfig) -> Self {
        let states = session.state();
        let last_state = session.current_state();
        Self {
            session,
            host,
            config,
            states,
            last_state,
            connected: false,
            next_operation: 1,
        }
    }
    async fn value(&self, field: MainZoneField) -> Result<MainZoneValue, OperationError> {
        let state = self.session.current_state();
        match field {
            MainZoneField::Power => state
                .main_zone
                .power
                .last_good
                .as_ref()
                .filter(|_| is_current(&state.main_zone.power))
                .map(|o| {
                    MainZoneValue::Power(match o.value {
                        denon_avr_domain::ZonePower::On => PowerState::On,
                        denon_avr_domain::ZonePower::Off => PowerState::Standby,
                    })
                })
                .ok_or_else(|| unavailable(field)),
            MainZoneField::Input => state
                .main_zone
                .source
                .last_good
                .as_ref()
                .filter(|_| is_current(&state.main_zone.source))
                .and_then(|o| denon_avr_domain::Input::new(o.value.as_str()).ok())
                .map(MainZoneValue::Input)
                .ok_or_else(|| unavailable(field)),
            MainZoneField::Volume => state
                .main_zone
                .volume
                .last_good
                .as_ref()
                .filter(|_| is_current(&state.main_zone.volume))
                .map(|o| MainZoneValue::Volume(volume(o.value)))
                .ok_or_else(|| unavailable(field)),
            MainZoneField::Mute => state
                .main_zone
                .mute
                .last_good
                .as_ref()
                .filter(|_| is_current(&state.main_zone.mute))
                .map(|o| MainZoneValue::Mute(o.value))
                .ok_or_else(|| unavailable(field)),
            MainZoneField::SurroundMode => state
                .main_zone
                .sound_mode
                .last_good
                .as_ref()
                .filter(|_| is_current(&state.main_zone.sound_mode))
                .and_then(|o| SurroundMode::new(o.value.id.clone()).ok())
                .map(MainZoneValue::SurroundMode)
                .ok_or_else(|| unavailable(field)),
        }
    }
}

impl ReceiverSession for CanonicalSessionAdapter {
    fn query_field(
        &mut self,
        field: MainZoneField,
    ) -> BoxFuture<'_, Result<MainZoneValue, OperationError>> {
        Box::pin(async move {
            let core_field = match field {
                MainZoneField::Power => denon_avr_domain::CoreField::MainZonePower,
                MainZoneField::Input => denon_avr_domain::CoreField::Source,
                MainZoneField::Volume => denon_avr_domain::CoreField::Volume,
                MainZoneField::Mute => denon_avr_domain::CoreField::Mute,
                MainZoneField::SurroundMode => denon_avr_domain::CoreField::SoundMode,
            };
            self.session.observe(core_field).await?;
            self.value(field).await
        })
    }
    fn execute_once(
        &mut self,
        control: MainZoneControl,
    ) -> BoxFuture<'_, Result<(), OperationError>> {
        let intent = match control {
            MainZoneControl::Power(PowerState::On) => {
                ReceiverIntent::MainZonePower(denon_avr_domain::ZonePower::On)
            }
            MainZoneControl::Power(PowerState::Standby) => {
                ReceiverIntent::MainZonePower(denon_avr_domain::ZonePower::Off)
            }
            MainZoneControl::Input(value) => {
                match denon_avr_domain::SourceId::new(value.as_str()) {
                    Ok(source) => ReceiverIntent::Source(source),
                    Err(error) => {
                        return Box::pin(async move {
                            Err(OperationError::new(
                                OperationErrorKind::InvalidSelection,
                                "receiver source",
                                error,
                            ))
                        });
                    }
                }
            }
            MainZoneControl::Volume(value) => ReceiverIntent::Volume(master_volume(value)),
            MainZoneControl::Mute(value) => ReceiverIntent::Mute(value),
            MainZoneControl::SurroundMode(value) => {
                ReceiverIntent::SoundMode(SoundModeIntent::Select(value.as_str().into()))
            }
            MainZoneControl::SelectSoundMode { mode, .. } => {
                ReceiverIntent::SoundMode(SoundModeIntent::Select(mode.as_str().into()))
            }
            MainZoneControl::RecallSoundModeCategory(category) => {
                ReceiverIntent::SoundMode(match category {
                    denon_avr_domain::SoundModeCategory::Movie => SoundModeIntent::RecallMovie,
                    denon_avr_domain::SoundModeCategory::Music => SoundModeIntent::RecallMusic,
                    denon_avr_domain::SoundModeCategory::Game => SoundModeIntent::RecallGame,
                    denon_avr_domain::SoundModeCategory::Pure => SoundModeIntent::PureDirect,
                })
            }
        };
        let session = Arc::clone(&self.session);
        let operation = denon_avr_domain::OperationId(self.next_operation);
        self.next_operation = self.next_operation.saturating_add(1);
        Box::pin(async move {
            let outcome = session
                .operate(denon_avr_application::OperationRequest {
                    id: operation,
                    intent,
                })
                .await;
            match outcome {
                denon_avr_domain::OperationOutcome::ObservedRequestedValue { .. }
                | denon_avr_domain::OperationOutcome::AlreadyObserved { .. } => {
                    debug!(?operation, "canonical control completed");
                    Ok(())
                }
                outcome => Err(OperationError::new(
                    OperationErrorKind::Unavailable,
                    "receiver control",
                    {
                        warn!(?operation, outcome = ?outcome, "canonical control did not complete");
                        format!("{outcome:?}")
                    },
                )),
            }
        })
    }
    fn next_event(&mut self) -> BoxFuture<'_, Result<SessionEvent, OperationError>> {
        Box::pin(async move {
            if !self.connected {
                self.connected = true;
                return Ok(SessionEvent::Connection(
                    denon_avr_domain::ConnectionState::Connected,
                ));
            }
            loop {
                let state = self.states.changed().await?;
                let previous_epoch = self.last_state.epoch;
                let current_epoch = state.epoch;
                let value = changed_value(&self.last_state, &state);
                self.last_state = state;
                if let Some(connection) = lifecycle_transition(previous_epoch, current_epoch) {
                    return Ok(SessionEvent::Connection(connection));
                }
                if let Some(value) = value {
                    return Ok(SessionEvent::MainZone(MainZoneEvent::Changed(value)));
                }
            }
        })
    }
    fn close(&mut self) -> BoxFuture<'_, Result<(), OperationError>> {
        let session = Arc::clone(&self.session);
        Box::pin(async move { session.close().await })
    }
    fn query_zone2_power(&mut self) -> BoxFuture<'_, Result<PowerState, OperationError>> {
        let session = Arc::clone(&self.session);
        Box::pin(async move {
            session
                .observe(denon_avr_domain::CoreField::Zone2Power)
                .await?;
            let state = session.current_state();
            state
                .zone2_power
                .last_good
                .as_ref()
                .filter(|_| is_current(&state.zone2_power))
                .map(|o| match o.value {
                    denon_avr_domain::ZonePower::On => PowerState::On,
                    denon_avr_domain::ZonePower::Off => PowerState::Standby,
                })
                .ok_or_else(|| unavailable(MainZoneField::Power))
        })
    }
    fn execute_zone2_once(
        &mut self,
        control: Zone2Control,
    ) -> BoxFuture<'_, Result<(), OperationError>> {
        let session = Arc::clone(&self.session);
        let operation = denon_avr_domain::OperationId(self.next_operation);
        self.next_operation = self.next_operation.saturating_add(1);
        Box::pin(async move {
            let intent = ReceiverIntent::Zone2Power(match control {
                Zone2Control::Power(PowerState::On) => denon_avr_domain::ZonePower::On,
                Zone2Control::Power(PowerState::Standby) => denon_avr_domain::ZonePower::Off,
            });
            match session
                .operate(denon_avr_application::OperationRequest {
                    id: operation,
                    intent,
                })
                .await
            {
                denon_avr_domain::OperationOutcome::ObservedRequestedValue { .. }
                | denon_avr_domain::OperationOutcome::AlreadyObserved { .. } => Ok(()),
                outcome => Err(OperationError::new(
                    OperationErrorKind::Unavailable,
                    "Zone 2 control",
                    format!("{outcome:?}"),
                )),
            }
        })
    }
    fn query_audio_context(&mut self) -> BoxFuture<'_, denon_avr_domain::AudioContextSnapshot> {
        Box::pin(async { denon_avr_domain::AudioContextSnapshot::default() })
    }
    fn refresh_http_information(
        &mut self,
    ) -> BoxFuture<'_, Result<denon_avr_domain::HttpInformationSnapshot, OperationError>> {
        let host = self.host.clone();
        let timeout = self.config.connect_timeout;
        let generation = self
            .session
            .current_state()
            .epoch
            .map_or(0, |epoch| epoch.0.saturating_sub(1));
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                HttpInformationHttpClient::new(host, timeout)
                    .and_then(|client| client.read(generation))
            })
            .await
            .map_err(|error| {
                OperationError::new(
                    OperationErrorKind::Stopped,
                    "HTTP information",
                    error.to_string(),
                )
            })?
            .map_err(|error| {
                OperationError::new(
                    OperationErrorKind::Connection,
                    "HTTP information",
                    error.to_string(),
                )
            })
        })
    }
    fn refresh_source_catalog(
        &mut self,
    ) -> BoxFuture<'_, Result<denon_avr_domain::SourceCatalogObservation, OperationError>> {
        let host = self.host.clone();
        let timeout = self.config.connect_timeout;
        let port = self.config.app_command_port;
        let generation = self
            .session
            .current_state()
            .epoch
            .map_or(0, |epoch| epoch.0.saturating_sub(1));
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                SourceCatalogHttpClient::new(
                    denon_avr_domain::ReceiverEndpoint { host, port },
                    timeout,
                )
                .and_then(|client| client.read(generation))
            })
            .await
            .map_err(|error| {
                OperationError::new(
                    OperationErrorKind::Stopped,
                    "source catalog",
                    error.to_string(),
                )
            })?
            .map_err(|error| {
                OperationError::new(
                    OperationErrorKind::Connection,
                    "source catalog",
                    error.to_string(),
                )
            })
        })
    }
    fn refresh_quick_select_names(
        &mut self,
    ) -> BoxFuture<'_, Result<denon_avr_domain::QuickSelectNameObservation, OperationError>> {
        let host = self.host.clone();
        let timeout = self.config.connect_timeout;
        let port = self.config.app_command_port;
        let generation = self
            .session
            .current_state()
            .epoch
            .map_or(0, |epoch| epoch.0.saturating_sub(1));
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                QuickSelectNamesHttpClient::new(
                    denon_avr_domain::ReceiverEndpoint { host, port },
                    timeout,
                )
                .and_then(|client| client.read(generation))
            })
            .await
            .map_err(|error| {
                OperationError::new(
                    OperationErrorKind::Stopped,
                    "Quick Select names",
                    error.to_string(),
                )
            })?
            .map_err(|error| {
                OperationError::new(
                    OperationErrorKind::Connection,
                    "Quick Select names",
                    error.to_string(),
                )
            })
        })
    }
}

fn lifecycle_transition(
    previous: Option<denon_avr_domain::Epoch>,
    current: Option<denon_avr_domain::Epoch>,
) -> Option<denon_avr_domain::ConnectionState> {
    match (previous.is_some(), current.is_some()) {
        (true, false) => Some(denon_avr_domain::ConnectionState::Reconnecting),
        (false, true) => Some(denon_avr_domain::ConnectionState::Connected),
        _ => None,
    }
}

fn is_current<T>(field: &denon_avr_domain::ReceiverFieldState<T>) -> bool {
    matches!(
        field.validity,
        denon_avr_domain::ReceiverFieldValidity::Current { .. }
    )
}
fn unavailable(field: MainZoneField) -> OperationError {
    OperationError::new(
        OperationErrorKind::Unavailable,
        field.name(),
        "canonical field is not current",
    )
}
fn volume(value: denon_avr_domain::MasterVolume) -> Volume {
    match value {
        denon_avr_domain::MasterVolume::Minimum => Volume::from_parts("MIN", 0),
        denon_avr_domain::MasterVolume::DbHalfSteps(step) => {
            let native = step + 160;
            let code = if native % 2 == 0 {
                format!("{:02}", native / 2)
            } else {
                format!("{:02}5", native / 2)
            };
            Volume::from_parts(code, step * 5)
        }
    }
}
fn master_volume(value: VolumeLevel) -> denon_avr_domain::MasterVolume {
    let code = value.to_native_code() / 10;
    let step = (code as i16) * 2 - 160;
    denon_avr_domain::MasterVolume::db_half_steps(step)
        .unwrap_or(denon_avr_domain::MasterVolume::Minimum)
}
fn changed_value(
    previous: &denon_avr_domain::ReceiverState,
    state: &denon_avr_domain::ReceiverState,
) -> Option<MainZoneValue> {
    if previous.main_zone.power.last_good.as_ref().map(|o| o.value)
        != state.main_zone.power.last_good.as_ref().map(|o| o.value)
    {
        return state.main_zone.power.last_good.as_ref().map(|o| {
            MainZoneValue::Power(match o.value {
                denon_avr_domain::ZonePower::On => PowerState::On,
                denon_avr_domain::ZonePower::Off => PowerState::Standby,
            })
        });
    }
    if previous
        .main_zone
        .source
        .last_good
        .as_ref()
        .map(|o| o.value.as_str())
        != state
            .main_zone
            .source
            .last_good
            .as_ref()
            .map(|o| o.value.as_str())
    {
        if let Some(value) = state
            .main_zone
            .source
            .last_good
            .as_ref()
            .and_then(|o| denon_avr_domain::Input::new(o.value.as_str()).ok())
        {
            return Some(MainZoneValue::Input(value));
        }
    }
    if previous
        .main_zone
        .volume
        .last_good
        .as_ref()
        .map(|o| o.value)
        != state.main_zone.volume.last_good.as_ref().map(|o| o.value)
    {
        if let Some(value) = state.main_zone.volume.last_good.as_ref() {
            return Some(MainZoneValue::Volume(volume(value.value)));
        }
    }
    if previous.main_zone.mute.last_good.as_ref().map(|o| o.value)
        != state.main_zone.mute.last_good.as_ref().map(|o| o.value)
    {
        if let Some(value) = state.main_zone.mute.last_good.as_ref() {
            return Some(MainZoneValue::Mute(value.value));
        }
    }
    state
        .main_zone
        .sound_mode
        .last_good
        .as_ref()
        .and_then(|o| SurroundMode::new(o.value.id.clone()).ok())
        .map(MainZoneValue::SurroundMode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use denon_avr_domain::{
        CoreFrame, Epoch, FrameSeq, MonotonicMillis, MuteState, ObservationOrigin,
    };

    #[test]
    fn unrelated_or_invalidated_state_does_not_fabricate_main_zone_events() {
        let receiver = ReceiverId::new("adapter-test").unwrap();
        let mut previous = denon_avr_domain::ReceiverState::new(receiver.clone());
        previous.establish_epoch(Epoch(1));
        previous.reduce(
            &receiver,
            Epoch(1),
            FrameSeq(1),
            MonotonicMillis(0),
            MonotonicMillis(10_000),
            ObservationOrigin::ReceiverFrame,
            CoreFrame::Mute(MuteState::Off),
        );
        let mut unrelated = previous.clone();
        unrelated.reduce(
            &receiver,
            Epoch(1),
            FrameSeq(2),
            MonotonicMillis(1),
            MonotonicMillis(10_001),
            ObservationOrigin::ReceiverFrame,
            CoreFrame::Zone2Power(denon_avr_domain::ZonePower::On),
        );
        assert!(changed_value(&previous, &unrelated).is_none());
        let mut disconnected = previous.clone();
        disconnected.mark_disconnected();
        assert!(changed_value(&previous, &disconnected).is_none());
    }

    #[test]
    fn epoch_transitions_map_to_lifecycle_events_without_cross_epoch_noise() {
        assert_eq!(
            lifecycle_transition(Some(Epoch(1)), None),
            Some(denon_avr_domain::ConnectionState::Reconnecting)
        );
        assert_eq!(
            lifecycle_transition(None, Some(Epoch(2))),
            Some(denon_avr_domain::ConnectionState::Connected)
        );
        assert_eq!(lifecycle_transition(Some(Epoch(1)), Some(Epoch(2))), None);
        assert_eq!(lifecycle_transition(None, None), None);
    }
}
