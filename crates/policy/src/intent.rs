//! What a rule can name: the kind of an intent and, for power and mute, a value.

use denon_avr_domain::{MuteState, ReceiverIntent, SystemPower, ZonePower};

/// The kind of a receiver intent, without its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntentKind {
    SystemPower,
    MainZonePower,
    Zone2Power,
    Source,
    Volume,
    Mute,
    SoundMode,
}

impl IntentKind {
    pub const ALL: [IntentKind; 7] = [
        Self::SystemPower,
        Self::MainZonePower,
        Self::Zone2Power,
        Self::Source,
        Self::Volume,
        Self::Mute,
        Self::SoundMode,
    ];

    /// The kind of `intent`. The match has no wildcard arm, so a new intent
    /// does not compile until it is mapped here.
    pub fn of(intent: &ReceiverIntent) -> Self {
        match intent {
            ReceiverIntent::SystemPower(_) => Self::SystemPower,
            ReceiverIntent::MainZonePower(_) => Self::MainZonePower,
            ReceiverIntent::Zone2Power(_) => Self::Zone2Power,
            ReceiverIntent::Source(_) => Self::Source,
            ReceiverIntent::Volume(_) => Self::Volume,
            ReceiverIntent::Mute(_) => Self::Mute,
            ReceiverIntent::SoundMode(_) => Self::SoundMode,
        }
    }

    /// The name a policy file uses.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SystemPower => "system_power",
            Self::MainZonePower => "main_zone_power",
            Self::Zone2Power => "zone2_power",
            Self::Source => "source",
            Self::Volume => "volume",
            Self::Mute => "mute",
            Self::SoundMode => "sound_mode",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }

    /// Whether a rule may name `value` for this kind.
    pub fn accepts(self, value: IntentValue) -> bool {
        match self {
            Self::SystemPower => matches!(value, IntentValue::On | IntentValue::Standby),
            Self::MainZonePower | Self::Zone2Power | Self::Mute => {
                matches!(value, IntentValue::On | IntentValue::Off)
            }
            Self::Source | Self::Volume | Self::SoundMode => false,
        }
    }
}

/// The value of a power or mute intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntentValue {
    On,
    Off,
    Standby,
}

impl IntentValue {
    /// The value of `intent`, for the kinds that have one.
    pub fn of(intent: &ReceiverIntent) -> Option<Self> {
        match intent {
            ReceiverIntent::SystemPower(SystemPower::On) => Some(Self::On),
            ReceiverIntent::SystemPower(SystemPower::Standby) => Some(Self::Standby),
            ReceiverIntent::MainZonePower(power) | ReceiverIntent::Zone2Power(power) => {
                Some(match power {
                    ZonePower::On => Self::On,
                    ZonePower::Off => Self::Off,
                })
            }
            ReceiverIntent::Mute(MuteState::On) => Some(Self::On),
            ReceiverIntent::Mute(MuteState::Off) => Some(Self::Off),
            ReceiverIntent::Source(_)
            | ReceiverIntent::Volume(_)
            | ReceiverIntent::SoundMode(_) => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
            Self::Standby => "standby",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        [Self::On, Self::Off, Self::Standby]
            .into_iter()
            .find(|value| value.as_str() == name)
    }
}
