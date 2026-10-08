//! The small value types the other wire types are made of.

use denon_avr_domain::{MasterVolume, MuteState, SoundModeIntent, SystemPower, ZonePower};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemPowerDto {
    On,
    Standby,
}

impl From<SystemPower> for SystemPowerDto {
    fn from(value: SystemPower) -> Self {
        match value {
            SystemPower::On => Self::On,
            SystemPower::Standby => Self::Standby,
        }
    }
}

impl From<SystemPowerDto> for SystemPower {
    fn from(value: SystemPowerDto) -> Self {
        match value {
            SystemPowerDto::On => Self::On,
            SystemPowerDto::Standby => Self::Standby,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZonePowerDto {
    On,
    Off,
}

impl From<ZonePower> for ZonePowerDto {
    fn from(value: ZonePower) -> Self {
        match value {
            ZonePower::On => Self::On,
            ZonePower::Off => Self::Off,
        }
    }
}

impl From<ZonePowerDto> for ZonePower {
    fn from(value: ZonePowerDto) -> Self {
        match value {
            ZonePowerDto::On => Self::On,
            ZonePowerDto::Off => Self::Off,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MuteDto {
    On,
    Off,
}

impl From<MuteState> for MuteDto {
    fn from(value: MuteState) -> Self {
        match value {
            MuteState::On => Self::On,
            MuteState::Off => Self::Off,
        }
    }
}

impl From<MuteDto> for MuteState {
    fn from(value: MuteDto) -> Self {
        match value {
            MuteDto::On => Self::On,
            MuteDto::Off => Self::Off,
        }
    }
}

/// A master volume: the receiver's `minimum`, or a level in half decibels from
/// -159 (-79.5 dB) to 36 (+18.0 dB). Written as `"minimum"` or
/// `{"half_steps": -71}`, so it cannot be both or neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumeDto {
    Minimum,
    HalfSteps(i16),
}

impl From<MasterVolume> for VolumeDto {
    fn from(value: MasterVolume) -> Self {
        match value {
            MasterVolume::Minimum => Self::Minimum,
            MasterVolume::DbHalfSteps(steps) => Self::HalfSteps(steps),
        }
    }
}

impl TryFrom<VolumeDto> for MasterVolume {
    type Error = &'static str;

    fn try_from(value: VolumeDto) -> Result<Self, Self::Error> {
        match value {
            VolumeDto::Minimum => Ok(Self::Minimum),
            VolumeDto::HalfSteps(steps) => Self::db_half_steps(steps),
        }
    }
}

/// A sound mode: one of the named recalls and modes, or `{"select": "name"}` for
/// a mode chosen by the receiver's own name for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundModeDto {
    Auto,
    Direct,
    PureDirect,
    Stereo,
    RecallMovie,
    RecallMusic,
    RecallGame,
    Select(String),
}

impl From<&SoundModeIntent> for SoundModeDto {
    fn from(value: &SoundModeIntent) -> Self {
        match value {
            SoundModeIntent::Auto => Self::Auto,
            SoundModeIntent::Direct => Self::Direct,
            SoundModeIntent::PureDirect => Self::PureDirect,
            SoundModeIntent::Stereo => Self::Stereo,
            SoundModeIntent::RecallMovie => Self::RecallMovie,
            SoundModeIntent::RecallMusic => Self::RecallMusic,
            SoundModeIntent::RecallGame => Self::RecallGame,
            SoundModeIntent::Select(name) => Self::Select(name.clone()),
        }
    }
}

impl From<SoundModeDto> for SoundModeIntent {
    fn from(value: SoundModeDto) -> Self {
        match value {
            SoundModeDto::Auto => Self::Auto,
            SoundModeDto::Direct => Self::Direct,
            SoundModeDto::PureDirect => Self::PureDirect,
            SoundModeDto::Stereo => Self::Stereo,
            SoundModeDto::RecallMovie => Self::RecallMovie,
            SoundModeDto::RecallMusic => Self::RecallMusic,
            SoundModeDto::RecallGame => Self::RecallGame,
            SoundModeDto::Select(name) => Self::Select(name),
        }
    }
}
