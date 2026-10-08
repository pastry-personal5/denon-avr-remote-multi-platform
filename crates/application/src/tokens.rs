//! Agent tokens, as the control service and the Control API see them.
//!
//! A token is the credential an agent presents. The server keeps only a digest of
//! it, so what flows through these types is an identifier, a label, and, once, the
//! secret itself. The store that holds them is a port; the file-backed adapter
//! lives in infrastructure.

use crate::control::AgentLabel;
use crate::ports::BoxFuture;
use denon_avr_domain::WallTime;
use std::fmt;
use std::sync::Arc;
use tokio::sync::watch;

const ID_PREFIX: &str = "t-";
const ID_DIGITS: usize = 8;

/// The identifier of one token: `t-` and eight lowercase hexadecimal digits. It
/// names a token in listings and revocations and is not a secret.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TokenId(String);

impl TokenId {
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        let digits = value.strip_prefix(ID_PREFIX).unwrap_or("");
        if digits.len() != ID_DIGITS
            || !digits
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err("a token id is \"t-\" and eight lowercase hexadecimal digits");
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TokenId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A token's secret, shown once when it is issued. It never prints: `Debug` hides
/// it and there is no `Display`.
#[derive(Clone, PartialEq, Eq)]
pub struct TokenSecret(String);

impl TokenSecret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The secret itself, for the one place that hands it to its owner.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for TokenSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// What the Operator may read about a token. It holds no secret and no digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRecord {
    pub id: TokenId,
    pub label: AgentLabel,
    pub created: WallTime,
    /// When it was revoked. A revoked token never authenticates again.
    pub revoked: Option<WallTime>,
}

impl TokenRecord {
    pub fn is_active(&self) -> bool {
        self.revoked.is_none()
    }
}

/// A token as it is issued: the record, and the only copy of the secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedToken {
    pub record: TokenRecord,
    pub secret: TokenSecret,
}

/// Whose credential a presented string is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credential {
    /// An active Agent token, with its label and id.
    Agent(AgentLabel, TokenId),
    /// The Operator's token. It is valid only on the Operator endpoint.
    OperatorToken,
    /// Neither: unknown, revoked, or not a token at all.
    Unknown,
}

/// Why the store refused or failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenError {
    /// The label cannot be a token's label.
    InvalidLabel,
    /// The label already has an active token. Revoke it first.
    LabelInUse,
    /// There is no token with that id.
    NotFound,
    /// The store could not read or write its file. The text is for the Operator's
    /// log and never reaches an agent.
    Storage(String),
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLabel => f.write_str("that label cannot be a token's label"),
            Self::LabelInUse => {
                f.write_str("that label already has an active token; revoke it first")
            }
            Self::NotFound => f.write_str("no token has that id"),
            Self::Storage(why) => write!(f, "token store: {why}"),
        }
    }
}

impl std::error::Error for TokenError {}

/// The tokens the server has issued.
///
/// Reads and the credential check are synchronous and answer from memory, because
/// the server asks on every request. Issuing and revoking write a file and are
/// asynchronous.
pub trait TokenStore: Send + Sync {
    /// Issue a token under `label`. The label has already been checked against
    /// the label rule; the store checks it again and refuses a second active token
    /// for it.
    fn issue(
        &self,
        label: AgentLabel,
        now: WallTime,
    ) -> BoxFuture<'_, Result<IssuedToken, TokenError>>;

    /// Every token issued, revoked ones included, oldest first.
    fn list(&self) -> Vec<TokenRecord>;

    /// Revoke a token. Revoking one already revoked returns it as it is.
    fn revoke<'a>(
        &'a self,
        id: &'a TokenId,
        now: WallTime,
    ) -> BoxFuture<'a, Result<TokenRecord, TokenError>>;

    /// Whose credential `presented` is. It compares digests in constant time and
    /// never reports a revoked token as an Agent's.
    fn authenticate(&self, presented: &str) -> Credential;

    /// Whether the token exists and has not been revoked.
    fn is_active(&self, id: &TokenId) -> bool;

    /// A counter that rises on every revocation, so a stream or a wait holding a
    /// token can look again whether it is still valid.
    fn changes(&self) -> watch::Receiver<u64>;

    /// Why the store cannot issue or revoke tokens, when it cannot: its file could
    /// not be read. The Operator's own token does not depend on that file, so the
    /// Operator is still served. The text names a path and is for the log.
    fn fault(&self) -> Option<String> {
        None
    }
}

pub type SharedTokenStore = Arc<dyn TokenStore>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_id_prints_and_reads_back() {
        let id = TokenId::new("t-0a1b2c3d").unwrap();
        assert_eq!(id.to_string(), "t-0a1b2c3d");
        assert_eq!(TokenId::new(id.to_string()).unwrap(), id);
        for bad in [
            "",
            "t-",
            "0a1b2c3d",
            "t-0A1B2C3D",
            "t-0a1b2c3",
            "t-0a1b2c3de",
            "t-0a1b2c3g",
            "x-0a1b2c3d",
            "t-0a1b2c3d\n",
        ] {
            assert!(TokenId::new(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_token_secret_never_prints() {
        let secret = TokenSecret::new("dara_super-secret-value");
        assert_eq!(format!("{secret:?}"), "<redacted>");
        let issued = IssuedToken {
            record: TokenRecord {
                id: TokenId::new("t-00000001").unwrap(),
                label: AgentLabel::new("openclaw").unwrap(),
                created: WallTime(5),
                revoked: None,
            },
            secret,
        };
        let printed = format!("{issued:?} {:#?}", issued.clone());
        assert!(!printed.contains("super-secret"), "{printed}");
        assert_eq!(issued.secret.expose(), "dara_super-secret-value");
    }

    #[test]
    fn a_revoked_record_is_not_active() {
        let mut record = TokenRecord {
            id: TokenId::new("t-00000001").unwrap(),
            label: AgentLabel::new("openclaw").unwrap(),
            created: WallTime(5),
            revoked: None,
        };
        assert!(record.is_active());
        record.revoked = Some(WallTime(9));
        assert!(!record.is_active());
    }
}
