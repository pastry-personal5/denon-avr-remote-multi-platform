//! `server.yaml`, the server's own settings, in the data directory.
//!
//! Its absence is a valid file, and the server then has an Operator endpoint and
//! nothing else: the Agent endpoint exists only because the owner wrote it down.
//! A key the file does not know is refused by name, so a typo cannot leave an
//! endpoint quietly unconfigured, and an error names the key and where it is and
//! never repeats a value.
//!
//! ```yaml
//! agent_endpoint:
//!   directory: /Users/Shared/Denon AVR Remote   # optional; this is the default
//!   uids: [503]                                 # required: the accounts admitted
//! ```
//!
//! The socket's mode is not a setting. It is `0600`: the directory's ACL admits the
//! account, and the server checks the peer's uid and the request's token besides.

use crate::start::AgentEndpointConfig;
use serde::{Deserialize, Deserializer};
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const SETTINGS_FILE: &str = "server.yaml";

/// Where the Agent endpoint's socket is when the file does not say. It is
/// `/Users/Shared`, which is sticky and writable by everyone on macOS, so the owner
/// can make the directory without `sudo` and the server checks it before it binds.
pub const DEFAULT_AGENT_DIRECTORY: &str = "/Users/Shared/Denon AVR Remote";

/// The largest settings file that loads, in bytes.
const MAX_BYTES: u64 = 64 * 1024;

/// What the file asks for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    pub agent_endpoint: Option<AgentEndpointSettings>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentEndpointSettings {
    pub directory: PathBuf,
    /// The accounts whose connections are served. Never empty.
    pub uids: Vec<u32>,
}

impl Settings {
    /// The Agent endpoint as the server takes it, when the file asks for one.
    pub fn agent_config(&self) -> Option<AgentEndpointConfig> {
        self.agent_endpoint
            .as_ref()
            .map(|settings| AgentEndpointConfig {
                directory: settings.directory.clone(),
                uids: settings.uids.clone(),
                mode: 0o600,
            })
    }
}

/// Why the settings did not load.
#[derive(Debug)]
pub enum SettingsError {
    Unreadable { path: PathBuf, why: String },
    Invalid { path: PathBuf, why: String },
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { path, why } => {
                write!(f, "{}: cannot be read: {why}", path.display())
            }
            Self::Invalid { path, why } => write!(f, "{}: {why}", path.display()),
        }
    }
}

impl std::error::Error for SettingsError {}

/// Read the settings at `path`. A file that is not there is no settings.
pub fn load(path: &Path) -> Result<Settings, SettingsError> {
    let unreadable = |why: String| SettingsError::Unreadable {
        path: path.to_owned(),
        why,
    };
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Settings::default())
        }
        Err(error) => return Err(unreadable(error.to_string())),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| unreadable(error.to_string()))?;
    let invalid = |why: String| SettingsError::Invalid {
        path: path.to_owned(),
        why,
    };
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid(format!("is larger than {} KiB", MAX_BYTES / 1024)));
    }
    let text = String::from_utf8(bytes).map_err(|_| invalid("is not text".into()))?;
    parse(&text).map_err(invalid)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    /// `Some(None)` is the key written with nothing after it.
    #[serde(default, deserialize_with = "present")]
    agent_endpoint: Option<Option<Raw>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    directory: Option<String>,
    uids: Option<Vec<u32>>,
}

fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Option<Raw>>, D::Error> {
    Option::<Raw>::deserialize(deserializer).map(Some)
}

/// Read settings from text. The error says what is wrong and never quotes a value.
pub fn parse(text: &str) -> Result<Settings, String> {
    let file = serde_yaml::from_str::<Option<File>>(text)
        .map_err(|error| describe(&error))?
        .unwrap_or(File {
            agent_endpoint: None,
        });
    let Some(raw) = file.agent_endpoint else {
        return Ok(Settings::default());
    };
    let raw = raw.ok_or("agent_endpoint needs `uids`, the accounts it admits")?;
    let uids = raw
        .uids
        .ok_or("agent_endpoint needs `uids`, the accounts it admits")?;
    if uids.is_empty() {
        return Err("agent_endpoint `uids` is empty: it must name the accounts admitted".into());
    }
    let directory = PathBuf::from(
        raw.directory
            .unwrap_or_else(|| DEFAULT_AGENT_DIRECTORY.to_owned()),
    );
    if !directory.is_absolute() {
        return Err("agent_endpoint `directory` must be an absolute path".into());
    }
    Ok(Settings {
        agent_endpoint: Some(AgentEndpointSettings { directory, uids }),
    })
}

/// What serde_yaml found wrong, without the values it saw.
fn describe(error: &serde_yaml::Error) -> String {
    let at = error
        .location()
        .map(|place| format!(" (line {}, column {})", place.line(), place.column()))
        .unwrap_or_default();
    let text = error.to_string();
    for (prefix, words) in [
        ("unknown field `", "unknown key"),
        ("missing field `", "missing key"),
    ] {
        if let Some(start) = text.find(prefix) {
            let rest = &text[start + prefix.len()..];
            if let Some(end) = rest.find('`') {
                let name: String = rest[..end].chars().take(64).collect();
                return format!("{words} `{name}`{at}");
            }
        }
    }
    format!("a setting is not what the file's format allows{at}")
}
