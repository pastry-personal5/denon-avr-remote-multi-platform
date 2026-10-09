//! The receiver configuration: saved identities, the configuration as `GET /v1/config`
//! answers it, and the strict shape `PUT /v1/config` is sent.

use denon_avr_domain::{ConfiguredReceivers, ReceiverIdentity, SoundModeFavorite};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A saved receiver: where it is, and what it calls itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityDto {
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub friendly_name: Option<String>,
}

impl From<&ReceiverIdentity> for IdentityDto {
    fn from(identity: &ReceiverIdentity) -> Self {
        Self {
            host: identity.host.clone(),
            model: identity.model.clone(),
            friendly_name: identity.friendly_name.clone(),
        }
    }
}

impl From<IdentityDto> for ReceiverIdentity {
    fn from(identity: IdentityDto) -> Self {
        Self {
            host: identity.host,
            model: identity.model,
            friendly_name: identity.friendly_name,
        }
    }
}

/// The receiver configuration, as `GET /v1/config` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    #[serde(default)]
    pub receivers: BTreeMap<String, IdentityDto>,
    #[serde(default)]
    pub sound_mode_favorites: BTreeMap<String, Vec<String>>,
}

impl From<&ConfiguredReceivers> for ConfigDto {
    fn from(config: &ConfiguredReceivers) -> Self {
        Self {
            current: config.current.clone(),
            receivers: config
                .receivers
                .iter()
                .map(|(name, identity)| (name.clone(), identity.into()))
                .collect(),
            sound_mode_favorites: config
                .sound_mode_favorites
                .iter()
                .map(|(name, favorites)| {
                    (
                        name.clone(),
                        favorites
                            .iter()
                            .map(|favorite| favorite.mode.clone())
                            .collect(),
                    )
                })
                .collect(),
        }
    }
}

impl TryFrom<ConfigDto> for ConfiguredReceivers {
    type Error = String;

    fn try_from(config: ConfigDto) -> Result<Self, String> {
        let favorites = config
            .sound_mode_favorites
            .into_iter()
            .map(|(name, modes)| {
                let set = modes
                    .into_iter()
                    .map(|mode| SoundModeFavorite::new(mode).map_err(str::to_owned))
                    .collect::<Result<BTreeSet<_>, String>>()?;
                Ok((name, set))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        Ok(Self {
            current: config.current,
            receivers: config
                .receivers
                .into_iter()
                .map(|(name, identity)| (name, identity.into()))
                .collect(),
            sound_mode_favorites: favorites,
        })
    }
}

/// A saved receiver, as `PUT /v1/config` is sent it: strict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityRequest {
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub friendly_name: Option<String>,
}

/// The configuration, as `PUT /v1/config` is sent it. It is the same shape as the
/// answer, but a field it does not know is refused by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    #[serde(default)]
    pub receivers: BTreeMap<String, IdentityRequest>,
    #[serde(default)]
    pub sound_mode_favorites: BTreeMap<String, Vec<String>>,
}

impl From<&ConfiguredReceivers> for ConfigRequest {
    fn from(config: &ConfiguredReceivers) -> Self {
        let dto = ConfigDto::from(config);
        Self {
            current: dto.current,
            receivers: dto
                .receivers
                .into_iter()
                .map(|(name, identity)| {
                    (
                        name,
                        IdentityRequest {
                            host: identity.host,
                            model: identity.model,
                            friendly_name: identity.friendly_name,
                        },
                    )
                })
                .collect(),
            sound_mode_favorites: dto.sound_mode_favorites,
        }
    }
}

impl TryFrom<ConfigRequest> for ConfiguredReceivers {
    type Error = String;

    fn try_from(request: ConfigRequest) -> Result<Self, String> {
        ConfigDto {
            current: request.current,
            receivers: request
                .receivers
                .into_iter()
                .map(|(name, identity)| {
                    (
                        name,
                        IdentityDto {
                            host: identity.host,
                            model: identity.model,
                            friendly_name: identity.friendly_name,
                        },
                    )
                })
                .collect(),
            sound_mode_favorites: request.sound_mode_favorites,
        }
        .try_into()
    }
}
