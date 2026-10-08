//! The JSON Lines audit adapter: what it writes, how it rotates, what it reads
//! back, and what it refuses.

use denon_avr_application::audit::{bounded, intent_text, AUDIT_SCHEMA, MAX_INTENT_TEXT};
use denon_avr_application::{
    AgentLabel, AuditDecision, AuditEvent, AuditLog, AuditQuery, AuditRecord, Durability,
    PolicyDigest, Principal,
};
use denon_avr_domain::{
    CoreField, DispatchCertainty, OperationId, ReceiverId, ReceiverIntent, SoundModeIntent,
    WallTime,
};
use denon_avr_infrastructure::{AuditLimits, JsonlAuditLog};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A directory under the system's temporary one, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "denon-audit-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn audit_dir(&self) -> PathBuf {
        self.0.join("audit")
    }

    fn log(&self) -> JsonlAuditLog {
        JsonlAuditLog::new(self.audit_dir())
    }

    fn small_log(&self, max_file_bytes: u64, max_files: usize) -> JsonlAuditLog {
        JsonlAuditLog::new(self.audit_dir()).with_limits(AuditLimits {
            max_file_bytes,
            max_files,
        })
    }

    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.audit_dir())
            .map(|entries| {
                entries
                    .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn agent() -> Principal {
    Principal::Agent(AgentLabel::new("openclaw").unwrap())
}

fn room() -> ReceiverId {
    ReceiverId::new("living-room").unwrap()
}

fn digest(byte: u8) -> PolicyDigest {
    PolicyDigest::from_bytes([byte; 32])
}

fn record(n: u64, event: AuditEvent) -> AuditRecord {
    AuditRecord {
        schema: AUDIT_SCHEMA,
        run: WallTime(1_000),
        at: WallTime(2_000 + n),
        operation: Some(OperationId(n)),
        principal: agent(),
        receiver: Some(room()),
        event,
    }
}

fn decided(n: u64) -> AuditRecord {
    record(
        n,
        AuditEvent::Decided {
            intent: "volume -35.0 dB".into(),
            decision: AuditDecision::RequireApproval,
            reasons: vec![
                "the volume target -35.0 dB is above the limit -40.0 dB".into(),
                "a second reason, long enough to give the line some weight".into(),
            ],
            rules: vec!["volume-ceiling".into(), "volume-step".into()],
            baseline: vec![CoreField::Volume],
            policy: Some(digest(7)),
        },
    )
}

/// One record of every kind, in the order a write would produce them, with the
/// shapes that differ: no operation, no receiver, an Operator, and every
/// dispatch certainty.
fn every_kind() -> Vec<AuditRecord> {
    let mut records = vec![
        AuditRecord {
            operation: None,
            principal: Principal::Operator,
            receiver: None,
            ..record(
                1,
                AuditEvent::PolicyLoaded {
                    digest: digest(0xab),
                },
            )
        },
        AuditRecord {
            operation: None,
            principal: Principal::Operator,
            receiver: None,
            ..record(
                2,
                AuditEvent::PolicyLoadFailed {
                    error: "line 4: unknown key \"time_of_day\"".into(),
                },
            )
        },
        decided(3),
        record(
            3,
            AuditEvent::Dispatching {
                intent: "volume -35.0 dB".into(),
                before: Some(-72),
                target: Some(-70),
                precondition_fields: vec![CoreField::Volume, CoreField::Mute],
            },
        ),
        record(
            4,
            AuditEvent::Dispatching {
                intent: "mute off".into(),
                before: None,
                target: None,
                precondition_fields: vec![],
            },
        ),
        AuditRecord {
            receiver: Some(ReceiverId::ad_hoc("192.0.2.50").unwrap()),
            principal: Principal::Operator,
            ..record(
                5,
                AuditEvent::Decided {
                    intent: "mute on".into(),
                    decision: AuditDecision::Allow,
                    reasons: vec![],
                    rules: vec![],
                    baseline: vec![],
                    policy: None,
                },
            )
        },
        record(
            6,
            AuditEvent::Decided {
                intent: "volume -10.0 dB".into(),
                decision: AuditDecision::Deny,
                reasons: vec!["too loud".into()],
                rules: vec!["volume-hard-limit".into()],
                baseline: vec![],
                policy: Some(digest(7)),
            },
        ),
    ];
    for (offset, dispatch) in [
        DispatchCertainty::NotDispatched,
        DispatchCertainty::PossiblyDispatched,
        DispatchCertainty::CompleteWrite,
        DispatchCertainty::Unknown,
    ]
    .into_iter()
    .enumerate()
    {
        records.push(record(
            7 + offset as u64,
            AuditEvent::Finished {
                status: if offset == 0 { "rejected" } else { "completed" }.into(),
                dispatch,
                confirmed: offset == 2,
                reason: (offset == 0).then(|| "audit log unavailable".to_string()),
            },
        ));
    }
    records
}

async fn newest_first(log: &JsonlAuditLog) -> Vec<(u64, AuditRecord)> {
    log.query(AuditQuery::new(500))
        .await
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| (entry.seq, entry.record))
        .collect()
}

#[tokio::test]
async fn records_round_trip_in_order() {
    let scratch = Scratch::new("round-trip");
    let log = scratch.log();
    let written = every_kind();
    for (index, record) in written.iter().enumerate() {
        let durability = if index % 2 == 0 {
            Durability::Flushed
        } else {
            Durability::Synced
        };
        log.append(record.clone(), durability).await.unwrap();
    }

    let page = log.query(AuditQuery::new(500)).await.unwrap();
    assert_eq!(page.next, None);
    let seqs: Vec<u64> = page.entries.iter().map(|entry| entry.seq).collect();
    let expected: Vec<u64> = (1..=written.len() as u64).rev().collect();
    assert_eq!(seqs, expected, "newest first, from 1");
    let read: Vec<AuditRecord> = page
        .entries
        .into_iter()
        .rev()
        .map(|entry| entry.record)
        .collect();
    assert_eq!(read, written);

    let since = log.since(WallTime(0)).await.unwrap();
    assert_eq!(since, written, "oldest first");
}

#[tokio::test]
async fn a_log_that_does_not_exist_yet_reads_as_empty_and_is_not_created() {
    let scratch = Scratch::new("absent");
    let log = scratch.log();
    assert_eq!(log.since(WallTime(0)).await.unwrap(), vec![]);
    let page = log.query(AuditQuery::new(10)).await.unwrap();
    assert!(page.entries.is_empty() && page.next.is_none());
    assert!(!scratch.audit_dir().exists());
}

#[tokio::test]
async fn rotation_keeps_the_newest_files_within_the_size_limit() {
    let scratch = Scratch::new("rotation");
    let log = scratch.small_log(1_000, 3);
    for n in 1..=30 {
        log.append(decided(n), Durability::Flushed).await.unwrap();
    }

    assert_eq!(
        scratch.files(),
        ["audit.1.jsonl", "audit.2.jsonl", "audit.jsonl"]
    );
    for name in scratch.files() {
        let size = std::fs::metadata(scratch.audit_dir().join(&name))
            .unwrap()
            .len();
        assert!(size <= 1_000, "{name} is {size} bytes");
    }

    let entries = newest_first(&log).await;
    let seqs: Vec<u64> = entries.iter().map(|(seq, _)| *seq).collect();
    assert_eq!(seqs[0], 30, "the newest record is kept");
    assert!(seqs.len() >= 3 && seqs.len() < 30, "the oldest are dropped");
    assert!(
        seqs.windows(2).all(|pair| pair[0] == pair[1] + 1),
        "what is kept is one run of the newest records: {seqs:?}"
    );
}

#[tokio::test]
async fn since_and_query_read_across_the_retained_files() {
    let scratch = Scratch::new("across");
    let log = scratch.small_log(1_000, 4);
    for n in 1..=30 {
        log.append(decided(n), Durability::Flushed).await.unwrap();
    }
    assert!(scratch.files().len() > 1, "the log rotated");

    let retained: Vec<(u64, AuditRecord)> = newest_first(&log).await.into_iter().rev().collect();
    assert!(retained.len() > 3);

    // `since` is every retained record at or after the time, oldest first.
    let from = retained[retained.len() / 2].1.at;
    let expected: Vec<AuditRecord> = retained
        .iter()
        .filter(|(_, record)| record.at >= from)
        .map(|(_, record)| record.clone())
        .collect();
    assert_eq!(log.since(from).await.unwrap(), expected);

    // Pages of four, followed by their cursors, are the same listing.
    let mut paged = Vec::new();
    let mut sizes = Vec::new();
    let mut query = AuditQuery::new(4);
    loop {
        let page = log.query(query).await.unwrap();
        sizes.push(page.entries.len());
        paged.extend(page.entries.iter().map(|e| e.seq));
        match page.next {
            Some(cursor) => query = AuditQuery::new(4).after(cursor),
            None => break,
        }
    }
    let all: Vec<u64> = retained.iter().rev().map(|(seq, _)| *seq).collect();
    assert_eq!(paged, all);
    let (last, full) = sizes.split_last().unwrap();
    assert!(
        full.iter().all(|size| *size == 4),
        "pages fill up: {sizes:?}"
    );
    assert!((1..=4).contains(last), "{sizes:?}");

    // A page that holds everything left has no next; one short of it has.
    let exact = log.query(AuditQuery::new(retained.len())).await.unwrap();
    assert_eq!(exact.entries.len(), retained.len());
    assert_eq!(exact.next, None);
    let short = log
        .query(AuditQuery::new(retained.len() - 1))
        .await
        .unwrap();
    assert_eq!(short.entries.len(), retained.len() - 1);
    assert!(short.next.is_some());
}

#[tokio::test]
async fn a_truncated_last_line_and_unknown_kinds_are_skipped() {
    let scratch = Scratch::new("damaged");
    let first = scratch.log();
    first.append(decided(1), Durability::Flushed).await.unwrap();
    drop(first);

    // A record of a kind from a newer version, one in a newer schema, a line
    // that is not JSON, and a write cut short.
    let path = scratch.audit_dir().join("audit.jsonl");
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str(
        "{\"seq\":2,\"schema\":1,\"run\":1,\"at\":9,\"operation\":null,\"principal\":\"operator\",\
         \"receiver\":null,\"event\":{\"kind\":\"from_the_future\",\"x\":1}}\n",
    );
    text.push_str(
        "{\"seq\":3,\"schema\":2,\"run\":1,\"at\":9,\"operation\":null,\"principal\":\"operator\",\
         \"receiver\":null,\"event\":{\"kind\":\"policy_load_failed\",\"error\":\"x\"}}\n",
    );
    text.push_str("this is not json\n");
    text.push_str(
        "{\"seq\":5,\"schema\":1,\"run\":1,\"at\":9,\"operation\":null,\"principal\":\"operator\",\
         \"receiver\":null,\"event\":{\"kind\":\"policy_loaded\",\"digest\":\"",
    );
    std::fs::write(&path, text).unwrap();

    let log = scratch.log();
    let entries = newest_first(&log).await;
    assert_eq!(entries.len(), 1, "only the readable record: {entries:?}");
    assert_eq!(entries[0], (1, decided(1)));
    assert_eq!(log.since(WallTime(0)).await.unwrap(), vec![decided(1)]);

    // The next record starts on its own line and numbers after the highest
    // sequence number that could be read, the records it skipped among them.
    log.append(decided(5), Durability::Synced).await.unwrap();
    let entries = newest_first(&log).await;
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0], (4, decided(5)));
    assert_eq!(entries[1], (1, decided(1)));
}

#[tokio::test]
async fn agent_text_is_bounded_and_escaped_in_the_line() {
    let scratch = Scratch::new("hostile");
    let log = scratch.log();
    let hostile = format!("a\nb\"c\\d\u{7}{}", "x".repeat(10_000));

    // Text the gate builds from an intent is already short.
    let from_intent = intent_text(&ReceiverIntent::SoundMode(SoundModeIntent::Select(
        hostile.clone(),
    )));
    assert!(from_intent.chars().count() <= MAX_INTENT_TEXT);

    // And the adapter bounds whatever it is handed, so a mistake elsewhere
    // cannot make a line long or split it.
    let record = AuditRecord {
        principal: Principal::Agent(AgentLabel::new("p".repeat(64)).unwrap()),
        ..record(
            1,
            AuditEvent::Decided {
                intent: hostile.clone(),
                decision: AuditDecision::Deny,
                reasons: vec![hostile.clone()],
                rules: vec![hostile.clone()],
                baseline: vec![],
                policy: None,
            },
        )
    };
    log.append(record, Durability::Flushed).await.unwrap();

    let text = std::fs::read_to_string(scratch.audit_dir().join("audit.jsonl")).unwrap();
    assert_eq!(text.matches('\n').count(), 1, "one line");
    assert!(text.ends_with('\n'));
    assert!(
        text.len() < 4_096,
        "the line is {} bytes for 30,000 characters of agent text",
        text.len()
    );
    let parsed: serde_json::Value = serde_json::from_str(text.trim_end()).unwrap();
    let event = &parsed["event"];
    let intent = event["intent"].as_str().unwrap();
    assert_eq!(intent.chars().count(), MAX_INTENT_TEXT);
    assert_eq!(intent, bounded(&hostile, MAX_INTENT_TEXT));
    assert!(event["reasons"][0].as_str().unwrap().chars().count() <= 512);

    // What comes back is the bounded text.
    let entries = newest_first(&log).await;
    let AuditEvent::Decided { intent: read, .. } = &entries[0].1.event else {
        panic!("{entries:?}");
    };
    assert_eq!(read.chars().count(), MAX_INTENT_TEXT);
}

#[cfg(unix)]
mod permissions {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[tokio::test]
    async fn files_are_0600_in_a_0700_directory() {
        let scratch = Scratch::new("modes");
        let log = scratch.small_log(1_000, 3);
        for n in 1..=12 {
            log.append(decided(n), Durability::Flushed).await.unwrap();
        }
        assert_eq!(mode(&scratch.audit_dir()), 0o700);
        assert!(scratch.files().len() > 1, "rotated files are checked too");
        for name in scratch.files() {
            assert_eq!(mode(&scratch.audit_dir().join(&name)), 0o600, "{name}");
        }
    }

    #[tokio::test]
    async fn wider_existing_permissions_are_an_error() {
        let scratch = Scratch::new("wide");
        let directory = scratch.audit_dir();
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();

        let error = scratch
            .log()
            .append(decided(1), Durability::Synced)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("permissions"), "{error}");
        assert_eq!(mode(&directory), 0o755, "not repaired");
        assert!(scratch.log().query(AuditQuery::new(1)).await.is_err());
        assert!(scratch.files().is_empty(), "nothing was written");

        // The same for a file inside a directory that is fine.
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let file = directory.join("audit.jsonl");
        std::fs::write(&file, "").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        let error = scratch
            .log()
            .append(decided(1), Durability::Flushed)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("permissions"), "{error}");
        assert_eq!(mode(&file), 0o644, "not repaired");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
    }
}

#[tokio::test]
async fn seq_continues_after_reopening() {
    let scratch = Scratch::new("reopen");
    let first = scratch.small_log(1_000, 3);
    for n in 1..=20 {
        first.append(decided(n), Durability::Flushed).await.unwrap();
    }
    drop(first);

    let second = scratch.small_log(1_000, 3);
    second
        .append(decided(21), Durability::Synced)
        .await
        .unwrap();
    let entries = newest_first(&second).await;
    assert_eq!(entries[0].0, 21, "numbered after the 20 already written");
    assert_eq!(entries[1].0, 20);
}

#[tokio::test]
async fn a_synced_append_returns_after_writing() {
    let scratch = Scratch::new("synced");
    let log = scratch.log();
    log.append(decided(1), Durability::Synced).await.unwrap();

    // Nothing else has run: the line is already in the file.
    let text = std::fs::read_to_string(scratch.audit_dir().join("audit.jsonl")).unwrap();
    assert_eq!(text.lines().count(), 1);
    assert!(text.contains("\"kind\":\"decided\""), "{text}");
}

#[tokio::test]
async fn concurrent_appends_never_interleave_lines() {
    let scratch = Scratch::new("concurrent");
    let log = Arc::new(scratch.log());
    let tasks: Vec<_> = (1..=50)
        .map(|n| {
            let log = Arc::clone(&log);
            tokio::spawn(async move { log.append(decided(n), Durability::Flushed).await })
        })
        .collect();
    for task in tasks {
        task.await.unwrap().unwrap();
    }

    let text = std::fs::read_to_string(scratch.audit_dir().join("audit.jsonl")).unwrap();
    assert_eq!(text.lines().count(), 50);
    for line in text.lines() {
        serde_json::from_str::<serde_json::Value>(line).unwrap();
    }
    let mut seqs: Vec<u64> = newest_first(&log).await.iter().map(|(s, _)| *s).collect();
    seqs.sort_unstable();
    assert_eq!(seqs, (1..=50).collect::<Vec<u64>>());
}
