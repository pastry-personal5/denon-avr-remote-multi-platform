//! The on-disk configuration schemas and the pure mapping between their text and
//! the domain's configured receivers. No filesystem access lives here.

use denon_avr_application::ports::{OperationError, OperationErrorKind};
use denon_avr_domain::{ConfiguredReceivers, ReceiverIdentity, SoundModeFavorite};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The schema version this release writes.
const CURRENT_VERSION: u32 = 2;

/// The single-receiver file written by earlier releases.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LegacyConfigFile {
    receiver: ReceiverRecord,
    #[serde(default, skip_serializing_if = "SoundModeFavoritesFile::is_empty")]
    sound_mode_favorites: SoundModeFavoritesFile,
}

/// The multi-receiver file. Map keys are the receiver ids.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    current: Option<String>,
    receivers: BTreeMap<String, ReceiverRecord>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    sound_mode_favorites: BTreeMap<String, BTreeSet<String>>,
}

/// The canonical form is one detailed-mode list per receiver. Category-keyed
/// files written by the prior UI are accepted so no favorite is lost during
/// migration; their category labels are deliberately discarded.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum SoundModeFavoritesFile {
    CategoryKeyed(BTreeMap<String, BTreeMap<String, BTreeSet<String>>>),
    Simple(BTreeMap<String, BTreeSet<String>>),
}

impl Default for SoundModeFavoritesFile {
    fn default() -> Self {
        Self::Simple(BTreeMap::new())
    }
}

impl SoundModeFavoritesFile {
    fn is_empty(&self) -> bool {
        match self {
            Self::CategoryKeyed(favorites) => favorites.is_empty(),
            Self::Simple(favorites) => favorites.is_empty(),
        }
    }

    fn into_modes(self) -> BTreeMap<String, BTreeSet<String>> {
        match self {
            Self::CategoryKeyed(favorites) => favorites
                .into_iter()
                .map(|(receiver, categories)| {
                    (receiver, categories.into_values().flatten().collect())
                })
                .collect(),
            Self::Simple(favorites) => favorites,
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiverRecord {
    host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    friendly_name: Option<String>,
}
fn create_operation_error(
    kind: OperationErrorKind,
    context: &'static str,
    error: impl ToString,
) -> OperationError {
    OperationError::new(kind, context, error.to_string())
}

fn optional_text(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty())
}

pub(super) fn configuration_error(context: &'static str, error: impl ToString) -> OperationError {
    create_operation_error(OperationErrorKind::Configuration, context, error)
}

fn identity_of(record: ReceiverRecord) -> ReceiverIdentity {
    ReceiverIdentity {
        host: record.host,
        model: optional_text(record.model),
        friendly_name: optional_text(record.friendly_name),
    }
}

fn favorites_of(
    favorites: BTreeMap<String, BTreeSet<String>>,
) -> BTreeMap<String, BTreeSet<SoundModeFavorite>> {
    favorites
        .into_iter()
        .map(|(receiver, modes)| {
            let favorites = modes
                .into_iter()
                .filter_map(|mode| SoundModeFavorite::new(mode).ok())
                .collect();
            (receiver, favorites)
        })
        .collect()
}

/// Which schema a file is in, told by its top-level key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Schema {
    /// One receiver, under `receiver`.
    SingleReceiver,
    /// Several receivers, with a `version`.
    MultiReceiver,
}

pub(super) fn schema_of(text: &str) -> Result<Schema, OperationError> {
    let value: serde_yaml::Value = serde_yaml::from_str(text)
        .map_err(|error| configuration_error("parsing configuration", error))?;
    let mapping = value.as_mapping().ok_or_else(|| {
        configuration_error("parsing configuration", "the file is not a YAML mapping")
    })?;
    match (
        mapping.contains_key("version"),
        mapping.contains_key("receiver"),
    ) {
        (true, false) => Ok(Schema::MultiReceiver),
        (false, true) => Ok(Schema::SingleReceiver),
        (true, true) => Err(configuration_error(
            "parsing configuration",
            "the file has both `version` and `receiver`",
        )),
        (false, false) => Err(configuration_error(
            "parsing configuration",
            "the file has neither `version` nor `receiver`",
        )),
    }
}

/// Decode either schema. The single-receiver file becomes one entry named by
/// its friendly name, or `default` when it has none, as it always has.
pub(super) fn decode_text(text: &str) -> Result<ConfiguredReceivers, OperationError> {
    let config = match schema_of(text)? {
        Schema::SingleReceiver => {
            let file: LegacyConfigFile = serde_yaml::from_str(text)
                .map_err(|error| configuration_error("parsing configuration", error))?;
            let identity = identity_of(file.receiver);
            let name = identity
                .friendly_name
                .clone()
                .unwrap_or_else(|| "default".into());
            ConfiguredReceivers {
                current: Some(name.clone()),
                receivers: BTreeMap::from([(name, identity)]),
                sound_mode_favorites: favorites_of(file.sound_mode_favorites.into_modes()),
            }
        }
        Schema::MultiReceiver => {
            // A duplicate entry name is a parse error, not a silent overwrite.
            let file: ConfigFile = serde_yaml::from_str(text)
                .map_err(|error| configuration_error("parsing configuration", error))?;
            if file.version != CURRENT_VERSION {
                return Err(configuration_error(
                    "parsing configuration",
                    format!(
                        "configuration version {} is not supported; this release reads version {CURRENT_VERSION}",
                        file.version
                    ),
                ));
            }
            ConfiguredReceivers {
                current: file.current,
                receivers: file
                    .receivers
                    .into_iter()
                    .map(|(name, record)| (name, identity_of(record)))
                    .collect(),
                sound_mode_favorites: favorites_of(file.sound_mode_favorites),
            }
        }
    };
    config
        .validate()
        .map_err(|error| configuration_error("validating configuration", error))?;
    Ok(config)
}

pub(super) fn encode_text(config: &ConfiguredReceivers) -> Result<String, OperationError> {
    config
        .validate()
        .map_err(|error| configuration_error("validating configuration", error))?;
    let file = ConfigFile {
        version: CURRENT_VERSION,
        current: config.current.clone(),
        receivers: config
            .receivers
            .iter()
            .map(|(name, identity)| {
                (
                    name.clone(),
                    ReceiverRecord {
                        host: identity.host.clone(),
                        model: optional_text(identity.model.clone()),
                        friendly_name: optional_text(identity.friendly_name.clone()),
                    },
                )
            })
            .collect(),
        sound_mode_favorites: config
            .sound_mode_favorites
            .iter()
            .map(|(receiver, favorites)| {
                (
                    receiver.clone(),
                    favorites
                        .iter()
                        .map(|favorite| favorite.mode.clone())
                        .collect(),
                )
            })
            .collect(),
    };
    serde_yaml::to_string(&file)
        .map_err(|error| configuration_error("encoding configuration", error))
}
