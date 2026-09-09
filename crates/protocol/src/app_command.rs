//! Denon AppCommand XML request and response protocol.
//!
//! AppCommand is exposed by receiver firmware and is not documented by Denon
//! as a stable cross-model API. This module therefore preserves unknown
//! commands, attributes, control codes, and padded values without assigning
//! receiver-independent semantics to them.

use quick_xml::escape::unescape;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer, XmlVersion};
use std::collections::BTreeMap;

pub const APP_COMMAND_0300_PATH: &str = "/goform/AppCommand0300.xml";
pub const APP_COMMAND_0301_PATH: &str = "/goform/AppCommand0301.xml";
const QUERY_ID: &str = "3";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppCommandQuery {
    name: String,
    parameters: Vec<String>,
}

impl AppCommandQuery {
    pub fn new(
        name: impl Into<String>,
        parameters: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, AppCommandProtocolError> {
        let name = validated_identifier(name.into(), "command name")?;
        if !name.starts_with("Get") {
            return Err(AppCommandProtocolError::StateChangingQuery);
        }
        let parameters = parameters
            .into_iter()
            .map(|parameter| validated_identifier(parameter.into(), "parameter name"))
            .collect::<Result<Vec<_>, _>>()?;
        if parameters.is_empty() {
            return Err(AppCommandProtocolError::EmptyParameterList);
        }
        Ok(Self { name, parameters })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn parameters(&self) -> &[String] {
        &self.parameters
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppCommandRequest {
    queries: Vec<AppCommandQuery>,
}

impl AppCommandRequest {
    pub fn new(queries: Vec<AppCommandQuery>) -> Result<Self, AppCommandProtocolError> {
        if queries.is_empty() {
            return Err(AppCommandProtocolError::EmptyRequest);
        }
        Ok(Self { queries })
    }

    pub fn audio_information() -> Self {
        Self::new(vec![
            AppCommandQuery::new("GetInputSignal", ["inputsigall"])
                .expect("built-in query is valid"),
            AppCommandQuery::new("GetActiveSpeaker", ["activespall"])
                .expect("built-in query is valid"),
            AppCommandQuery::new("GetVideoInfo", ["videooutput", "hdmisigin", "hdmisigout"])
                .expect("built-in query is valid"),
            AppCommandQuery::new(
                "GetAudioInfo",
                ["inputmode", "output", "signal", "sound", "fs"],
            )
            .expect("built-in query is valid"),
            AppCommandQuery::new(
                "GetAudyssyInfo",
                ["eqname", "eqvalue", "dynamiceq", "dynamicvol"],
            )
            .expect("built-in query is valid"),
        ])
        .expect("built-in request is valid")
    }

    pub fn input_signal() -> Self {
        Self::new(vec![AppCommandQuery::new(
            "GetInputSignal",
            ["inputsigall"],
        )
        .expect("built-in query is valid")])
        .expect("built-in request is valid")
    }

    pub fn active_speaker() -> Self {
        Self::new(vec![AppCommandQuery::new(
            "GetActiveSpeaker",
            ["activespall"],
        )
        .expect("built-in query is valid")])
        .expect("built-in request is valid")
    }

    pub fn audio_info() -> Self {
        Self::new(vec![AppCommandQuery::new(
            "GetAudioInfo",
            ["inputmode", "output", "signal", "sound", "fs"],
        )
        .expect("built-in query is valid")])
        .expect("built-in request is valid")
    }

    pub fn video_info() -> Self {
        Self::new(vec![AppCommandQuery::new(
            "GetVideoInfo",
            ["videooutput", "hdmisigin", "hdmisigout"],
        )
        .expect("built-in query is valid")])
        .expect("built-in request is valid")
    }

    pub fn audyssey_info() -> Self {
        Self::new(vec![AppCommandQuery::new(
            "GetAudyssyInfo",
            ["eqname", "eqvalue", "dynamiceq", "dynamicvol"],
        )
        .expect("built-in query is valid")])
        .expect("built-in request is valid")
    }

    pub fn queries(&self) -> &[AppCommandQuery] {
        &self.queries
    }

    pub fn to_xml(&self) -> Result<String, AppCommandProtocolError> {
        // AVR-X3800H firmware 6000-1060-0071-9831 accepts the documented
        // AppCommand structure only when its elements are line-delimited.
        // Compact, otherwise equivalent XML receives an empty rx response.
        // Keep this transport-facing wire shape stable rather than relying on
        // a receiver XML parser to normalize insignificant whitespace.
        let mut writer = Writer::new(Vec::new());
        writer
            .write_event(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)))
            .map_err(xml_error)?;
        write_line_break(&mut writer)?;
        writer
            .write_event(Event::Start(BytesStart::new("tx")))
            .map_err(xml_error)?;
        write_line_break(&mut writer)?;
        for query in &self.queries {
            let mut command = BytesStart::new("cmd");
            command.push_attribute(("id", QUERY_ID));
            writer
                .write_event(Event::Start(command))
                .map_err(xml_error)?;
            write_line_break(&mut writer)?;
            write_text_element(&mut writer, "name", query.name())?;
            write_line_break(&mut writer)?;
            writer
                .write_event(Event::Start(BytesStart::new("list")))
                .map_err(xml_error)?;
            write_line_break(&mut writer)?;
            for parameter in query.parameters() {
                let mut element = BytesStart::new("param");
                element.push_attribute(("name", parameter.as_str()));
                writer
                    .write_event(Event::Start(element))
                    .map_err(xml_error)?;
                writer
                    .write_event(Event::End(BytesEnd::new("param")))
                    .map_err(xml_error)?;
                write_line_break(&mut writer)?;
            }
            writer
                .write_event(Event::End(BytesEnd::new("list")))
                .map_err(xml_error)?;
            write_line_break(&mut writer)?;
            writer
                .write_event(Event::End(BytesEnd::new("cmd")))
                .map_err(xml_error)?;
            write_line_break(&mut writer)?;
        }
        writer
            .write_event(Event::End(BytesEnd::new("tx")))
            .map_err(xml_error)?;
        String::from_utf8(writer.into_inner()).map_err(|error| {
            AppCommandProtocolError::MalformedXml(format!("encoded XML is not UTF-8: {error}"))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppCommandParameter {
    pub name: String,
    pub attributes: BTreeMap<String, String>,
    /// Exact decoded text, including firmware-added padding.
    pub value: String,
}

impl AppCommandParameter {
    pub fn control(&self) -> Option<&str> {
        self.attributes.get("control").map(String::as_str)
    }

    pub fn control_code(&self) -> Option<u8> {
        self.control()?.parse().ok()
    }

    pub fn trimmed_value(&self) -> &str {
        self.value.trim()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppCommandResult {
    pub name: String,
    pub parameters: Vec<AppCommandParameter>,
}

impl AppCommandResult {
    pub fn parameter(&self, name: &str) -> Option<&AppCommandParameter> {
        self.parameters
            .iter()
            .find(|parameter| parameter.name == name)
    }

    pub fn parameters_with_control(&self, code: u8) -> impl Iterator<Item = &AppCommandParameter> {
        self.parameters
            .iter()
            .filter(move |parameter| parameter.control_code() == Some(code))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppCommandResponse {
    pub raw_xml: String,
    pub commands: Vec<AppCommandResult>,
}

impl AppCommandResponse {
    pub fn command(&self, name: &str) -> Option<&AppCommandResult> {
        self.commands.iter().find(|command| command.name == name)
    }

    pub fn parameters(&self) -> impl Iterator<Item = (&str, &AppCommandParameter)> {
        self.commands.iter().flat_map(|command| {
            command
                .parameters
                .iter()
                .map(move |parameter| (command.name.as_str(), parameter))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppCommandProtocolError {
    EmptyRequest,
    EmptyParameterList,
    InvalidIdentifier(&'static str),
    StateChangingQuery,
    MissingResponse,
    MissingResponseRoot,
    MultipleResponseRoots,
    CommandOutsideResponseRoot,
    NestedCommand,
    ParameterOutsideCommand,
    MissingCommandName,
    DuplicateCommandName,
    MalformedParameter,
    MalformedXml(String),
}

impl std::fmt::Display for AppCommandProtocolError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::EmptyRequest => "AppCommand request contains no queries",
            Self::EmptyParameterList => "AppCommand query contains no parameters",
            Self::InvalidIdentifier(kind) => return write!(formatter, "invalid {kind}"),
            Self::StateChangingQuery => {
                "state-changing AppCommand operation is not a read-only query"
            }
            Self::MissingResponse => "AppCommand response is empty",
            Self::MissingResponseRoot => "AppCommand response has no rx root element",
            Self::MultipleResponseRoots => "AppCommand response has multiple rx root elements",
            Self::CommandOutsideResponseRoot => {
                "AppCommand response command is outside the rx root element"
            }
            Self::NestedCommand => "AppCommand response contains nested command elements",
            Self::ParameterOutsideCommand => "AppCommand response parameter is outside a command",
            Self::MissingCommandName => "AppCommand response command has no name",
            Self::DuplicateCommandName => "AppCommand response command has multiple names",
            Self::MalformedParameter => "AppCommand response contains a malformed parameter",
            Self::MalformedXml(message) => message,
        })
    }
}

impl std::error::Error for AppCommandProtocolError {}

pub fn parse_app_command_response(
    xml: &str,
) -> Result<AppCommandResponse, AppCommandProtocolError> {
    if xml.trim().is_empty() {
        return Err(AppCommandProtocolError::MissingResponse);
    }
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut commands = Vec::new();
    let mut current: Option<CommandBuilder> = None;
    let mut saw_root = false;
    let mut inside_root = false;
    let mut closed_root = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) if element.name().as_ref() == b"rx" => {
                if saw_root {
                    return Err(AppCommandProtocolError::MultipleResponseRoots);
                }
                saw_root = true;
                inside_root = true;
            }
            Ok(Event::Empty(element)) if element.name().as_ref() == b"rx" => {
                if saw_root {
                    return Err(AppCommandProtocolError::MultipleResponseRoots);
                }
                saw_root = true;
                closed_root = true;
            }
            Ok(Event::End(element)) if element.name().as_ref() == b"rx" => {
                inside_root = false;
                closed_root = true;
            }
            Ok(Event::Start(element)) if element.name().as_ref() == b"cmd" => {
                if !inside_root {
                    return Err(AppCommandProtocolError::CommandOutsideResponseRoot);
                }
                if current.is_some() {
                    return Err(AppCommandProtocolError::NestedCommand);
                }
                current = Some(CommandBuilder::default());
            }
            Ok(Event::End(element)) if element.name().as_ref() == b"cmd" => {
                let command = current
                    .take()
                    .ok_or(AppCommandProtocolError::MissingCommandName)?;
                commands.push(command.finish()?);
            }
            Ok(Event::Start(element)) if element.name().as_ref() == b"name" => {
                let current = current
                    .as_mut()
                    .ok_or(AppCommandProtocolError::MissingCommandName)?;
                let text = decoded_text(&mut reader, element.name())?;
                if current.name.replace(text.trim().to_owned()).is_some() {
                    return Err(AppCommandProtocolError::DuplicateCommandName);
                }
            }
            Ok(Event::Start(element)) if element.name().as_ref() == b"param" => {
                let current = current
                    .as_mut()
                    .ok_or(AppCommandProtocolError::ParameterOutsideCommand)?;
                let (name, attributes) = parameter_attributes(&reader, &element)?;
                current.parameters.push(AppCommandParameter {
                    name,
                    attributes,
                    value: decoded_text(&mut reader, element.name())?,
                });
            }
            Ok(Event::Empty(element)) if element.name().as_ref() == b"param" => {
                let current = current
                    .as_mut()
                    .ok_or(AppCommandProtocolError::ParameterOutsideCommand)?;
                let (name, attributes) = parameter_attributes(&reader, &element)?;
                current.parameters.push(AppCommandParameter {
                    name,
                    attributes,
                    value: String::new(),
                });
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => return Err(xml_error(error)),
        }
    }
    if current.is_some() || inside_root || (saw_root && !closed_root) {
        return Err(AppCommandProtocolError::MalformedXml(
            "AppCommand response contains an unclosed element".to_owned(),
        ));
    }
    if !saw_root {
        return Err(AppCommandProtocolError::MissingResponseRoot);
    }
    Ok(AppCommandResponse {
        raw_xml: xml.to_owned(),
        commands,
    })
}

#[derive(Default)]
struct CommandBuilder {
    name: Option<String>,
    parameters: Vec<AppCommandParameter>,
}

impl CommandBuilder {
    fn finish(self) -> Result<AppCommandResult, AppCommandProtocolError> {
        let name = self
            .name
            .filter(|name| !name.is_empty())
            .ok_or(AppCommandProtocolError::MissingCommandName)?;
        Ok(AppCommandResult {
            name,
            parameters: self.parameters,
        })
    }
}

fn validated_identifier(
    value: String,
    kind: &'static str,
) -> Result<String, AppCommandProtocolError> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err(AppCommandProtocolError::InvalidIdentifier(kind));
    }
    Ok(value)
}

fn write_text_element(
    writer: &mut Writer<Vec<u8>>,
    name: &str,
    text: &str,
) -> Result<(), AppCommandProtocolError> {
    writer
        .write_event(Event::Start(BytesStart::new(name)))
        .map_err(xml_error)?;
    writer
        .write_event(Event::Text(BytesText::new(text)))
        .map_err(xml_error)?;
    writer
        .write_event(Event::End(BytesEnd::new(name)))
        .map_err(xml_error)
}

fn write_line_break(writer: &mut Writer<Vec<u8>>) -> Result<(), AppCommandProtocolError> {
    writer
        .write_event(Event::Text(BytesText::new("\n")))
        .map_err(xml_error)
}

fn decoded_text(
    reader: &mut Reader<&[u8]>,
    end: quick_xml::name::QName<'_>,
) -> Result<String, AppCommandProtocolError> {
    let value = reader
        .read_text(end)
        .map_err(xml_error)?
        .decode()
        .map_err(xml_error)?;
    unescape(&value)
        .map(|value| value.into_owned())
        .map_err(xml_error)
}

fn parameter_attributes(
    reader: &Reader<&[u8]>,
    element: &BytesStart<'_>,
) -> Result<(String, BTreeMap<String, String>), AppCommandProtocolError> {
    let mut attributes = BTreeMap::new();
    for attribute in element.attributes() {
        let attribute = attribute.map_err(xml_error)?;
        let name = std::str::from_utf8(attribute.key.as_ref())
            .map_err(xml_error)?
            .to_owned();
        let value = attribute
            .decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())
            .map_err(xml_error)?
            .into_owned();
        attributes.insert(name, value);
    }
    let name = attributes
        .remove("name")
        .filter(|name| !name.is_empty())
        .ok_or(AppCommandProtocolError::MalformedParameter)?;
    Ok((name, attributes))
}

fn xml_error(error: impl std::fmt::Display) -> AppCommandProtocolError {
    AppCommandProtocolError::MalformedXml(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_information_request_is_read_only_and_well_formed() {
        let request = AppCommandRequest::audio_information();
        let xml = request.to_xml().unwrap();
        assert_eq!(request.queries().len(), 5);
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>"));
        assert!(xml.contains("<name>GetAudioInfo</name>"));
        assert!(xml.contains("<name>GetInputSignal</name>"));
        assert!(xml.contains("<name>GetActiveSpeaker</name>"));
        assert!(xml.contains("<name>GetVideoInfo</name>"));
        assert!(xml.contains("<name>GetAudyssyInfo</name>"));
        assert!(xml.contains("<param name=\"inputsigall\"></param>"));
        assert!(xml.contains("\n<tx>\n"));
        assert!(xml.contains("\n<cmd id=\"3\">\n"));
        assert!(!xml.contains("Set"));
    }

    #[test]
    fn request_writer_escapes_dynamic_identifiers() {
        let request =
            AppCommandRequest::new(vec![AppCommandQuery::new("Get&A", ["a&b"]).unwrap()]).unwrap();
        let xml = request.to_xml().unwrap();
        assert!(xml.contains("<name>Get&amp;A</name>"));
        assert!(xml.contains("name=\"a&amp;b\""));
    }

    #[test]
    fn request_rejects_empty_and_control_character_identifiers() {
        assert_eq!(
            AppCommandRequest::new(Vec::new()),
            Err(AppCommandProtocolError::EmptyRequest)
        );
        assert_eq!(
            AppCommandQuery::new("Get", Vec::<String>::new()),
            Err(AppCommandProtocolError::EmptyParameterList)
        );
        assert_eq!(
            AppCommandQuery::new("Get\nBad", ["field"]),
            Err(AppCommandProtocolError::InvalidIdentifier("command name"))
        );
        assert_eq!(
            AppCommandQuery::new("SetInputFunction", ["input"]),
            Err(AppCommandProtocolError::StateChangingQuery)
        );
    }

    #[test]
    fn parses_live_x3800h_shape_losslessly() {
        let xml = r#"<?xml version="1.0" encoding="utf-8" ?>
<rx><cmd><name>GetAudioInfo</name><list>
<param name="signal" control="1">PCM                  </param>
<param name="fs" control="1">48 kHz</param>
</list></cmd><cmd><name>GetActiveSpeaker</name><list>
<param name="activespb2" control="2" future="kept">FL</param>
</list></cmd></rx>"#;
        let response = parse_app_command_response(xml).unwrap();
        assert_eq!(response.commands.len(), 2);
        assert_eq!(
            response.command("GetAudioInfo").unwrap().parameters[0].value,
            "PCM                  "
        );
        let active = &response.command("GetActiveSpeaker").unwrap().parameters[0];
        assert_eq!(active.control_code(), Some(2));
        assert_eq!(
            active.attributes.get("future").map(String::as_str),
            Some("kept")
        );
        assert_eq!(active.trimmed_value(), "FL");
        assert_eq!(
            response
                .command("GetActiveSpeaker")
                .unwrap()
                .parameters_with_control(2)
                .count(),
            1
        );
    }

    #[test]
    fn rejects_structurally_invalid_responses() {
        assert_eq!(
            parse_app_command_response(" "),
            Err(AppCommandProtocolError::MissingResponse)
        );
        assert_eq!(
            parse_app_command_response("<tx></tx>"),
            Err(AppCommandProtocolError::MissingResponseRoot)
        );
        assert_eq!(
            parse_app_command_response("<rx><param name=\"fs\">48K</param></rx>"),
            Err(AppCommandProtocolError::ParameterOutsideCommand)
        );
        assert_eq!(
            parse_app_command_response("<rx><cmd><list></list></cmd></rx>"),
            Err(AppCommandProtocolError::MissingCommandName)
        );
        assert_eq!(
            parse_app_command_response("<rx></rx><cmd><name>GetLate</name></cmd>"),
            Err(AppCommandProtocolError::CommandOutsideResponseRoot)
        );
        assert_eq!(
            parse_app_command_response("<rx></rx><rx></rx>"),
            Err(AppCommandProtocolError::MultipleResponseRoots)
        );
        assert_eq!(
            parse_app_command_response("<rx/>"),
            Ok(AppCommandResponse {
                raw_xml: "<rx/>".to_owned(),
                commands: Vec::new(),
            })
        );
    }
}
