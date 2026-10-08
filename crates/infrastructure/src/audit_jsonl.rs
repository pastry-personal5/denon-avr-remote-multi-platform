//! The audit log as JSON Lines files.
//!
//! One JSON object per line, appended under an async lock so lines never
//! interleave. The active file is `audit.jsonl`; when the next record would
//! take it past the size limit the set rotates to `audit.1.jsonl`,
//! `audit.2.jsonl`, and so on, and the oldest beyond the file count is deleted.
//! The directory is created 0700 and the files 0600; a directory or file that
//! already exists with wider permissions is an error on first use, never a
//! silent repair.
//!
//! Reading skips what it cannot use: a line cut short by a crash, a line that
//! is not JSON, and a record of a kind or schema this code does not know. A
//! record it cannot use still counts for the sequence number, so the numbers
//! keep rising after a restart.

use denon_avr_application::audit::{
    bounded, core_field_name, parse_core_field, AUDIT_SCHEMA, MAX_INTENT_TEXT, MAX_REASON_TEXT,
};
use denon_avr_application::ports::BoxFuture;
use denon_avr_application::{
    AgentLabel, AuditCursor, AuditDecision, AuditEntry, AuditError, AuditEvent, AuditLog,
    AuditPage, AuditQuery, AuditRecord, Durability, PolicyDigest, Principal,
};
use denon_avr_domain::{CoreField, DispatchCertainty, OperationId, ReceiverId, WallTime};
use serde_json::{json, Map, Value};
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Mutex;

const ACTIVE: &str = "audit.jsonl";
const MAX_STATUS_TEXT: usize = 64;

/// How much audit is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditLimits {
    /// A record that would take the active file past this size starts a new
    /// file. One record larger than this still goes in a file of its own.
    pub max_file_bytes: u64,
    /// Files kept, the active one included. At least one.
    pub max_files: usize,
}

impl Default for AuditLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 20 * 1024 * 1024,
            max_files: 10,
        }
    }
}

/// The audit log in a directory of JSON Lines files.
pub struct JsonlAuditLog {
    directory: PathBuf,
    limits: AuditLimits,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// Whether the directory and its files have passed the permission check.
    checked: bool,
    /// The sequence number the next record gets, once it has been worked out.
    next_seq: Option<u64>,
    writer: Option<Writer>,
}

struct Writer {
    file: File,
    len: u64,
}

impl JsonlAuditLog {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            limits: AuditLimits::default(),
            state: Mutex::new(State::default()),
        }
    }

    pub fn with_limits(mut self, limits: AuditLimits) -> Self {
        self.limits = AuditLimits {
            max_files: limits.max_files.max(1),
            ..limits
        };
        self
    }

    /// The directory the files are in.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    fn active_path(&self) -> PathBuf {
        self.directory.join(ACTIVE)
    }

    fn rotated_path(&self, index: usize) -> PathBuf {
        self.directory.join(format!("audit.{index}.jsonl"))
    }

    async fn append_locked(
        &self,
        state: &mut State,
        record: &AuditRecord,
        durability: Durability,
    ) -> Result<(), AuditError> {
        self.prepare(state, true).await?;
        let seq = match state.next_seq {
            Some(seq) => seq,
            None => {
                let seq = self.highest_seq().await? + 1;
                state.next_seq = Some(seq);
                seq
            }
        };
        let mut line = encode(seq, record);
        line.push('\n');
        let needed = line.len() as u64;

        if state.writer.is_none() {
            state.writer = Some(self.open_writer().await?);
        }
        let len = state.writer.as_ref().map_or(0, |writer| writer.len);
        if len > 0 && len + needed > self.limits.max_file_bytes {
            state.writer = None;
            self.rotate().await?;
            state.writer = Some(self.open_writer().await?);
        }

        let writer = state.writer.as_mut().expect("the writer was just opened");
        let written = async {
            writer.file.write_all(line.as_bytes()).await?;
            writer.file.flush().await?;
            if durability == Durability::Synced {
                writer.file.sync_data().await?;
            }
            Ok::<(), std::io::Error>(())
        }
        .await;
        match written {
            Ok(()) => {
                writer.len += needed;
                state.next_seq = Some(seq + 1);
                Ok(())
            }
            Err(error) => {
                // A write that failed part way leaves a line cut short. Opening
                // the file again ends it before anything else is added.
                state.writer = None;
                Err(self.error("writing", &self.active_path(), error))
            }
        }
    }

    /// Check the directory and its files, creating the directory when `create`
    /// is set. Returns whether it exists.
    async fn prepare(&self, state: &mut State, create: bool) -> Result<bool, AuditError> {
        if state.checked {
            return Ok(true);
        }
        match fs::metadata(&self.directory).await {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    return Err(AuditError::new(format!(
                        "{} is not a directory",
                        self.directory.display()
                    )));
                }
                check_permissions(&self.directory, &metadata)?;
                for (_, path) in self.list_files().await? {
                    let metadata = fs::metadata(&path)
                        .await
                        .map_err(|error| self.error("reading", &path, error))?;
                    check_permissions(&path, &metadata)?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if !create {
                    return Ok(false);
                }
                self.create_directory().await?;
            }
            Err(error) => return Err(self.error("reading", &self.directory, error)),
        }
        state.checked = true;
        Ok(true)
    }

    async fn create_directory(&self) -> Result<(), AuditError> {
        if let Some(parent) = self
            .directory
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .await
                .map_err(|error| self.error("creating", parent, error))?;
        }
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        builder.mode(0o700);
        match builder.create(&self.directory).await {
            Ok(()) => Ok(()),
            // Someone made it between the check and now. It is checked on the
            // next use rather than trusted.
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(AuditError::new(format!(
                    "{} appeared while it was being created; try again",
                    self.directory.display()
                )))
            }
            Err(error) => Err(self.error("creating", &self.directory, error)),
        }
    }

    /// Open the active file for appending, ending a line a crash cut short.
    async fn open_writer(&self) -> Result<Writer, AuditError> {
        let path = self.active_path();
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options
            .open(&path)
            .await
            .map_err(|error| self.error("opening", &path, error))?;
        let mut len = file
            .metadata()
            .await
            .map_err(|error| self.error("reading", &path, error))?
            .len();
        if len > 0
            && !ends_with_newline(&path)
                .await
                .map_err(|e| self.error("reading", &path, e))?
        {
            file.write_all(b"\n")
                .await
                .map_err(|error| self.error("writing", &path, error))?;
            file.flush()
                .await
                .map_err(|error| self.error("writing", &path, error))?;
            len += 1;
        }
        Ok(Writer { file, len })
    }

    /// Move every file one place older and drop what falls past the count.
    async fn rotate(&self) -> Result<(), AuditError> {
        let files = self.list_files().await?;
        let keep = self.limits.max_files - 1;
        for (index, path) in files.iter().rev() {
            if *index + 1 > keep {
                fs::remove_file(path)
                    .await
                    .map_err(|error| self.error("removing", path, error))?;
            } else {
                let to = self.rotated_path(*index + 1);
                fs::rename(path, &to)
                    .await
                    .map_err(|error| self.error("renaming", path, error))?;
            }
        }
        Ok(())
    }

    /// The log's files as `(index, path)`, newest first: the active file is
    /// index 0.
    async fn list_files(&self) -> Result<Vec<(usize, PathBuf)>, AuditError> {
        let mut entries = match fs::read_dir(&self.directory).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(self.error("reading", &self.directory, error)),
        };
        let mut files = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| self.error("reading", &self.directory, error))?
        {
            let name = entry.file_name();
            if let Some(index) = name.to_str().and_then(file_index) {
                files.push((index, entry.path()));
            }
        }
        files.sort();
        Ok(files)
    }

    /// The highest sequence number in the newest file that has one.
    async fn highest_seq(&self) -> Result<u64, AuditError> {
        for (_, path) in self.list_files().await? {
            let highest = self
                .read_lines(&path)
                .await?
                .iter()
                .filter_map(|line| decode_line(line))
                .map(|decoded| decoded.seq)
                .max();
            if let Some(highest) = highest {
                return Ok(highest);
            }
        }
        Ok(0)
    }

    async fn read_lines(&self, path: &Path) -> Result<Vec<String>, AuditError> {
        let bytes = fs::read(path)
            .await
            .map_err(|error| self.error("reading", path, error))?;
        // A crash can cut a multi-byte character; the lossy line fails to parse
        // and is skipped like any other cut-short line.
        Ok(String::from_utf8_lossy(&bytes)
            .split('\n')
            .filter(|line| !line.trim().is_empty())
            .map(str::to_owned)
            .collect())
    }

    fn error(&self, doing: &str, path: &Path, error: std::io::Error) -> AuditError {
        AuditError::new(format!("{doing} {}: {error}", path.display()))
    }

    async fn since_locked(
        &self,
        state: &mut State,
        from: WallTime,
    ) -> Result<Vec<AuditRecord>, AuditError> {
        if !self.prepare(state, false).await? {
            return Ok(Vec::new());
        }
        let mut records = Vec::new();
        for (_, path) in self.list_files().await?.iter().rev() {
            for line in self.read_lines(path).await? {
                if let Some(record) = decode_line(&line).and_then(|decoded| decoded.record) {
                    if record.at >= from {
                        records.push(record);
                    }
                }
            }
        }
        Ok(records)
    }

    async fn query_locked(
        &self,
        state: &mut State,
        query: AuditQuery,
    ) -> Result<AuditPage, AuditError> {
        let mut page = AuditPage {
            entries: Vec::new(),
            next: None,
        };
        if !self.prepare(state, false).await? {
            return Ok(page);
        }
        let before = query.cursor().map(AuditCursor::seq);
        // One more than the page, to learn whether older entries remain.
        let wanted = query.limit() + 1;
        let mut found = Vec::new();
        'files: for (_, path) in self.list_files().await? {
            for line in self.read_lines(&path).await?.iter().rev() {
                let Some(decoded) = decode_line(line) else {
                    continue;
                };
                let Some(record) = decoded.record else {
                    continue;
                };
                if before.is_some_and(|before| decoded.seq >= before) {
                    continue;
                }
                found.push(AuditEntry {
                    seq: decoded.seq,
                    record,
                });
                if found.len() == wanted {
                    break 'files;
                }
            }
        }
        if found.len() == wanted {
            found.pop();
            page.next = found.last().map(|entry| AuditCursor::from_seq(entry.seq));
        }
        page.entries = found;
        Ok(page)
    }
}

impl AuditLog for JsonlAuditLog {
    fn append(
        &self,
        record: AuditRecord,
        durability: Durability,
    ) -> BoxFuture<'_, Result<(), AuditError>> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            self.append_locked(&mut state, &record, durability).await
        })
    }

    fn since(&self, from: WallTime) -> BoxFuture<'_, Result<Vec<AuditRecord>, AuditError>> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            self.since_locked(&mut state, from).await
        })
    }

    fn query(&self, query: AuditQuery) -> BoxFuture<'_, Result<AuditPage, AuditError>> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            self.query_locked(&mut state, query).await
        })
    }
}

/// `0` for the active file, `n` for `audit.n.jsonl`, and `None` for anything
/// else in the directory.
fn file_index(name: &str) -> Option<usize> {
    if name == ACTIVE {
        return Some(0);
    }
    let digits = name.strip_prefix("audit.")?.strip_suffix(".jsonl")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|index| *index > 0)
}

async fn ends_with_newline(path: &Path) -> std::io::Result<bool> {
    let mut file = File::open(path).await?;
    file.seek(SeekFrom::End(-1)).await?;
    let mut last = [0u8; 1];
    file.read_exact(&mut last).await?;
    Ok(last[0] == b'\n')
}

#[cfg(unix)]
fn check_permissions(path: &Path, metadata: &std::fs::Metadata) -> Result<(), AuditError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = metadata.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(AuditError::new(format!(
            "{} has permissions {mode:o}, wider than owner-only; fix them before the audit log is used",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_permissions(_: &Path, _: &std::fs::Metadata) -> Result<(), AuditError> {
    Ok(())
}

// ---- The line format ----

fn encode(seq: u64, record: &AuditRecord) -> String {
    let mut object = Map::new();
    object.insert("seq".into(), json!(seq));
    object.insert("schema".into(), json!(record.schema));
    object.insert("run".into(), json!(record.run.as_millis()));
    object.insert("at".into(), json!(record.at.as_millis()));
    object.insert("operation".into(), json!(record.operation.map(|id| id.0)));
    object.insert(
        "principal".into(),
        match &record.principal {
            Principal::Operator => json!("operator"),
            Principal::Agent(label) => json!({ "agent": label.as_str() }),
        },
    );
    object.insert(
        "receiver".into(),
        json!(record.receiver.as_ref().map(ReceiverId::as_str)),
    );
    object.insert("event".into(), encode_event(&record.event));
    Value::Object(object).to_string()
}

fn texts(items: &[String], max: usize) -> Vec<String> {
    items.iter().map(|item| bounded(item, max)).collect()
}

fn field_names(fields: &[CoreField]) -> Vec<&'static str> {
    fields.iter().copied().map(core_field_name).collect()
}

fn encode_event(event: &AuditEvent) -> Value {
    match event {
        AuditEvent::Decided {
            intent,
            decision,
            reasons,
            rules,
            baseline,
            policy,
        } => json!({
            "kind": "decided",
            "intent": bounded(intent, MAX_INTENT_TEXT),
            "decision": match decision {
                AuditDecision::Allow => "allow",
                AuditDecision::RequireApproval => "require_approval",
                AuditDecision::Deny => "deny",
            },
            "reasons": texts(reasons, MAX_REASON_TEXT),
            "rules": texts(rules, MAX_INTENT_TEXT),
            "baseline": field_names(baseline),
            "policy": policy.map(|digest| digest.to_string()),
        }),
        AuditEvent::Dispatching {
            intent,
            before,
            target,
            precondition_fields,
        } => json!({
            "kind": "dispatching",
            "intent": bounded(intent, MAX_INTENT_TEXT),
            "before": before,
            "target": target,
            "precondition_fields": field_names(precondition_fields),
        }),
        AuditEvent::Finished {
            status,
            dispatch,
            confirmed,
            reason,
        } => json!({
            "kind": "finished",
            "status": bounded(status, MAX_STATUS_TEXT),
            "dispatch": dispatch.as_str(),
            "confirmed": confirmed,
            "reason": reason.as_deref().map(|reason| bounded(reason, MAX_REASON_TEXT)),
        }),
        AuditEvent::PolicyLoaded { digest } => json!({
            "kind": "policy_loaded",
            "digest": digest.to_string(),
        }),
        AuditEvent::PolicyLoadFailed { error } => json!({
            "kind": "policy_load_failed",
            "error": bounded(error, MAX_REASON_TEXT),
        }),
    }
}

/// A line that parsed: its sequence number, and the record when this code can
/// use it.
struct Decoded {
    seq: u64,
    record: Option<AuditRecord>,
}

fn decode_line(line: &str) -> Option<Decoded> {
    let Value::Object(object) = serde_json::from_str(line).ok()? else {
        return None;
    };
    let seq = object.get("seq")?.as_u64()?;
    Some(Decoded {
        seq,
        record: decode_record(&object),
    })
}

fn decode_record(object: &Map<String, Value>) -> Option<AuditRecord> {
    let schema = u32::try_from(object.get("schema")?.as_u64()?).ok()?;
    if schema != AUDIT_SCHEMA {
        return None;
    }
    let operation = match object.get("operation")? {
        Value::Null => None,
        value => Some(OperationId(value.as_u64()?)),
    };
    let principal = match object.get("principal")? {
        Value::String(name) if name == "operator" => Principal::Operator,
        Value::Object(agent) => {
            Principal::Agent(AgentLabel::new(agent.get("agent")?.as_str()?).ok()?)
        }
        _ => return None,
    };
    let receiver = match object.get("receiver")? {
        Value::Null => None,
        value => Some(parse_receiver(value.as_str()?)?),
    };
    Some(AuditRecord {
        schema,
        run: WallTime(object.get("run")?.as_u64()?),
        at: WallTime(object.get("at")?.as_u64()?),
        operation,
        principal,
        receiver,
        event: decode_event(object.get("event")?.as_object()?)?,
    })
}

fn parse_receiver(text: &str) -> Option<ReceiverId> {
    ReceiverId::new(text).ok().or_else(|| {
        // A receiver chosen by address has an id a configuration entry cannot.
        let marker = ReceiverId::ad_hoc("x").ok()?;
        let prefix = marker.as_str().strip_suffix('x')?;
        ReceiverId::ad_hoc(text.strip_prefix(prefix)?).ok()
    })
}

fn strings(value: Option<&Value>) -> Option<Vec<String>> {
    value?
        .as_array()?
        .iter()
        .map(|item| item.as_str().map(str::to_owned))
        .collect()
}

/// Field names this code knows. A name it does not know is dropped, because it
/// is informational and the record is still true without it.
fn fields(value: Option<&Value>) -> Option<Vec<CoreField>> {
    Some(
        strings(value)?
            .iter()
            .filter_map(|name| parse_core_field(name))
            .collect(),
    )
}

fn optional_i16(value: Option<&Value>) -> Option<Option<i16>> {
    match value? {
        Value::Null => Some(None),
        number => Some(Some(i16::try_from(number.as_i64()?).ok()?)),
    }
}

fn decode_event(event: &Map<String, Value>) -> Option<AuditEvent> {
    match event.get("kind")?.as_str()? {
        "decided" => Some(AuditEvent::Decided {
            intent: event.get("intent")?.as_str()?.to_owned(),
            decision: match event.get("decision")?.as_str()? {
                "allow" => AuditDecision::Allow,
                "require_approval" => AuditDecision::RequireApproval,
                "deny" => AuditDecision::Deny,
                _ => return None,
            },
            reasons: strings(event.get("reasons"))?,
            rules: strings(event.get("rules"))?,
            baseline: fields(event.get("baseline"))?,
            policy: match event.get("policy")? {
                Value::Null => None,
                value => Some(PolicyDigest::from_hex(value.as_str()?)?),
            },
        }),
        "dispatching" => Some(AuditEvent::Dispatching {
            intent: event.get("intent")?.as_str()?.to_owned(),
            before: optional_i16(event.get("before"))?,
            target: optional_i16(event.get("target"))?,
            precondition_fields: fields(event.get("precondition_fields"))?,
        }),
        "finished" => Some(AuditEvent::Finished {
            status: event.get("status")?.as_str()?.to_owned(),
            dispatch: match event.get("dispatch")?.as_str()? {
                "not_dispatched" => DispatchCertainty::NotDispatched,
                "possibly_dispatched" => DispatchCertainty::PossiblyDispatched,
                "complete_write" => DispatchCertainty::CompleteWrite,
                "unknown" => DispatchCertainty::Unknown,
                _ => return None,
            },
            confirmed: event.get("confirmed")?.as_bool()?,
            reason: match event.get("reason")? {
                Value::Null => None,
                value => Some(value.as_str()?.to_owned()),
            },
        }),
        "policy_loaded" => Some(AuditEvent::PolicyLoaded {
            digest: PolicyDigest::from_hex(event.get("digest")?.as_str()?)?,
        }),
        "policy_load_failed" => Some(AuditEvent::PolicyLoadFailed {
            error: event.get("error")?.as_str()?.to_owned(),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_logs_own_files_have_an_index() {
        assert_eq!(file_index("audit.jsonl"), Some(0));
        assert_eq!(file_index("audit.1.jsonl"), Some(1));
        assert_eq!(file_index("audit.12.jsonl"), Some(12));
        for other in [
            "audit.0.jsonl",
            "audit..jsonl",
            "audit.x.jsonl",
            "audit.1.jsonl.bak",
            "audit.-1.jsonl",
            "notes.txt",
            "audit",
        ] {
            assert_eq!(file_index(other), None, "{other}");
        }
    }
}
