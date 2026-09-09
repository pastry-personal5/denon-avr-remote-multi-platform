//! Main zone domain types and state management.

use super::{AudioContextSnapshot, HttpInformationSnapshot};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState {
    On,
    Standby,
}

impl fmt::Display for PowerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::On => "on",
            Self::Standby => "standby",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuteState {
    On,
    Off,
}

impl fmt::Display for MuteState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::On => "on",
            Self::Off => "off",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input(String);

impl Input {
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        if value.trim().is_empty() {
            Err("input must not be empty")
        } else {
            Ok(Self(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Input {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurroundMode(String);

impl SurroundMode {
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        if value.trim().is_empty() {
            Err("surround mode must not be empty")
        } else {
            Ok(Self(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SurroundMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListeningModeGroup {
    Movie,
    Music,
    Game,
}

impl ListeningModeGroup {
    pub const ALL: [Self; 3] = [Self::Movie, Self::Music, Self::Game];
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "Movie",
            Self::Music => "Music",
            Self::Game => "Game",
        }
    }
    pub const fn command_suffix(self) -> &'static str {
        match self {
            Self::Movie => "MOVIE",
            Self::Music => "MUSIC",
            Self::Game => "GAME",
        }
    }
}

impl fmt::Display for ListeningModeGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioContextField {
    InputMode,
    DigitalMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioContextValue(String);

impl AudioContextValue {
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        if value.trim().is_empty() {
            Err("audio context value must not be empty")
        } else {
            Ok(Self(value))
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AudioContextValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    code: String,
    db_tenths: i16,
}

/// A receiver-independent volume level for user interfaces.
///
/// Values are stored in half-level steps: `0` represents 0.0 and `1000`
/// represents 100.0. Values decoded from the receiver also retain their exact
/// native code so that an unchanged level can be sent back without rounding.
#[derive(Debug, Clone, Copy)]
pub struct VolumeLevel {
    tenths: u16,
    /// The exact AVR code when this level originated from a receiver status
    /// response. The normalized 0–100 scale cannot encode every AVR half-step.
    native_code: Option<u16>,
}

impl PartialEq for VolumeLevel {
    fn eq(&self, other: &Self) -> bool {
        self.tenths == other.tenths
    }
}

impl Eq for VolumeLevel {}

impl VolumeLevel {
    pub const MIN: u16 = 0;
    pub const MAX: u16 = 1000;

    pub fn new(tenths: u16) -> Result<Self, &'static str> {
        if tenths > Self::MAX || !tenths.is_multiple_of(5) {
            return Err("volume level must be between 0.0 and 100.0 in 0.5 steps");
        }
        Ok(Self {
            tenths,
            native_code: None,
        })
    }

    pub fn from_native_code(code: u16) -> Result<Self, &'static str> {
        if code > 985 || !code.is_multiple_of(5) {
            return Err("native volume code is outside the supported range");
        }
        let level = ((u32::from(code) * u32::from(Self::MAX) + 492) / 985) as u16;
        Ok(Self {
            tenths: (level / 5) * 5,
            native_code: Some(code),
        })
    }

    pub fn tenths(self) -> u16 {
        self.tenths
    }

    pub fn as_f32(self) -> f32 {
        self.tenths as f32 / 10.0
    }

    pub fn to_native_code(self) -> u16 {
        self.native_code.unwrap_or_else(|| {
            let raw = (u32::from(self.tenths) * 985 + 500) / 1000;
            (((raw + 2) / 5) * 5) as u16
        })
    }
}

impl fmt::Display for VolumeLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.1}", self.as_f32())
    }
}

impl Volume {
    pub fn from_parts(code: impl Into<String>, db_tenths: i16) -> Self {
        Self {
            code: code.into(),
            db_tenths,
        }
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn db_tenths(&self) -> i16 {
        self.db_tenths
    }

    pub fn native_code(&self) -> u16 {
        let code = self.code.trim();
        let parsed = code.parse::<u16>().unwrap_or(0);
        if code.len() == 2 {
            parsed.saturating_mul(10)
        } else {
            parsed
        }
    }

    pub fn level(&self) -> Result<VolumeLevel, &'static str> {
        VolumeLevel::from_native_code(self.native_code())
    }
}

impl fmt::Display for Volume {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "code {} ({:.1} dB)",
            self.code,
            self.db_tenths as f32 / 10.0
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainZoneField {
    Power,
    Input,
    Volume,
    Mute,
    SurroundMode,
}

impl MainZoneField {
    pub const ALL: [Self; 5] = [
        Self::Power,
        Self::Input,
        Self::Volume,
        Self::Mute,
        Self::SurroundMode,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Power => "power",
            Self::Input => "input",
            Self::Volume => "volume",
            Self::Mute => "mute",
            Self::SurroundMode => "surround",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainZoneValue {
    Power(PowerState),
    Input(Input),
    Volume(Volume),
    Mute(MuteState),
    SurroundMode(SurroundMode),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainZoneControl {
    Power(PowerState),
    Input(Input),
    Volume(VolumeLevel),
    Mute(MuteState),
    SurroundMode(SurroundMode),
    ListeningModeGroup(ListeningModeGroup),
}

impl fmt::Display for MainZoneValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Power(value) => value.fmt(f),
            Self::Input(value) => value.fmt(f),
            Self::Volume(value) => value.fmt(f),
            Self::Mute(value) => value.fmt(f),
            Self::SurroundMode(value) => value.fmt(f),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldErrorKind {
    Unsupported,
    Timeout,
    Disconnected,
    Malformed,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError {
    pub kind: FieldErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldStatus<T> {
    Value(T),
    Unavailable(FieldError),
}

impl<T> FieldStatus<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Unavailable(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Unknown,
    Live,
    /// A refresh completed, but one or more independent fields retained an
    /// older observation because their query failed.
    Partial,
    Invalidated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateAuthority {
    Unconfirmed,
    Event,
    Authoritative,
}

fn not_queried() -> FieldError {
    FieldError {
        kind: FieldErrorKind::Unavailable,
        message: "not queried".into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainZoneSnapshot {
    pub audio_context: AudioContextSnapshot,
    /// Read-only, model-gated AppCommand information. This is deliberately
    /// separate from core Telnet status so an HTTP failure cannot invalidate it.
    pub http_information: HttpInformationSnapshot,
    pub power: FieldStatus<PowerState>,
    pub input: FieldStatus<Input>,
    pub volume: FieldStatus<Volume>,
    pub mute: FieldStatus<MuteState>,
    pub surround_mode: FieldStatus<SurroundMode>,
    pub freshness: Freshness,
    pub authority: StateAuthority,
    resource_version: u64,
}

impl Default for MainZoneSnapshot {
    fn default() -> Self {
        Self {
            audio_context: AudioContextSnapshot::default(),
            http_information: HttpInformationSnapshot::default(),
            power: FieldStatus::Unavailable(not_queried()),
            input: FieldStatus::Unavailable(not_queried()),
            volume: FieldStatus::Unavailable(not_queried()),
            mute: FieldStatus::Unavailable(not_queried()),
            surround_mode: FieldStatus::Unavailable(not_queried()),
            freshness: Freshness::Unknown,
            authority: StateAuthority::Unconfirmed,
            resource_version: 0,
        }
    }
}

impl MainZoneSnapshot {
    pub fn invalidate(&mut self) {
        let version = self.resource_version.saturating_add(1);
        *self = Self::default();
        self.freshness = Freshness::Invalidated;
        self.http_information.invalidate(0);
        self.resource_version = version;
    }

    pub fn invalidate_audio_context(&mut self) {
        self.audio_context.invalidate();
        self.resource_version = self.resource_version.saturating_add(1);
    }

    pub fn invalidate_http_information(&mut self, generation: u64) {
        self.http_information.invalidate(generation);
        self.resource_version = self.resource_version.saturating_add(1);
    }

    pub fn set_http_information(&mut self, information: HttpInformationSnapshot) {
        if self.http_information != information {
            self.resource_version = self.resource_version.saturating_add(1);
        }
        self.http_information = information;
    }

    pub fn set_audio_context(&mut self, context: AudioContextSnapshot) {
        if self.audio_context != context {
            self.resource_version = self.resource_version.saturating_add(1);
        }
        self.audio_context = context;
    }

    pub fn set_value(&mut self, value: MainZoneValue, authority: StateAuthority) {
        let invalidates_audio_context = matches!(
            &value,
            MainZoneValue::Input(_) | MainZoneValue::SurroundMode(_)
        );
        let field = match &value {
            MainZoneValue::Power(_) => MainZoneField::Power,
            MainZoneValue::Input(_) => MainZoneField::Input,
            MainZoneValue::Volume(_) => MainZoneField::Volume,
            MainZoneValue::Mute(_) => MainZoneField::Mute,
            MainZoneValue::SurroundMode(_) => MainZoneField::SurroundMode,
        };
        if self.value(field).as_ref() != Some(&value) {
            self.resource_version = self.resource_version.saturating_add(1);
        }
        match value {
            MainZoneValue::Power(value) => self.power = FieldStatus::Value(value),
            MainZoneValue::Input(value) => self.input = FieldStatus::Value(value),
            MainZoneValue::Volume(value) => self.volume = FieldStatus::Value(value),
            MainZoneValue::Mute(value) => self.mute = FieldStatus::Value(value),
            MainZoneValue::SurroundMode(value) => self.surround_mode = FieldStatus::Value(value),
        }
        if invalidates_audio_context && !self.audio_context.invalidated {
            self.invalidate_audio_context();
        }
        if matches!(self.power, FieldStatus::Value(PowerState::Standby)) {
            self.http_information.invalidate(0);
        }
        self.freshness = Freshness::Live;
        self.authority = authority;
    }

    pub fn set_error(&mut self, field: MainZoneField, error: FieldError) {
        let changed = match field {
            MainZoneField::Power => {
                !matches!(&self.power, FieldStatus::Unavailable(old) if old == &error)
            }
            MainZoneField::Input => {
                !matches!(&self.input, FieldStatus::Unavailable(old) if old == &error)
            }
            MainZoneField::Volume => {
                !matches!(&self.volume, FieldStatus::Unavailable(old) if old == &error)
            }
            MainZoneField::Mute => {
                !matches!(&self.mute, FieldStatus::Unavailable(old) if old == &error)
            }
            MainZoneField::SurroundMode => {
                !matches!(&self.surround_mode, FieldStatus::Unavailable(old) if old == &error)
            }
        };
        if changed {
            self.resource_version = self.resource_version.saturating_add(1);
        }
        match field {
            MainZoneField::Power => self.power = FieldStatus::Unavailable(error),
            MainZoneField::Input => self.input = FieldStatus::Unavailable(error),
            MainZoneField::Volume => self.volume = FieldStatus::Unavailable(error),
            MainZoneField::Mute => self.mute = FieldStatus::Unavailable(error),
            MainZoneField::SurroundMode => self.surround_mode = FieldStatus::Unavailable(error),
        }
    }

    pub fn apply_event(&mut self, event: MainZoneEvent) {
        if let MainZoneEvent::Changed(value) = event {
            self.set_value(value, StateAuthority::Event);
        }
    }

    pub fn apply_authoritative_event(&mut self, event: MainZoneEvent) {
        if let MainZoneEvent::Changed(value) = event {
            self.set_value(value, StateAuthority::Authoritative);
        }
    }

    pub fn probe_succeeded(&self) -> bool {
        fn field_ok<T>(field: &FieldStatus<T>) -> bool {
            match field {
                FieldStatus::Value(_) => true,
                FieldStatus::Unavailable(error) => {
                    error.kind == FieldErrorKind::Unavailable
                        && error.message.contains("volume is unavailable")
                }
            }
        }
        field_ok(&self.power)
            && field_ok(&self.input)
            && field_ok(&self.volume)
            && field_ok(&self.mute)
            && field_ok(&self.surround_mode)
    }

    pub fn value(&self, field: MainZoneField) -> Option<MainZoneValue> {
        match field {
            MainZoneField::Power => self.power.value().copied().map(MainZoneValue::Power),
            MainZoneField::Input => self.input.value().cloned().map(MainZoneValue::Input),
            MainZoneField::Volume => self.volume.value().cloned().map(MainZoneValue::Volume),
            MainZoneField::Mute => self.mute.value().copied().map(MainZoneValue::Mute),
            MainZoneField::SurroundMode => self
                .surround_mode
                .value()
                .cloned()
                .map(MainZoneValue::SurroundMode),
        }
    }

    pub fn resource_version(&self) -> u64 {
        self.resource_version
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainZoneEvent {
    Changed(MainZoneValue),
    Unknown(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Connected,
    Reconnecting,
    Disconnected,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reducer_preserves_partial_typed_state_and_authority() {
        let mut state = MainZoneSnapshot::default();
        state.set_value(
            MainZoneValue::Input(Input::new("CD").unwrap()),
            StateAuthority::Event,
        );
        assert_eq!(state.input.value().map(Input::as_str), Some("CD"));
        assert_eq!(state.authority, StateAuthority::Event);
        assert!(state.power.value().is_none());
        state.invalidate();
        assert_eq!(state.freshness, Freshness::Invalidated);
    }

    #[test]
    fn event_reducer_handles_unsolicited_and_authoritative_updates() {
        let mut state = MainZoneSnapshot::default();
        state.apply_event(MainZoneEvent::Changed(MainZoneValue::Power(PowerState::On)));
        assert_eq!(state.power.value(), Some(&PowerState::On));
        assert_eq!(state.authority, StateAuthority::Event);
        state
            .apply_authoritative_event(MainZoneEvent::Changed(MainZoneValue::Mute(MuteState::Off)));
        assert_eq!(state.mute.value(), Some(&MuteState::Off));
        assert_eq!(state.authority, StateAuthority::Authoritative);
    }

    #[test]
    fn volume_level_round_trips_native_endpoints_and_half_steps() {
        let minimum = VolumeLevel::new(0).unwrap();
        let midpoint = VolumeLevel::new(500).unwrap();
        let maximum = VolumeLevel::new(1000).unwrap();
        assert_eq!(minimum.to_native_code(), 0);
        assert_eq!(midpoint.to_native_code(), 495);
        assert_eq!(maximum.to_native_code(), 985);
        assert_eq!(VolumeLevel::from_native_code(0).unwrap(), minimum);
        assert_eq!(VolumeLevel::from_native_code(985).unwrap(), maximum);
        assert_eq!(
            VolumeLevel::from_native_code(245).unwrap().to_native_code(),
            245
        );
        assert_eq!(Volume::from_parts("800", 0).native_code(), 800);
        assert_eq!(Volume::from_parts("80", 0).native_code(), 800);
        assert!(VolumeLevel::new(501).is_err());
    }
}
