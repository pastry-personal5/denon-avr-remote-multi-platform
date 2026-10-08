//! The rule for an agent label.
//!
//! A label names one agent in the audit log and in policy. A token is issued under
//! a label and a policy rule can be narrowed to labels, and the two meet by exact,
//! case-sensitive comparison. Holding both to one rule means a rule cannot name a
//! label that no token could ever carry, which would apply to nobody and read as
//! though it restricted somebody.

/// The longest label, in characters.
pub const MAX_LABEL_LEN: usize = 64;

/// A label the audit log uses for a caller with no valid credential, so no token
/// can be issued under it.
pub const RESERVED_LABEL: &str = "unauthenticated";

/// Whether `label` can be the label of a token: 1 to 64 characters of lowercase
/// letters, digits, `.`, `_`, and `-`, starting with a letter or a digit, and not
/// the reserved word.
pub fn label_is_well_formed(label: &str) -> bool {
    if label.is_empty() || label.len() > MAX_LABEL_LEN || label == RESERVED_LABEL {
        return false;
    }
    let mut characters = label.chars();
    let starts_well = characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit());
    starts_well
        && label
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}
