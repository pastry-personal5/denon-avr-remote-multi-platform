//! Where the server's files are, and how a client builds a request path.

use denon_avr_domain::ReceiverId;
use std::path::{Path, PathBuf};

/// The server's files under its data directory: the Operator endpoint and the
/// lock in `run/`, and the tokens in `credentials/`. Both directories are the
/// server's own and are kept at 0700; the data directory's mode is not touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointPaths {
    pub run_directory: PathBuf,
    pub credentials_directory: PathBuf,
    pub operator_socket: PathBuf,
    pub lock_file: PathBuf,
    pub operator_token: PathBuf,
    pub agent_tokens: PathBuf,
}

impl EndpointPaths {
    pub fn under(data_directory: &Path) -> Self {
        let run_directory = data_directory.join("run");
        let credentials_directory = data_directory.join("credentials");
        Self {
            operator_socket: run_directory.join("operator.sock"),
            lock_file: run_directory.join("server.lock"),
            operator_token: credentials_directory.join("operator.token"),
            agent_tokens: credentials_directory.join("agent-tokens.json"),
            run_directory,
            credentials_directory,
        }
    }
}

/// `text` as one path segment: everything but the unreserved characters of RFC
/// 3986 becomes `%XX`, so a `/`, a `?`, a `%`, a space, or a letter outside ASCII
/// in a receiver id stays inside its segment.
pub fn encode_segment(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The path of each resource, as a client builds it. The patterns a server
/// routes are in [`crate::routes::TABLE`].
pub mod request {
    use super::*;

    pub fn health() -> String {
        "/v1/health".into()
    }
    pub fn receivers() -> String {
        "/v1/receivers".into()
    }
    pub fn state(receiver: &ReceiverId) -> String {
        format!("/v1/receivers/{}/state", encode_segment(receiver.as_str()))
    }
    pub fn sources(receiver: &ReceiverId) -> String {
        format!(
            "/v1/receivers/{}/sources",
            encode_segment(receiver.as_str())
        )
    }
    pub fn state_events(receiver: &ReceiverId) -> String {
        format!("/v1/receivers/{}/events", encode_segment(receiver.as_str()))
    }
    pub fn operation_events() -> String {
        "/v1/operations/events".into()
    }
    pub fn submit(receiver: &ReceiverId) -> String {
        format!(
            "/v1/receivers/{}/operations",
            encode_segment(receiver.as_str())
        )
    }
    pub fn dry_run(receiver: &ReceiverId) -> String {
        format!(
            "/v1/receivers/{}/operations/dry-run",
            encode_segment(receiver.as_str())
        )
    }
    pub fn operation(id: u64, wait_ms: Option<u64>) -> String {
        match wait_ms {
            Some(wait) => format!("/v1/operations/{id}?wait_ms={wait}"),
            None => format!("/v1/operations/{id}"),
        }
    }
    pub fn cancel(id: u64) -> String {
        format!("/v1/operations/{id}/cancel")
    }
    pub fn discover() -> String {
        "/v1/receivers/discover".into()
    }
    pub fn ad_hoc() -> String {
        "/v1/receivers/ad-hoc".into()
    }
    pub fn config() -> String {
        "/v1/config".into()
    }
    pub fn quick_select_names(receiver: &ReceiverId) -> String {
        format!(
            "/v1/receivers/{}/quick-select-names",
            encode_segment(receiver.as_str())
        )
    }
    pub fn http_information(receiver: &ReceiverId) -> String {
        format!(
            "/v1/receivers/{}/http-information",
            encode_segment(receiver.as_str())
        )
    }
    pub fn refresh(receiver: &ReceiverId) -> String {
        format!(
            "/v1/receivers/{}/refresh",
            encode_segment(receiver.as_str())
        )
    }
    pub fn policy() -> String {
        "/v1/policy".into()
    }
    pub fn policy_reload() -> String {
        "/v1/policy/reload".into()
    }
    pub fn audit(limit: usize, cursor: Option<u64>) -> String {
        match cursor {
            Some(cursor) => format!("/v1/audit?limit={limit}&cursor={cursor}"),
            None => format!("/v1/audit?limit={limit}"),
        }
    }
    pub fn tokens() -> String {
        "/v1/tokens".into()
    }
    pub fn token(id: &str) -> String {
        format!("/v1/tokens/{}", encode_segment(id))
    }
}
