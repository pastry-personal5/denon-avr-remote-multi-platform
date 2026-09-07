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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    code: String,
    db_tenths: i16,
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
    }

    pub fn set_value(&mut self, value: MainZoneValue, authority: StateAuthority) {
        match value {
            MainZoneValue::Power(value) => self.power = FieldStatus::Value(value),
            MainZoneValue::Input(value) => self.input = FieldStatus::Value(value),
            MainZoneValue::Volume(value) => self.volume = FieldStatus::Value(value),
            MainZoneValue::Mute(value) => self.mute = FieldStatus::Value(value),
            MainZoneValue::SurroundMode(value) => self.surround_mode = FieldStatus::Value(value),
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
}
