//! Agent tokens and the Operator's token, in files.
//!
//! `credentials/operator.token` holds the Operator's token, created once when the
//! server first starts. `credentials/agent-tokens.json` holds one record for each
//! Agent token the server has issued, with the token's SHA-256 digest and never the
//! token. Both are owner-only (0600) in a directory the server keeps at 0700, and
//! a directory or file that already exists with wider permissions is an error, not
//! a repair.
//!
//! A token is a prefix and 43 characters of base64url (32 random bytes): `dara_`
//! for an Agent, `daro_` for the Operator. The prefix lets a secret scanner find a
//! leaked one and lets this store tell the two kinds apart without hashing.
//!
//! Tokens are high-entropy random values, so a plain SHA-256 is enough to store
//! them; what matters is that a digest is compared in constant time.

use denon_avr_application::ports::BoxFuture;
use denon_avr_application::{
    AgentLabel, Credential, IssuedToken, TokenError, TokenId, TokenRecord, TokenSecret, TokenStore,
};
use denon_avr_domain::WallTime;
use denon_avr_policy::label_is_well_formed;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use subtle::ConstantTimeEq;
use tokio::sync::watch;

const AGENT_PREFIX: &str = "dara_";
const OPERATOR_PREFIX: &str = "daro_";
const SECRET_BYTES: usize = 32;
const ENCODED_LEN: usize = 43;
const OPERATOR_FILE: &str = "operator.token";
const AGENT_FILE: &str = "agent-tokens.json";
const FORMAT_VERSION: u32 = 1;

/// Why the store could not be opened. The text names a path and is for the
/// Operator's log, never for an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenStoreError(String);

impl TokenStoreError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for TokenStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "token store: {}", self.0)
    }
}

impl std::error::Error for TokenStoreError {}

/// What is written to `agent-tokens.json`.
#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    tokens: Vec<Stored>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Stored {
    id: String,
    label: String,
    created_ms: u64,
    #[serde(default)]
    revoked_ms: Option<u64>,
    /// The token's SHA-256, as 64 lowercase hexadecimal digits.
    digest: String,
}

struct Entry {
    record: TokenRecord,
    digest: [u8; 32],
}

struct Shared {
    agent_file: PathBuf,
    /// Why the agent file could not be read, when it could not. The store then
    /// holds no Agent tokens and refuses to issue or revoke, and does not replace
    /// the file.
    fault: Option<String>,
    operator_digest: [u8; 32],
    entries: Mutex<Vec<Entry>>,
    /// One issue or revocation at a time, from the change to the file.
    writing: tokio::sync::Mutex<()>,
    revocations: watch::Sender<u64>,
}

/// The token store in a credentials directory.
#[derive(Clone)]
pub struct FileTokenStore {
    shared: Arc<Shared>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FileTokenStore {
    /// Open the store in `credentials`, creating the directory (0700) and the
    /// Operator's token on a first start.
    pub fn open(credentials: &Path) -> Result<Self, TokenStoreError> {
        ensure_directory(credentials)?;
        let operator_digest = operator_digest(&credentials.join(OPERATOR_FILE))?;
        let agent_file = credentials.join(AGENT_FILE);
        // The Operator's token is needed to serve anyone, so a file it cannot read
        // is an error. A damaged agent file is a fault the Operator can fix: the
        // store runs without Agent tokens and says why.
        let (entries, fault) = match read_entries(&agent_file) {
            Ok(entries) => (entries, None),
            Err(error) => (Vec::new(), Some(error.to_string())),
        };
        Ok(Self {
            shared: Arc::new(Shared {
                agent_file,
                fault,
                operator_digest,
                entries: Mutex::new(entries),
                writing: tokio::sync::Mutex::new(()),
                revocations: watch::channel(0).0,
            }),
        })
    }
}

impl TokenStore for FileTokenStore {
    fn issue(
        &self,
        label: AgentLabel,
        now: WallTime,
    ) -> BoxFuture<'_, Result<IssuedToken, TokenError>> {
        Box::pin(async move {
            if !label_is_well_formed(label.as_str()) {
                return Err(TokenError::InvalidLabel);
            }
            if let Some(fault) = &self.shared.fault {
                return Err(TokenError::Storage(fault.clone()));
            }
            let _one_at_a_time = self.shared.writing.lock().await;
            let mut next: Vec<Stored> = {
                let entries = locked(&self.shared.entries);
                if entries
                    .iter()
                    .any(|entry| entry.record.label == label && entry.record.is_active())
                {
                    return Err(TokenError::LabelInUse);
                }
                entries.iter().map(Entry::stored).collect()
            };
            let secret = generate_secret(AGENT_PREFIX)?;
            let digest = digest_of(&secret);
            let id = fresh_id(&next)?;
            let record = TokenRecord {
                id: TokenId::new(id.clone()).map_err(|_| storage("an id could not be formed"))?,
                label: label.clone(),
                created: now,
                revoked: None,
            };
            next.push(Stored {
                id,
                label: label.as_str().to_owned(),
                created_ms: now.as_millis(),
                revoked_ms: None,
                digest: to_hex(&digest),
            });
            self.write(next).await?;
            locked(&self.shared.entries).push(Entry {
                record: record.clone(),
                digest,
            });
            Ok(IssuedToken {
                record,
                secret: TokenSecret::new(secret),
            })
        })
    }

    fn list(&self) -> Vec<TokenRecord> {
        locked(&self.shared.entries)
            .iter()
            .map(|entry| entry.record.clone())
            .collect()
    }

    fn revoke<'a>(
        &'a self,
        id: &'a TokenId,
        now: WallTime,
    ) -> BoxFuture<'a, Result<TokenRecord, TokenError>> {
        Box::pin(async move {
            if let Some(fault) = &self.shared.fault {
                return Err(TokenError::Storage(fault.clone()));
            }
            let _one_at_a_time = self.shared.writing.lock().await;
            let mut next: Vec<Stored> = {
                let entries = locked(&self.shared.entries);
                let entry = entries
                    .iter()
                    .find(|entry| &entry.record.id == id)
                    .ok_or(TokenError::NotFound)?;
                if !entry.record.is_active() {
                    return Ok(entry.record.clone());
                }
                entries.iter().map(Entry::stored).collect()
            };
            for stored in &mut next {
                if stored.id == id.as_str() {
                    stored.revoked_ms = Some(now.as_millis());
                }
            }
            self.write(next).await?;
            let record = {
                let mut entries = locked(&self.shared.entries);
                let entry = entries
                    .iter_mut()
                    .find(|entry| &entry.record.id == id)
                    .ok_or(TokenError::NotFound)?;
                entry.record.revoked = Some(now);
                entry.record.clone()
            };
            self.shared.revocations.send_modify(|count| *count += 1);
            Ok(record)
        })
    }

    fn authenticate(&self, presented: &str) -> Credential {
        // A string with neither prefix is not a token, and is not hashed.
        let agent = presented.starts_with(AGENT_PREFIX);
        let operator = presented.starts_with(OPERATOR_PREFIX);
        if !agent && !operator {
            return Credential::Unknown;
        }
        let digest = digest_of(presented);
        if operator {
            return if bool::from(digest.ct_eq(&self.shared.operator_digest)) {
                Credential::OperatorToken
            } else {
                Credential::Unknown
            };
        }
        // An Agent token is compared with every active token, none skipped on a
        // match, and a revoked one is not compared at all.
        let entries = locked(&self.shared.entries);
        let mut found = None;
        for entry in entries.iter().filter(|entry| entry.record.is_active()) {
            if bool::from(digest.ct_eq(&entry.digest)) {
                found = Some((entry.record.label.clone(), entry.record.id.clone()));
            }
        }
        match found {
            Some((label, id)) => Credential::Agent(label, id),
            None => Credential::Unknown,
        }
    }

    fn is_active(&self, id: &TokenId) -> bool {
        locked(&self.shared.entries)
            .iter()
            .any(|entry| &entry.record.id == id && entry.record.is_active())
    }

    fn changes(&self) -> watch::Receiver<u64> {
        self.shared.revocations.subscribe()
    }

    fn fault(&self) -> Option<String> {
        self.shared.fault.clone()
    }
}

impl FileTokenStore {
    /// Write the whole file, then rename it into place, on a thread that may
    /// block.
    async fn write(&self, tokens: Vec<Stored>) -> Result<(), TokenError> {
        let path = self.shared.agent_file.clone();
        tokio::task::spawn_blocking(move || write_atomically(&path, &tokens))
            .await
            .map_err(|_| storage("the writer stopped"))?
            .map_err(|why| TokenError::Storage(why.to_string()))
    }
}

impl Entry {
    fn stored(&self) -> Stored {
        Stored {
            id: self.record.id.as_str().to_owned(),
            label: self.record.label.as_str().to_owned(),
            created_ms: self.record.created.as_millis(),
            revoked_ms: self.record.revoked.map(WallTime::as_millis),
            digest: to_hex(&self.digest),
        }
    }
}

fn storage(why: &str) -> TokenError {
    TokenError::Storage(why.to_owned())
}

// ---- Files ----

fn ensure_directory(path: &Path) -> Result<(), TokenStoreError> {
    crate::data_directory::ensure_private_directory(path).map_err(TokenStoreError::new)
}

#[cfg(unix)]
fn check_file_permissions(
    path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<(), TokenStoreError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = metadata.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(TokenStoreError::new(format!(
            "{} has permissions {mode:o}, wider than owner-only; fix them first",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

/// The digest of the Operator's token, creating the token on a first start.
fn operator_digest(path: &Path) -> Result<[u8; 32], TokenStoreError> {
    match std::fs::metadata(path) {
        Ok(metadata) => {
            check_file_permissions(path, &metadata)?;
            let text = std::fs::read_to_string(path).map_err(|error| {
                TokenStoreError::new(format!("cannot read {}: {error}", path.display()))
            })?;
            let secret = text.trim();
            if !well_formed(secret, OPERATOR_PREFIX) {
                return Err(TokenStoreError::new(format!(
                    "{} does not hold an Operator token; delete it to have a new one made",
                    path.display()
                )));
            }
            Ok(digest_of(secret))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let secret = generate_secret(OPERATOR_PREFIX).map_err(|why| {
                TokenStoreError::new(format!("cannot make an Operator token: {why}"))
            })?;
            let mut file = create_private(path).map_err(|error| {
                TokenStoreError::new(format!("cannot create {}: {error}", path.display()))
            })?;
            file.write_all(format!("{secret}\n").as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|error| {
                    TokenStoreError::new(format!("cannot write {}: {error}", path.display()))
                })?;
            Ok(digest_of(&secret))
        }
        Err(error) => Err(TokenStoreError::new(format!(
            "cannot read {}: {error}",
            path.display()
        ))),
    }
}

fn read_entries(path: &Path) -> Result<Vec<Entry>, TokenStoreError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(TokenStoreError::new(format!(
                "cannot read {}: {error}",
                path.display()
            )))
        }
    };
    check_file_permissions(path, &metadata)?;
    let malformed = |why: String| {
        TokenStoreError::new(format!(
            "{} cannot be used ({why}); it was not replaced",
            path.display()
        ))
    };
    let text = std::fs::read_to_string(path).map_err(|error| {
        TokenStoreError::new(format!("cannot read {}: {error}", path.display()))
    })?;
    let file: File = serde_json::from_str(&text).map_err(|error| malformed(error.to_string()))?;
    if file.version != FORMAT_VERSION {
        return Err(malformed(format!("version {} is not known", file.version)));
    }
    let mut entries: Vec<Entry> = Vec::with_capacity(file.tokens.len());
    for stored in file.tokens {
        let label = AgentLabel::new(stored.label.clone())
            .ok()
            .filter(|label| label_is_well_formed(label.as_str()))
            .ok_or_else(|| malformed("a token has a label outside the label rule".into()))?;
        let record = TokenRecord {
            id: TokenId::new(stored.id).map_err(|why| malformed(why.to_owned()))?,
            label,
            created: WallTime(stored.created_ms),
            revoked: stored.revoked_ms.map(WallTime),
        };
        let digest = from_hex(&stored.digest)
            .ok_or_else(|| malformed("a digest is not 64 hexadecimal digits".into()))?;
        if entries.iter().any(|entry| entry.record.id == record.id) {
            return Err(malformed("two tokens have one id".into()));
        }
        if record.is_active()
            && entries
                .iter()
                .any(|entry| entry.record.is_active() && entry.record.label == record.label)
        {
            return Err(malformed("two active tokens have one label".into()));
        }
        entries.push(Entry { record, digest });
    }
    Ok(entries)
}

/// Write `tokens` to a temporary file beside `path`, sync it, and rename it over
/// `path`, so a crash leaves the old file or the new one and never half of either.
fn write_atomically(path: &Path, tokens: &[Stored]) -> std::io::Result<()> {
    let temporary = path.with_extension("json.tmp");
    // A temporary file left by a crash is of no use.
    let _ = std::fs::remove_file(&temporary);
    let json = serde_json::to_string_pretty(&File {
        version: FORMAT_VERSION,
        tokens: tokens.to_vec(),
    })
    .map_err(std::io::Error::other)?;
    {
        let mut file = create_private(&temporary)?;
        file.write_all(json.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    std::fs::rename(&temporary, path)?;
    if let Some(directory) = path.parent() {
        // Make the rename durable. A directory that cannot be synced is not an
        // error: the file is in place.
        if let Ok(directory) = std::fs::File::open(directory) {
            let _ = directory.sync_all();
        }
    }
    Ok(())
}

// ---- Tokens ----

fn generate_secret(prefix: &str) -> Result<String, TokenError> {
    let mut bytes = [0u8; SECRET_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| TokenError::Storage(error.to_string()))?;
    Ok(format!("{prefix}{}", base64url(&bytes)))
}

fn fresh_id(existing: &[Stored]) -> Result<String, TokenError> {
    for _ in 0..16 {
        let mut bytes = [0u8; 4];
        getrandom::fill(&mut bytes).map_err(|error| TokenError::Storage(error.to_string()))?;
        let id = format!("t-{}", to_hex(&bytes));
        if existing.iter().all(|stored| stored.id != id) {
            return Ok(id);
        }
    }
    Err(storage("no unused token id was found"))
}

fn well_formed(secret: &str, prefix: &str) -> bool {
    secret
        .strip_prefix(prefix)
        .is_some_and(|rest| rest.len() == ENCODED_LEN && rest.bytes().all(is_base64url))
}

fn is_base64url(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

fn digest_of(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.as_bytes()).into()
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn from_hex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    let mut out = [0u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// Base64url without padding (RFC 4648 section 5).
fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(char::from(ALPHABET[(n >> 18) as usize & 63]));
        out.push(char::from(ALPHABET[(n >> 12) as usize & 63]));
        if chunk.len() > 1 {
            out.push(char::from(ALPHABET[(n >> 6) as usize & 63]));
        }
        if chunk.len() > 2 {
            out.push(char::from(ALPHABET[n as usize & 63]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_matches_the_rfc_vectors_without_padding() {
        for (input, expected) in [
            (&b""[..], ""),
            (b"f", "Zg"),
            (b"fo", "Zm8"),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg"),
            (b"fooba", "Zm9vYmE"),
            (b"foobar", "Zm9vYmFy"),
            // The two characters that differ from base64.
            (&[0xfb, 0xff, 0xfe], "-__-"),
        ] {
            assert_eq!(base64url(input), expected);
        }
        assert_eq!(base64url(&[0u8; SECRET_BYTES]).len(), ENCODED_LEN);
    }

    #[test]
    fn hex_reads_back_what_it_writes_and_refuses_the_rest() {
        let digest = digest_of("dara_x");
        assert_eq!(from_hex(&to_hex(&digest)), Some(digest));
        assert_eq!(from_hex(&"A".repeat(64)), None);
        assert_eq!(from_hex(&"a".repeat(63)), None);
        assert_eq!(from_hex(&"g".repeat(64)), None);
    }

    #[test]
    fn a_thousand_secrets_and_ids_are_distinct() {
        let mut secrets = std::collections::BTreeSet::new();
        let mut ids = std::collections::BTreeSet::new();
        for _ in 0..1_000 {
            assert!(secrets.insert(generate_secret(AGENT_PREFIX).unwrap()));
            assert!(ids.insert(fresh_id(&[]).unwrap()));
        }
        // An id already in use is not chosen again.
        let taken: Vec<Stored> = ids
            .iter()
            .map(|id| Stored {
                id: id.clone(),
                label: "a".into(),
                created_ms: 0,
                revoked_ms: None,
                digest: "0".repeat(64),
            })
            .collect();
        assert!(!ids.contains(&fresh_id(&taken).unwrap()));
    }

    #[test]
    fn a_secret_has_the_documented_shape() {
        for prefix in [AGENT_PREFIX, OPERATOR_PREFIX] {
            let secret = generate_secret(prefix).unwrap();
            assert!(well_formed(&secret, prefix), "{secret}");
            assert!(!well_formed(&secret, "dxxx_"));
        }
        assert!(!well_formed("dara_short", AGENT_PREFIX));
    }
}
