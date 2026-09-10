//! Receiver identity and configuration types.

use super::SoundModeCategory;
use std::collections::{BTreeMap, BTreeSet};

/// A single row in the Sound Mode table. The category is part of the identity
/// because one detailed mode can intentionally appear under several groups.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SoundModeFavorite {
    pub category: SoundModeCategory,
    pub mode: String,
}

impl SoundModeFavorite {
    pub fn new(category: SoundModeCategory, mode: impl Into<String>) -> Result<Self, &'static str> {
        let mode = mode.into();
        if mode.trim().is_empty() {
            return Err("sound mode favorite must not be empty");
        }
        Ok(Self { category, mode })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverIdentity {
    pub host: String,
    pub model: Option<String>,
    pub friendly_name: Option<String>,
}

impl ReceiverIdentity {
    pub fn ad_hoc(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            model: None,
            friendly_name: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConfiguredReceivers {
    pub current: Option<String>,
    pub receivers: BTreeMap<String, ReceiverIdentity>,
    /// User-owned, per-receiver sound mode favorites. Receiver state never
    /// writes this collection; it is presentation preference only.
    pub sound_mode_favorites: BTreeMap<String, BTreeSet<SoundModeFavorite>>,
}

impl ConfiguredReceivers {
    pub fn current(&self) -> Option<(&str, &ReceiverIdentity)> {
        let name = self.current.as_deref()?;
        self.receivers.get(name).map(|identity| (name, identity))
    }

    pub fn validate(&self) -> Result<(), String> {
        for (name, receiver) in &self.receivers {
            if name.trim().is_empty() {
                return Err("receiver name must not be empty".into());
            }
            if receiver.host.trim().is_empty() {
                return Err(format!("receiver {name} has an empty host"));
            }
            if name.contains(['\r', '\n']) || receiver.host.contains(['\r', '\n']) {
                return Err(format!("receiver {name} contains a line break"));
            }
        }
        if let Some(current) = &self.current {
            if !self.receivers.contains_key(current) {
                return Err(format!("current receiver {current} is not configured"));
            }
        }
        if let Some(name) = self
            .sound_mode_favorites
            .keys()
            .find(|name| !self.receivers.contains_key(*name))
        {
            return Err(format!(
                "sound mode favorites reference unknown receiver {name}"
            ));
        }
        Ok(())
    }

    pub fn is_sound_mode_favorite(&self, category: SoundModeCategory, mode: &str) -> bool {
        self.current
            .as_ref()
            .and_then(|name| self.sound_mode_favorites.get(name))
            .is_some_and(|favorites| {
                favorites.contains(&SoundModeFavorite {
                    category,
                    mode: mode.to_owned(),
                })
            })
    }

    pub fn toggle_current_sound_mode_favorite(
        &mut self,
        category: SoundModeCategory,
        mode: &str,
    ) -> Result<bool, String> {
        let name = self
            .current
            .clone()
            .ok_or("select and save a receiver before setting favorites")?;
        if !self.receivers.contains_key(&name) {
            return Err("current receiver is not configured".into());
        }
        let favorites = self.sound_mode_favorites.entry(name.clone()).or_default();
        let favorite = SoundModeFavorite::new(category, mode).map_err(str::to_owned)?;
        let favorite = if favorites.contains(&favorite) {
            favorites.remove(&favorite);
            false
        } else {
            favorites.insert(favorite);
            true
        };
        if favorites.is_empty() {
            self.sound_mode_favorites.remove(&name);
        }
        Ok(favorite)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverEndpoint {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredReceiver {
    pub address: ReceiverEndpoint,
    pub location: Option<String>,
    pub server: Option<String>,
    pub model: Option<String>,
    pub search_target: Option<String>,
    pub unique_service_name: Option<String>,
}

impl DiscoveredReceiver {
    pub fn identity(&self) -> ReceiverIdentity {
        ReceiverIdentity {
            host: self.address.host.clone(),
            model: self.model.clone(),
            friendly_name: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_mode_favorites_are_scoped_to_the_current_receiver() {
        let mut configured = ConfiguredReceivers {
            current: Some("living-room".into()),
            receivers: BTreeMap::from([(
                "living-room".into(),
                ReceiverIdentity::ad_hoc("192.0.2.10"),
            )]),
            ..ConfiguredReceivers::default()
        };

        assert!(configured
            .toggle_current_sound_mode_favorite(SoundModeCategory::Movie, "DTS NEURAL:X")
            .unwrap());
        assert!(configured.is_sound_mode_favorite(SoundModeCategory::Movie, "DTS NEURAL:X"));
        assert!(!configured.is_sound_mode_favorite(SoundModeCategory::Music, "DTS NEURAL:X"));
        assert!(!configured
            .toggle_current_sound_mode_favorite(SoundModeCategory::Movie, "DTS NEURAL:X")
            .unwrap());
        assert!(!configured.is_sound_mode_favorite(SoundModeCategory::Movie, "DTS NEURAL:X"));
        assert!(configured.sound_mode_favorites.is_empty());
    }
}
