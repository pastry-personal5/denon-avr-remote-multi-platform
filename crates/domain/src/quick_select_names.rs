//! Receiver-owned Quick Select display names returned by AppCommand.

use super::{Freshness, QuickSelectName};
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickSelectNameResponseEvidence {
    Complete,
    Partial,
    Unsupported,
    Malformed,
    Timeout,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickSelectNameObservation {
    pub names: [Option<QuickSelectName>; 4],
    pub sources: [Option<String>; 4],
    pub freshness: Freshness,
    pub generation: u64,
    pub observed_at: Option<SystemTime>,
    pub error: Option<String>,
    pub raw_response: String,
    pub response_evidence: QuickSelectNameResponseEvidence,
}

impl Default for QuickSelectNameObservation {
    fn default() -> Self {
        Self {
            names: [None, None, None, None],
            sources: [None, None, None, None],
            freshness: Freshness::Unknown,
            generation: 0,
            observed_at: None,
            error: None,
            raw_response: String::new(),
            response_evidence: QuickSelectNameResponseEvidence::Unsupported,
        }
    }
}
