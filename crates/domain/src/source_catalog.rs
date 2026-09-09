//! Receiver-owned source labels and visibility.
//!
//! Canonical source IDs are kept separate from their receiver-provided labels:
//! labels are presentation data and must never be sent as AVR input commands.

use super::Freshness;
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(String);

impl SourceId {
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            return Err("source ID must be non-empty and contain no control characters");
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SourceId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceVisibility {
    Shown,
    Hidden,
    /// The candidate reply had no unambiguous visibility row for this ID.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEntry {
    pub id: SourceId,
    /// A non-empty receiver-provided label. `None` falls back to the canonical
    /// source label in presentation code.
    pub display_name: Option<String>,
    pub visibility: SourceVisibility,
}

impl SourceEntry {
    pub fn display_name_or<'a>(&'a self, fallback: &'a str) -> &'a str {
        self.display_name
            .as_deref()
            .filter(|label| !label.trim().is_empty())
            .unwrap_or(fallback)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCatalog {
    pub entries: Vec<SourceEntry>,
    pub freshness: Freshness,
    pub generation: u64,
    pub observed_at: Option<SystemTime>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogResponseEvidence {
    Complete,
    Partial,
    Unsupported,
    Malformed,
    Timeout,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCatalogObservation {
    pub catalog: SourceCatalog,
    /// Retained for diagnostics and provenance; it is never used as a label.
    pub raw_response: String,
    pub response_evidence: CatalogResponseEvidence,
}

impl Default for SourceCatalog {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            freshness: Freshness::Unknown,
            generation: 0,
            observed_at: None,
            error: None,
        }
    }
}

impl SourceCatalog {
    pub fn invalidate(&mut self, generation: u64) {
        *self = Self {
            freshness: Freshness::Invalidated,
            generation,
            ..Self::default()
        };
    }

    pub fn entry(&self, id: &str) -> Option<&SourceEntry> {
        self.entries.iter().find(|entry| entry.id.as_str() == id)
    }

    pub fn selectable_entries<'a>(
        &'a self,
        validated_inputs: &'a [&'a str],
    ) -> impl Iterator<Item = &'a SourceEntry> {
        self.entries.iter().filter(move |entry| {
            entry.visibility == SourceVisibility::Shown
                && validated_inputs.contains(&entry.id.as_str())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_and_unknown_entries_are_not_selectable() {
        let catalog = SourceCatalog {
            entries: vec![
                SourceEntry {
                    id: SourceId::new("GAME").unwrap(),
                    display_name: Some("Console".into()),
                    visibility: SourceVisibility::Shown,
                },
                SourceEntry {
                    id: SourceId::new("AUX1").unwrap(),
                    display_name: None,
                    visibility: SourceVisibility::Hidden,
                },
            ],
            ..SourceCatalog::default()
        };
        assert_eq!(
            catalog
                .selectable_entries(&["GAME", "AUX1"])
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            vec!["GAME"]
        );
    }
}
