//! Main zone domain types and state management.

use super::HttpInformationSnapshot;
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

/// Presentation-only organization for the receiver's individual sound modes.
/// It is deliberately not a control: selecting a mode always sends its exact
/// `MS…` value, never the old category-recall command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SoundModeCategory {
    Movie,
    Music,
    Game,
    Pure,
}

impl SoundModeCategory {
    pub const fn key(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Music => "music",
            Self::Game => "game",
            Self::Pure => "pure",
        }
    }

    pub fn from_key(value: &str) -> Option<Self> {
        match value {
            "movie" => Some(Self::Movie),
            "music" => Some(Self::Music),
            "game" => Some(Self::Game),
            "pure" => Some(Self::Pure),
            _ => None,
        }
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
    /// Select an individual table row. Its category remains part of the
    /// presentation state after the AVR confirms the exact detailed mode.
    SelectSoundMode {
        category: SoundModeCategory,
        mode: SurroundMode,
    },
    /// Recall the receiver-owned mode remembered for this Sound Mode group.
    /// The resulting detailed mode is confirmed from an authoritative `MS?`
    /// read rather than predicted by the application.
    RecallSoundModeCategory(SoundModeCategory),
}

/// The independent Zone 2 state. It is intentionally separate from the Main
/// Zone snapshot because Denon `Z2` commands do not target `PW`/Zone 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone2Snapshot {
    pub power: FieldStatus<PowerState>,
    pub freshness: Freshness,
    pub authority: StateAuthority,
}

impl Default for Zone2Snapshot {
    fn default() -> Self {
        Self {
            power: FieldStatus::Unavailable(not_queried()),
            freshness: Freshness::Unknown,
            authority: StateAuthority::Unconfirmed,
        }
    }
}

impl Zone2Snapshot {
    pub fn invalidate(&mut self) {
        *self = Self::default();
        self.freshness = Freshness::Invalidated;
    }
    pub fn set_power(&mut self, power: PowerState, authority: StateAuthority) {
        self.power = FieldStatus::Value(power);
        self.freshness = Freshness::Live;
        self.authority = authority;
    }
    pub fn set_error(&mut self, error: FieldError) {
        self.power = FieldStatus::Unavailable(error);
    }
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
}

impl Default for MainZoneSnapshot {
    fn default() -> Self {
        Self {
            http_information: HttpInformationSnapshot::default(),
            power: FieldStatus::Unavailable(not_queried()),
            input: FieldStatus::Unavailable(not_queried()),
            volume: FieldStatus::Unavailable(not_queried()),
            mute: FieldStatus::Unavailable(not_queried()),
            surround_mode: FieldStatus::Unavailable(not_queried()),
            freshness: Freshness::Unknown,
            authority: StateAuthority::Unconfirmed,
        }
    }
}

impl MainZoneSnapshot {
    pub fn invalidate(&mut self) {
        *self = Self::default();
        self.freshness = Freshness::Invalidated;
        self.http_information.invalidate(0);
    }

    pub fn invalidate_http_information(&mut self, generation: u64) {
        self.http_information.invalidate(generation);
    }

    pub fn set_http_information(&mut self, information: HttpInformationSnapshot) {
        self.http_information = information;
    }

    pub fn set_value(&mut self, value: MainZoneValue, authority: StateAuthority) {
        match value {
            MainZoneValue::Power(value) => self.power = FieldStatus::Value(value),
            MainZoneValue::Input(value) => self.input = FieldStatus::Value(value),
            MainZoneValue::Volume(value) => self.volume = FieldStatus::Value(value),
            MainZoneValue::Mute(value) => self.mute = FieldStatus::Value(value),
            MainZoneValue::SurroundMode(value) => self.surround_mode = FieldStatus::Value(value),
        }
        if matches!(self.power, FieldStatus::Value(PowerState::Standby)) {
            self.http_information.invalidate(0);
        }
        self.freshness = Freshness::Live;
        self.authority = authority;
    }

    pub fn set_error(&mut self, field: MainZoneField, error: FieldError) {
        match field {
            MainZoneField::Power => self.power = FieldStatus::Unavailable(error),
            MainZoneField::Input => self.input = FieldStatus::Unavailable(error),
            MainZoneField::Volume => self.volume = FieldStatus::Unavailable(error),
            MainZoneField::Mute => self.mute = FieldStatus::Unavailable(error),
            MainZoneField::SurroundMode => self.surround_mode = FieldStatus::Unavailable(error),
        }
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
    fn a_standby_reading_invalidates_the_http_information() {
        let mut snapshot = MainZoneSnapshot::default();
        snapshot.set_http_information(HttpInformationSnapshot {
            generation: 3,
            freshness: Freshness::Live,
            ..HttpInformationSnapshot::default()
        });
        snapshot.set_value(
            MainZoneValue::Power(PowerState::On),
            StateAuthority::Authoritative,
        );
        assert_eq!(snapshot.http_information.freshness, Freshness::Live);
        snapshot.set_value(
            MainZoneValue::Power(PowerState::Standby),
            StateAuthority::Authoritative,
        );
        assert_eq!(snapshot.http_information.freshness, Freshness::Invalidated);
    }

    #[test]
    fn a_field_error_is_unavailable_and_keeps_the_others() {
        let mut snapshot = MainZoneSnapshot::default();
        snapshot.set_value(
            MainZoneValue::Mute(MuteState::Off),
            StateAuthority::Authoritative,
        );
        snapshot.set_error(
            MainZoneField::Volume,
            FieldError {
                kind: FieldErrorKind::Timeout,
                message: "late".into(),
            },
        );
        assert_eq!(snapshot.mute.value(), Some(&MuteState::Off));
        assert!(snapshot.volume.value().is_none());
        assert_eq!(
            snapshot.value(MainZoneField::Mute),
            Some(MainZoneValue::Mute(MuteState::Off))
        );
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
