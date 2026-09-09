//! Candidate AppCommand source-catalog protocol.
//!
//! This module deliberately models only the observed read shape.  It contains
//! no HTTP, sockets, or write commands.  Product use remains gated by the
//! model/firmware validation capability.

use denon_avr_domain::{CatalogResponseEvidence, SourceEntry, SourceId, SourceVisibility};
use quick_xml::escape::unescape;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::BTreeMap;

// X3800H AppCommand firmware requires line-delimited elements. Keep this
// read-only request in the same accepted wire shape as the information batch.
pub const SOURCE_CATALOG_REQUEST_XML: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<tx>\n<cmd id=\"1\">GetRenameSource</cmd>\n<cmd id=\"1\">GetDeletedSource</cmd>\n</tx>";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSourceCatalog {
    pub entries: Vec<SourceEntry>,
    pub evidence: CatalogResponseEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceCatalogProtocolError {
    MalformedXml(String),
    ConflictingEntry(String),
}

impl std::fmt::Display for SourceCatalogProtocolError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedXml(error) => write!(formatter, "malformed source catalog XML: {error}"),
            Self::ConflictingEntry(id) => {
                write!(formatter, "conflicting source catalog entry for {id}")
            }
        }
    }
}

impl std::error::Error for SourceCatalogProtocolError {}

pub fn source_catalog_request_xml() -> &'static str {
    SOURCE_CATALOG_REQUEST_XML
}

/// Parse the candidate `functionrename` and `functiondelete` response rows.
/// Unknown functions are ignored; absent or malformed visibility is represented
/// as `Unknown`, never as hidden.
pub fn parse_source_catalog_response(
    xml: &str,
) -> Result<ParsedSourceCatalog, SourceCatalogProtocolError> {
    let rows = xml_rows(xml)?;
    let mut entries: BTreeMap<String, SourceEntry> = BTreeMap::new();
    let mut saw_rename = false;
    let mut saw_delete = false;

    for row in rows {
        match row.section.as_str() {
            "functionrename" => {
                saw_rename = true;
                let Some(id) = row.value("name") else {
                    continue;
                };
                let id = id.trim();
                if id.is_empty() {
                    continue;
                }
                let source_id = SourceId::new(id)
                    .map_err(|error| SourceCatalogProtocolError::MalformedXml(error.to_string()))?;
                let entry = entries.entry(id.to_owned()).or_insert_with(|| SourceEntry {
                    id: source_id,
                    display_name: None,
                    visibility: SourceVisibility::Unknown,
                });
                let label = row
                    .value("rename")
                    .map(str::trim)
                    .filter(|label| !label.is_empty());
                if let (Some(previous), Some(label)) = (&entry.display_name, label) {
                    if previous != label {
                        return Err(SourceCatalogProtocolError::ConflictingEntry(id.into()));
                    }
                } else if let Some(label) = label {
                    entry.display_name = Some(label.to_owned());
                }
            }
            "functiondelete" => {
                saw_delete = true;
                let Some(id) = row.value("funcname") else {
                    continue;
                };
                let id = id.trim();
                if id.is_empty() {
                    continue;
                }
                let source_id = SourceId::new(id)
                    .map_err(|error| SourceCatalogProtocolError::MalformedXml(error.to_string()))?;
                let entry = entries.entry(id.to_owned()).or_insert_with(|| SourceEntry {
                    id: source_id,
                    display_name: None,
                    visibility: SourceVisibility::Unknown,
                });
                let visibility = match row.value("use").map(str::trim) {
                    Some("0") => SourceVisibility::Hidden,
                    Some("1") => SourceVisibility::Shown,
                    _ => SourceVisibility::Unknown,
                };
                if entry.visibility != SourceVisibility::Unknown
                    && visibility != SourceVisibility::Unknown
                    && entry.visibility != visibility
                {
                    return Err(SourceCatalogProtocolError::ConflictingEntry(id.into()));
                }
                if visibility != SourceVisibility::Unknown {
                    entry.visibility = visibility;
                }
            }
            _ => {}
        }
    }

    Ok(ParsedSourceCatalog {
        entries: entries.into_values().collect(),
        evidence: match (saw_rename, saw_delete) {
            (true, true) => CatalogResponseEvidence::Complete,
            (true, false) | (false, true) => CatalogResponseEvidence::Partial,
            (false, false) => CatalogResponseEvidence::Unsupported,
        },
    })
}

#[derive(Default)]
struct Row {
    section: String,
    values: BTreeMap<String, String>,
}
impl Row {
    fn value(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }
}

fn xml_rows(xml: &str) -> Result<Vec<Row>, SourceCatalogProtocolError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut path = Vec::<String>::new();
    let mut rows = Vec::new();
    let mut row: Option<Row> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => {
                let name = std::str::from_utf8(element.name().as_ref())
                    .map_err(|error| SourceCatalogProtocolError::MalformedXml(error.to_string()))?
                    .to_ascii_lowercase();
                path.push(name.clone());
                if name == "list"
                    && path.len() >= 2
                    && matches!(
                        path[path.len() - 2].as_str(),
                        "functionrename" | "functiondelete"
                    )
                {
                    row = Some(Row {
                        section: path[path.len() - 2].clone(),
                        ..Row::default()
                    });
                }
            }
            Ok(Event::Text(text)) => append_text(row.as_mut(), &path, text.as_ref())?,
            Ok(Event::CData(text)) => append_text(row.as_mut(), &path, text.as_ref())?,
            Ok(Event::GeneralRef(reference)) => {
                let entity = std::str::from_utf8(reference.as_ref())
                    .map_err(|error| SourceCatalogProtocolError::MalformedXml(error.to_string()))?;
                append_text(row.as_mut(), &path, format!("&{entity};").as_bytes())?;
            }
            Ok(Event::End(element)) => {
                let name = std::str::from_utf8(element.name().as_ref())
                    .map_err(|error| SourceCatalogProtocolError::MalformedXml(error.to_string()))?
                    .to_ascii_lowercase();
                if name == "list" {
                    if let Some(completed) = row.take() {
                        rows.push(completed);
                    }
                }
                path.pop();
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => return Err(SourceCatalogProtocolError::MalformedXml(error.to_string())),
        }
    }
    if !path.is_empty() {
        return Err(SourceCatalogProtocolError::MalformedXml(
            "unclosed element".into(),
        ));
    }
    Ok(rows)
}

fn append_text(
    row: Option<&mut Row>,
    path: &[String],
    bytes: &[u8],
) -> Result<(), SourceCatalogProtocolError> {
    let Some(row) = row else { return Ok(()) };
    let Some(name) = path.last() else {
        return Ok(());
    };
    if name == "list" {
        return Ok(());
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|error| SourceCatalogProtocolError::MalformedXml(error.to_string()))?;
    let decoded = unescape(text)
        .map_err(|error| SourceCatalogProtocolError::MalformedXml(error.to_string()))?;
    row.values
        .entry(name.clone())
        .or_default()
        .push_str(&decoded);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_is_the_exact_read_only_candidate_shape() {
        assert_eq!(source_catalog_request_xml(), SOURCE_CATALOG_REQUEST_XML);
        assert!(!source_catalog_request_xml().contains("Set"));
    }

    #[test]
    fn parses_escaped_labels_and_visibility_without_hiding_unknowns() {
        let parsed = parse_source_catalog_response(
            "<rx><functionrename><list><name>GAME</name><rename>PlayStation &amp; TV</rename></list></functionrename><functiondelete><list><FuncName>GAME</FuncName><use>1</use></list><list><FuncName>AUX1</FuncName><use>0</use></list><list><FuncName>CD</FuncName><use>x</use></list></functiondelete></rx>",
        )
        .unwrap();
        assert_eq!(parsed.evidence, CatalogResponseEvidence::Complete);
        let game = parsed
            .entries
            .iter()
            .find(|entry| entry.id.as_str() == "GAME")
            .unwrap();
        assert_eq!(game.display_name.as_deref(), Some("PlayStation & TV"));
        assert_eq!(game.visibility, SourceVisibility::Shown);
        assert_eq!(
            parsed
                .entries
                .iter()
                .find(|entry| entry.id.as_str() == "AUX1")
                .unwrap()
                .visibility,
            SourceVisibility::Hidden
        );
        assert_eq!(
            parsed
                .entries
                .iter()
                .find(|entry| entry.id.as_str() == "CD")
                .unwrap()
                .visibility,
            SourceVisibility::Unknown
        );
    }

    #[test]
    fn rejects_conflicting_rows() {
        assert!(matches!(
            parse_source_catalog_response("<rx><functiondelete><list><FuncName>GAME</FuncName><use>0</use></list><list><FuncName>GAME</FuncName><use>1</use></list></functiondelete></rx>"),
            Err(SourceCatalogProtocolError::ConflictingEntry(_))
        ));
    }

    #[test]
    fn rejects_malformed_xml_instead_of_treating_it_as_hidden() {
        assert!(matches!(
            parse_source_catalog_response("<rx><functiondelete>"),
            Err(SourceCatalogProtocolError::MalformedXml(_))
        ));
    }
}
