//! Where the policy file comes from.
//!
//! The control service holds the policy it loaded and identifies it by the
//! digest of the bytes it read, so the Operator, the audit log, and a dry run
//! can all say which file a decision rested on.

use std::fmt;

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
