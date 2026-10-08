//! Where the policy file comes from.
//!
//! The control service holds the policy it loaded and identifies it by the
//! digest of the bytes it read, so the Operator, the audit log, and a dry run
//! can all say which file a decision rested on.

use crate::ports::BoxFuture;
use denon_avr_policy::PolicyConfig;
use std::fmt;
use std::sync::Arc;

/// A policy that loaded: the rules in force, the digest of the bytes they came
/// from, and the text that was read, which the Operator's view shows.
#[derive(Debug, Clone)]
pub struct LoadedPolicy {
    pub config: PolicyConfig,
    pub digest: PolicyDigest,
    pub text: String,
}

/// Why no policy loaded. The text names the rule or key at fault and never
/// quotes the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyLoadError {
    Missing,
    Unreadable(String),
    Invalid(String),
}

impl fmt::Display for PolicyLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str("there is no policy file"),
            Self::Unreadable(why) => write!(f, "the policy file cannot be read: {why}"),
            Self::Invalid(why) => write!(f, "the policy file is invalid: {why}"),
        }
    }
}

impl std::error::Error for PolicyLoadError {}

/// Where the policy comes from. Each call reads it again, so a reload sees an
/// edit.
pub trait PolicySource: Send + Sync {
    fn load(&self) -> BoxFuture<'_, Result<LoadedPolicy, PolicyLoadError>>;
}

pub type SharedPolicySource = Arc<dyn PolicySource>;

/// The SHA-256 of the policy file's bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PolicyDigest([u8; 32]);

impl PolicyDigest {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Read the 64 lowercase hex digits [`Display`](fmt::Display) prints.
    pub fn from_hex(text: &str) -> Option<Self> {
        if text.len() != 64 {
            return None;
        }
        let digits = text.as_bytes();
        let mut bytes = [0u8; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = hex_digit(digits[2 * index])? << 4 | hex_digit(digits[2 * index + 1])?;
        }
        Some(Self(bytes))
    }
}

fn hex_digit(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        _ => None,
    }
}

impl fmt::Display for PolicyDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

impl fmt::Debug for PolicyDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PolicyDigest({self})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_digest_prints_as_64_lowercase_hex_digits_and_reads_back() {
        let mut bytes = [0u8; 32];
        bytes[0] = 0xab;
        bytes[31] = 0x07;
        let digest = PolicyDigest::from_bytes(bytes);
        let text = digest.to_string();
        assert_eq!(text.len(), 64);
        assert!(text.starts_with("ab") && text.ends_with("07"));
        assert!(text
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
        assert_eq!(PolicyDigest::from_hex(&text), Some(digest));
    }

    #[test]
    fn a_load_error_says_what_failed_without_the_files_contents() {
        assert_eq!(
            PolicyLoadError::Missing.to_string(),
            "there is no policy file"
        );
        assert_eq!(
            PolicyLoadError::Unreadable("permission denied".into()).to_string(),
            "the policy file cannot be read: permission denied"
        );
        assert_eq!(
            PolicyLoadError::Invalid("rule \"volume-step\": must be a multiple of 0.5 dB".into())
                .to_string(),
            "the policy file is invalid: rule \"volume-step\": must be a multiple of 0.5 dB"
        );
    }

    #[tokio::test]
    async fn a_policy_source_is_a_usable_object() {
        struct Fixed;
        impl PolicySource for Fixed {
            fn load(&self) -> BoxFuture<'_, Result<LoadedPolicy, PolicyLoadError>> {
                Box::pin(async {
                    Ok(LoadedPolicy {
                        config: denon_avr_policy::PolicyConfig::new(
                            Vec::new(),
                            std::time::Duration::from_secs(300),
                        )
                        .unwrap(),
                        digest: PolicyDigest::from_bytes([1; 32]),
                        text: String::new(),
                    })
                })
            }
        }
        let source: SharedPolicySource = std::sync::Arc::new(Fixed);
        let loaded = source.load().await.unwrap();
        assert!(loaded.config.rules().is_empty());
        assert_eq!(loaded.digest, PolicyDigest::from_bytes([1; 32]));
    }

    #[test]
    fn hex_that_is_not_a_digest_is_refused() {
        for bad in [
            "",
            "ab",
            &"g".repeat(64),
            &"A".repeat(64),
            &"a".repeat(63),
            &"a".repeat(65),
        ] {
            assert_eq!(PolicyDigest::from_hex(bad), None, "{bad:?}");
        }
    }
}
