//! The file-backed token store: what it issues, what it writes, what it refuses,
//! and how it survives a crash.

#![cfg(unix)]

use denon_avr_application::{AgentLabel, Credential, TokenError, TokenId, TokenStore};
use denon_avr_domain::WallTime;
use denon_avr_infrastructure::FileTokenStore;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A directory under the system's temporary one, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "denon-tokens-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn credentials(&self) -> PathBuf {
        self.0.join("credentials")
    }

    fn open(&self) -> FileTokenStore {
        FileTokenStore::open(&self.credentials()).unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn label(name: &str) -> AgentLabel {
    AgentLabel::new(name).unwrap()
}

fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn operator_token(scratch: &Scratch) -> String {
    std::fs::read_to_string(scratch.credentials().join("operator.token"))
        .unwrap()
        .trim()
        .to_owned()
}

#[tokio::test]
async fn an_issued_token_authenticates_as_its_label_and_a_revoked_one_never_does() {
    let scratch = Scratch::new("authenticate");
    let store = scratch.open();
    let issued = store
        .issue(label("claude-code"), WallTime(10))
        .await
        .unwrap();
    let secret = issued.secret.expose();

    assert_eq!(
        store.authenticate(secret),
        Credential::Agent(label("claude-code"), issued.record.id.clone())
    );
    assert!(store.is_active(&issued.record.id));

    // One character different is another credential.
    let mut wrong = secret.to_owned();
    wrong.pop();
    wrong.push(if secret.ends_with('A') { 'B' } else { 'A' });
    assert_eq!(store.authenticate(&wrong), Credential::Unknown);

    let revoked = store.revoke(&issued.record.id, WallTime(99)).await.unwrap();
    assert_eq!(revoked.revoked, Some(WallTime(99)));
    assert_eq!(store.authenticate(secret), Credential::Unknown);
    assert!(!store.is_active(&issued.record.id));
}

#[tokio::test]
async fn a_revoked_record_stays_in_the_listing() {
    let scratch = Scratch::new("listing");
    let store = scratch.open();
    let first = store.issue(label("a"), WallTime(1)).await.unwrap();
    let second = store.issue(label("b"), WallTime(2)).await.unwrap();
    store.revoke(&first.record.id, WallTime(3)).await.unwrap();
    let listed = store.list();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, first.record.id);
    assert_eq!(listed[0].revoked, Some(WallTime(3)));
    assert_eq!(listed[1].id, second.record.id);
    assert_eq!(listed[1].revoked, None);
    // Revoking again returns it as it is.
    let again = store.revoke(&first.record.id, WallTime(50)).await.unwrap();
    assert_eq!(again.revoked, Some(WallTime(3)));
    assert_eq!(
        store
            .revoke(&TokenId::new("t-0000ffff").unwrap(), WallTime(1))
            .await,
        Err(TokenError::NotFound)
    );
}

#[tokio::test]
async fn the_file_holds_a_digest_and_never_the_token() {
    let scratch = Scratch::new("digest");
    let store = scratch.open();
    let issued = store.issue(label("openclaw"), WallTime(1)).await.unwrap();
    let text = std::fs::read_to_string(scratch.credentials().join("agent-tokens.json")).unwrap();
    assert!(!text.contains(issued.secret.expose()), "{text}");
    let body = issued.secret.expose().trim_start_matches("dara_");
    assert!(!text.contains(body));
    // The digest is the SHA-256 of the token, in hex.
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    let digest = json["tokens"][0]["digest"].as_str().unwrap();
    assert_eq!(digest.len(), 64);
    assert!(digest
        .bytes()
        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    // And it is read back after a restart.
    let reopened = scratch.open();
    assert_eq!(
        reopened.authenticate(issued.secret.expose()),
        Credential::Agent(label("openclaw"), issued.record.id)
    );
}

#[tokio::test]
async fn the_operator_token_is_created_once_and_read_back() {
    let scratch = Scratch::new("operator");
    let first = scratch.open();
    let token = operator_token(&scratch);
    assert!(
        token.starts_with("daro_") && token.len() == 5 + 43,
        "{token}"
    );
    assert_eq!(first.authenticate(&token), Credential::OperatorToken);
    assert_eq!(
        std::fs::read_to_string(scratch.credentials().join("operator.token"))
            .unwrap()
            .matches('\n')
            .count(),
        1
    );

    // A second open reads the same token and does not make another.
    let second = scratch.open();
    assert_eq!(operator_token(&scratch), token);
    assert_eq!(second.authenticate(&token), Credential::OperatorToken);
}

#[tokio::test]
async fn an_operator_token_is_not_an_agent_credential_and_the_reverse() {
    let scratch = Scratch::new("kinds");
    let store = scratch.open();
    let operator = operator_token(&scratch);
    let agent = store.issue(label("openclaw"), WallTime(1)).await.unwrap();
    assert_eq!(store.authenticate(&operator), Credential::OperatorToken);
    assert!(matches!(
        store.authenticate(agent.secret.expose()),
        Credential::Agent(..)
    ));
    // The body of one under the prefix of the other is neither.
    let swapped = format!("dara_{}", operator.trim_start_matches("daro_"));
    assert_eq!(store.authenticate(&swapped), Credential::Unknown);
    let swapped = format!("daro_{}", agent.secret.expose().trim_start_matches("dara_"));
    assert_eq!(store.authenticate(&swapped), Credential::Unknown);
}

#[tokio::test]
async fn an_unknown_or_unprefixed_string_is_unknown() {
    let scratch = Scratch::new("unknown");
    let store = scratch.open();
    let issued = store.issue(label("openclaw"), WallTime(1)).await.unwrap();
    let body = issued
        .secret
        .expose()
        .trim_start_matches("dara_")
        .to_owned();
    for presented in [
        "",
        " ",
        "dara_",
        "daro_",
        "dara_not-a-token",
        &body,
        &format!(" {}", issued.secret.expose()),
        &format!("{} ", issued.secret.expose()),
        &format!("Bearer {}", issued.secret.expose()),
        &issued.secret.expose().to_uppercase(),
    ] {
        assert_eq!(
            store.authenticate(presented),
            Credential::Unknown,
            "{presented:?}"
        );
    }
}

#[tokio::test]
async fn a_second_active_token_for_a_label_is_refused_and_a_revoked_one_does_not_count() {
    let scratch = Scratch::new("one-per-label");
    let store = scratch.open();
    let first = store.issue(label("openclaw"), WallTime(1)).await.unwrap();
    assert_eq!(
        store
            .issue(label("openclaw"), WallTime(2))
            .await
            .unwrap_err(),
        TokenError::LabelInUse
    );
    store.revoke(&first.record.id, WallTime(3)).await.unwrap();
    let second = store.issue(label("openclaw"), WallTime(4)).await.unwrap();
    assert_ne!(second.record.id, first.record.id);
    assert_eq!(
        store.authenticate(first.secret.expose()),
        Credential::Unknown,
        "the old token stays revoked"
    );
    // The label rule holds in the store as well.
    for bad in ["Claude-Code", "a b", "unauthenticated"] {
        assert_eq!(
            store.issue(label(bad), WallTime(5)).await.unwrap_err(),
            TokenError::InvalidLabel,
            "{bad}"
        );
    }
}

#[tokio::test]
async fn a_damaged_agent_file_degrades_the_store_and_is_not_replaced() {
    let scratch = Scratch::new("corrupt");
    drop(scratch.open());
    let operator = operator_token(&scratch);
    let path = scratch.credentials().join("agent-tokens.json");
    for junk in [
        "not json",
        "{\"version\":2,\"tokens\":[]}",
        "{\"version\":1,\"tokens\":[{\"id\":\"nope\",\"label\":\"a\",\"created_ms\":1,\"digest\":\"00\"}]}",
        // A label outside the rule.
        &format!(
            "{{\"version\":1,\"tokens\":[{{\"id\":\"t-00000001\",\"label\":\"Claude\",\"created_ms\":1,\"digest\":\"{}\"}}]}}",
            "0".repeat(64)
        ),
        // Two active tokens for one label.
        &format!(
            "{{\"version\":1,\"tokens\":[{{\"id\":\"t-00000001\",\"label\":\"a\",\"created_ms\":1,\"digest\":\"{d}\"}},{{\"id\":\"t-00000002\",\"label\":\"a\",\"created_ms\":2,\"digest\":\"{d}\"}}]}}",
            d = "0".repeat(64)
        ),
    ] {
        std::fs::write(&path, junk).unwrap();
        // The store opens, because the Operator's token does not live in that file.
        let store = FileTokenStore::open(&scratch.credentials()).expect(junk);
        let fault = store.fault().expect("a fault is reported");
        assert!(fault.contains("agent-tokens.json"), "{fault}");
        assert_eq!(store.authenticate(&operator), Credential::OperatorToken);
        assert!(store.list().is_empty());
        // It refuses to issue or revoke, and does not replace the file.
        assert!(matches!(
            store.issue(label("openclaw"), WallTime(1)).await,
            Err(TokenError::Storage(_))
        ));
        assert!(matches!(
            store
                .revoke(&TokenId::new("t-00000001").unwrap(), WallTime(1))
                .await,
            Err(TokenError::Storage(_))
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), junk, "not replaced");
    }

    // A file in the Operator's place that is not a token is an error: nobody could
    // be served without it. It is not replaced either.
    std::fs::remove_file(&path).unwrap();
    let operator_file = scratch.credentials().join("operator.token");
    std::fs::write(&operator_file, "hello\n").unwrap();
    let error = FileTokenStore::open(&scratch.credentials()).err().unwrap();
    assert!(error.to_string().contains("operator.token"), "{error}");
    assert_eq!(std::fs::read_to_string(&operator_file).unwrap(), "hello\n");
}

#[tokio::test]
async fn credentials_are_0600_in_a_0700_directory() {
    let scratch = Scratch::new("modes");
    let store = scratch.open();
    store.issue(label("openclaw"), WallTime(1)).await.unwrap();
    assert_eq!(mode(&scratch.credentials()), 0o700);
    assert_eq!(mode(&scratch.credentials().join("operator.token")), 0o600);
    assert_eq!(
        mode(&scratch.credentials().join("agent-tokens.json")),
        0o600
    );
    // Revoking rewrites the file, and it stays owner-only.
    let id = store.list()[0].id.clone();
    store.revoke(&id, WallTime(2)).await.unwrap();
    assert_eq!(
        mode(&scratch.credentials().join("agent-tokens.json")),
        0o600
    );
    let names: BTreeSet<String> = std::fs::read_dir(scratch.credentials())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(
        names,
        BTreeSet::from(["agent-tokens.json".into(), "operator.token".into()]),
        "no temporary file is left"
    );
}

#[tokio::test]
async fn wider_existing_permissions_are_an_error() {
    use std::os::unix::fs::PermissionsExt;
    let scratch = Scratch::new("wide");
    drop(scratch.open());
    let set = |path: &Path, mode: u32| {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap()
    };

    set(&scratch.credentials(), 0o755);
    let error = FileTokenStore::open(&scratch.credentials()).err().unwrap();
    assert!(error.to_string().contains("755"), "{error}");
    set(&scratch.credentials(), 0o700);

    let token = scratch.credentials().join("operator.token");
    set(&token, 0o644);
    assert!(FileTokenStore::open(&scratch.credentials()).is_err());
    set(&token, 0o600);

    let store = scratch.open();
    store.issue(label("openclaw"), WallTime(1)).await.unwrap();
    // The agent file is a fault the Operator can fix; it does not lock them out.
    let agents = scratch.credentials().join("agent-tokens.json");
    set(&agents, 0o640);
    let degraded = FileTokenStore::open(&scratch.credentials()).unwrap();
    assert!(degraded.fault().unwrap().contains("640"));
}

#[tokio::test]
async fn a_crash_between_the_temporary_file_and_the_rename_keeps_the_old_file() {
    let scratch = Scratch::new("crash");
    let store = scratch.open();
    let issued = store.issue(label("openclaw"), WallTime(1)).await.unwrap();
    // A crash after the temporary file was written and before the rename leaves
    // it behind, with whatever was in it.
    let leftover = scratch.credentials().join("agent-tokens.json.tmp");
    std::fs::write(&leftover, "{ half a file").unwrap();

    let reopened = scratch.open();
    assert_eq!(reopened.list().len(), 1, "the old file is the truth");
    assert!(matches!(
        reopened.authenticate(issued.secret.expose()),
        Credential::Agent(..)
    ));
    // The next change replaces the leftover and leaves none.
    reopened
        .issue(label("claude-code"), WallTime(2))
        .await
        .unwrap();
    assert!(!leftover.exists());
    assert_eq!(scratch.open().list().len(), 2);
}

#[tokio::test]
async fn changes_bump_on_revoke_and_on_nothing_else() {
    let scratch = Scratch::new("changes");
    let store = scratch.open();
    let changes = store.changes();
    assert_eq!(*changes.borrow(), 0);
    let issued = store.issue(label("openclaw"), WallTime(1)).await.unwrap();
    store.authenticate(issued.secret.expose());
    store.list();
    assert_eq!(
        *changes.borrow(),
        0,
        "issuing and reading are not revocations"
    );
    store.revoke(&issued.record.id, WallTime(2)).await.unwrap();
    assert_eq!(*changes.borrow(), 1);
    store.revoke(&issued.record.id, WallTime(3)).await.unwrap();
    assert_eq!(*changes.borrow(), 1, "revoking twice is one revocation");
    let _ = store
        .revoke(&TokenId::new("t-0000ffff").unwrap(), WallTime(4))
        .await;
    assert_eq!(*changes.borrow(), 1);
}

#[tokio::test]
async fn concurrent_issues_never_lose_a_record() {
    let scratch = Scratch::new("concurrent");
    let store = Arc::new(scratch.open());
    let mut tasks = Vec::new();
    for n in 0..24 {
        let store = Arc::clone(&store);
        tasks.push(tokio::spawn(async move {
            store
                .issue(label(&format!("agent-{n}")), WallTime(n))
                .await
                .unwrap()
        }));
    }
    let mut ids = BTreeSet::new();
    for task in tasks {
        let issued = task.await.unwrap();
        assert!(ids.insert(issued.record.id.clone()));
        assert!(matches!(
            store.authenticate(issued.secret.expose()),
            Credential::Agent(..)
        ));
    }
    assert_eq!(store.list().len(), 24);
    // What is on disk is what is in memory.
    let reopened = scratch.open();
    assert_eq!(reopened.list(), store.list());
}

#[tokio::test]
async fn issued_tokens_are_distinct_and_have_the_documented_shape() {
    let scratch = Scratch::new("shape");
    let store = scratch.open();
    let mut secrets = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for n in 0..40_u64 {
        // Revoke each as it is made so one label can be reused.
        let issued = store.issue(label("openclaw"), WallTime(n)).await.unwrap();
        let secret = issued.secret.expose();
        let rest = secret.strip_prefix("dara_").expect("the prefix");
        assert_eq!(rest.len(), 43, "{secret}");
        assert!(rest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        assert!(secrets.insert(secret.to_owned()), "a secret repeated");
        assert!(
            ids.insert(issued.record.id.as_str().to_owned()),
            "an id repeated"
        );
        let id = issued.record.id.as_str();
        assert!(id.starts_with("t-") && id.len() == 10);
        store.revoke(&issued.record.id, WallTime(n)).await.unwrap();
    }
    assert_eq!(store.list().len(), 40);
    let operator = operator_token(&scratch);
    assert!(!secrets.contains(&operator));
}
