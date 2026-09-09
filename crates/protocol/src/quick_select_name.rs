//! Parser for the receiver-advertised `GetQuickSelectName` AppCommand reply.

use quick_xml::events::Event;
use quick_xml::Reader;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedQuickSelectNames {
    pub names: [Option<String>; 4],
    pub sources: [Option<String>; 4],
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuickSelectNameProtocolError {
    MalformedXml(String),
    MissingResponse,
}

impl std::fmt::Display for QuickSelectNameProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedXml(error) => write!(f, "malformed Quick Select name XML: {error}"),
            Self::MissingResponse => f.write_str("Quick Select name response is empty"),
        }
    }
}
impl std::error::Error for QuickSelectNameProtocolError {}

pub fn parse_quick_select_names(
    xml: &str,
) -> Result<ParsedQuickSelectNames, QuickSelectNameProtocolError> {
    if xml.trim().is_empty() {
        return Err(QuickSelectNameProtocolError::MissingResponse);
    }
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut names: [Option<String>; 4] = [None, None, None, None];
    let mut sources: [Option<String>; 4] = [None, None, None, None];
    let mut current: Option<(bool, usize)> = None;
    let mut saw_root = false;
    let mut saw_command = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => {
                let tag = element.name().as_ref().to_vec();
                if tag.as_slice() == b"rx" {
                    if saw_root {
                        return Err(QuickSelectNameProtocolError::MalformedXml(
                            "duplicate rx root".into(),
                        ));
                    }
                    saw_root = true;
                } else if tag.as_slice() == b"cmd" {
                    if !saw_root || saw_command {
                        return Err(QuickSelectNameProtocolError::MalformedXml(
                            "unexpected cmd element".into(),
                        ));
                    }
                    saw_command = true;
                } else if let Some(index) = indexed_field(&tag, b"Name") {
                    current = Some((true, index));
                } else if let Some(index) = indexed_field(&tag, b"Source") {
                    current = Some((false, index));
                }
            }
            Ok(Event::Text(text)) if current.is_some() => {
                let value = text
                    .decode()
                    .map_err(|error| QuickSelectNameProtocolError::MalformedXml(error.to_string()))?
                    .into_owned();
                if let Some((is_name, index)) = current {
                    if is_name {
                        names[index] = Some(value);
                    } else {
                        sources[index] = Some(value);
                    }
                }
            }
            Ok(Event::End(element)) => {
                let tag = element.name().as_ref().to_vec();
                if tag.as_slice() == b"rx" {
                    saw_root = false;
                } else if tag.as_slice() == b"cmd"
                    || indexed_field(&tag, b"Name").is_some()
                    || indexed_field(&tag, b"Source").is_some()
                {
                    current = None;
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(QuickSelectNameProtocolError::MalformedXml(
                    error.to_string(),
                ))
            }
            _ => {}
        }
    }
    if !saw_command {
        return Err(QuickSelectNameProtocolError::MalformedXml(
            "response has no cmd".into(),
        ));
    }
    Ok(ParsedQuickSelectNames {
        complete: names.iter().all(Option::is_some),
        names,
        sources,
    })
}

fn indexed_field(tag: &[u8], prefix: &[u8]) -> Option<usize> {
    let suffix = tag.strip_prefix(prefix)?;
    let number = std::str::from_utf8(suffix).ok()?.parse::<usize>().ok()?;
    (1..=4).contains(&number).then_some(number - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_padded_names_and_sources() {
        let parsed = parse_quick_select_names("<rx><cmd><Name1>Movie   </Name1><Name2>Game</Name2><Name3>N3</Name3><Name4>N4</Name4><Source1>BD</Source1></cmd></rx>").unwrap();
        assert_eq!(parsed.names[0].as_deref(), Some("Movie   "));
        assert_eq!(parsed.sources[0].as_deref(), Some("BD"));
        assert!(parsed.complete);
    }
    #[test]
    fn rejects_empty_or_missing_command() {
        assert_eq!(
            parse_quick_select_names(" "),
            Err(QuickSelectNameProtocolError::MissingResponse)
        );
        assert!(parse_quick_select_names("<rx/>").is_err());
    }
}
