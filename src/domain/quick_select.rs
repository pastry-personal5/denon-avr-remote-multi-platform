//! Main Zone Quick Select observations and operations.

use super::{Input, SurroundMode, VolumeLevel};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QuickSelectSlot(u8);

impl QuickSelectSlot {
    pub const ALL: [Self; 4] = [Self(1), Self(2), Self(3), Self(4)];
    pub fn new(slot: u8) -> Result<Self, &'static str> {
        (1..=4)
            .contains(&slot)
            .then_some(Self(slot))
            .ok_or("Quick Select slot must be 1 through 4")
    }
    pub const fn number(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickSelectName(String);
impl QuickSelectName {
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        let value = value.trim();
        if value.is_empty() {
            Err("Quick Select name must not be empty")
        } else if value.chars().count() > 16 {
            Err("Quick Select name must not exceed 16 characters")
        } else if value.chars().any(char::is_control) {
            Err("Quick Select name must not contain control characters")
        } else {
            Ok(Self(value.to_owned()))
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Registered<T> {
    Included(T),
    Omitted,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QuickSelectSummary {
    pub input: Registered<Input>,
    pub volume: Registered<VolumeLevel>,
    pub sound_mode: Registered<SurroundMode>,
    pub channel_levels: Registered<String>,
    pub audyssey: Registered<String>,
    pub restorer: Registered<String>,
    pub dialog_enhancer: Registered<String>,
    pub hdmi_video_output: Registered<String>,
    pub speaker_preset: Registered<String>,
    pub dirac_live: Registered<String>,
    pub playback_content: Registered<String>,
    pub all_zone_stereo: Registered<String>,
    pub tv_audio_sharing: Registered<String>,
    pub video_select: Registered<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickSelectPreset {
    pub slot: QuickSelectSlot,
    pub name: Option<QuickSelectName>,
    pub available: bool,
    pub summary: QuickSelectSummary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickSelectSnapshot {
    pub presets: [Option<QuickSelectPreset>; 4],
    pub freshness: super::Freshness,
    pub generation: u64,
    resource_version: u64,
}
impl Default for QuickSelectSnapshot {
    fn default() -> Self {
        Self {
            presets: [None, None, None, None],
            freshness: super::Freshness::Unknown,
            generation: 0,
            resource_version: 0,
        }
    }
}
impl QuickSelectSnapshot {
    pub fn preset(&self, slot: QuickSelectSlot) -> Option<&QuickSelectPreset> {
        self.presets[slot.number() as usize - 1].as_ref()
    }
    pub fn invalidate(&mut self) {
        self.resource_version = self.resource_version.saturating_add(1);
        self.presets = [None, None, None, None];
        self.freshness = super::Freshness::Invalidated;
    }
    pub fn set(&mut self, preset: QuickSelectPreset) {
        let index = preset.slot.number() as usize - 1;
        self.presets[index] = Some(preset);
        self.resource_version = self.resource_version.saturating_add(1);
        self.freshness = super::Freshness::Live;
    }
    pub fn resource_version(&self) -> u64 {
        self.resource_version
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuickSelectRecallOutcome {
    Pending { slot: QuickSelectSlot },
    Confirmed { slot: QuickSelectSlot },
    Rejected(String),
    Conflict { expected: u64, current: u64 },
    Unsupported(String),
    TransportFailure(String),
    Unconfirmed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickSelectRecallConfirmation {
    Authoritative,
    Dispatched,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slots_are_exactly_main_zone_slots() {
        assert!(QuickSelectSlot::new(0).is_err());
        assert_eq!(QuickSelectSlot::ALL.len(), 4);
    }
    #[test]
    fn omitted_is_not_unknown() {
        assert_ne!(Registered::<String>::Omitted, Registered::Unknown);
    }

    #[test]
    fn names_follow_the_receivers_sixteen_character_limit() {
        assert_eq!(
            QuickSelectName::new("  Movie night  ").unwrap().as_str(),
            "Movie night"
        );
        assert!(QuickSelectName::new("12345678901234567").is_err());
        assert!(QuickSelectName::new("bad\nname").is_err());
    }
}
