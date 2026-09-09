//! Diagnostic-only HTTP information probe.
//!
//! This binary only issues the fixed `Get*` AppCommand queries. It preserves
//! receiver replies as evidence instead of assigning them capability semantics.
use denon_avr_domain::ReceiverEndpoint;
use denon_avr_infrastructure::{AppCommandHttpClient, RawHttpResponse};
use denon_avr_protocol::app_command::{
    parse_app_command_response, AppCommandRequest, AppCommandResponse, APP_COMMAND_0300_PATH,
    APP_COMMAND_0301_PATH,
};
use std::env;
use std::io::{self, Write};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const REQUESTED_COMMANDS: [&str; 5] = [
    "GetInputSignal",
    "GetActiveSpeaker",
    "GetVideoInfo",
    "GetAudioInfo",
    "GetAudyssyInfo",
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct Arguments {
    host: String,
    timeout: Duration,
    model: String,
    firmware: String,
    port: u16,
}

#[derive(Debug, Default)]
struct AttemptObservation {
    parsed: bool,
    empty_response: bool,
    returned_requested: Vec<String>,
}

impl AttemptObservation {
    fn usable(&self) -> bool {
        self.parsed && !self.returned_requested.is_empty()
    }
}

fn main() -> io::Result<()> {
    let arguments = parse_arguments(env::args().skip(1)).unwrap_or_else(|message| usage(&message));
    let endpoint = ReceiverEndpoint {
        host: arguments.host.clone(),
        port: arguments.port,
    };
    let client = AppCommandHttpClient::new(endpoint, arguments.timeout)?;
    let mut output = io::stdout().lock();
    if !run_probe(&client, &arguments, &mut output)? {
        return Err(io::Error::other(
            "no AppCommand attempt returned a requested command",
        ));
    }
    Ok(())
}

fn run_probe(
    client: &AppCommandHttpClient,
    arguments: &Arguments,
    output: &mut impl Write,
) -> io::Result<bool> {
    let timestamp = epoch_seconds();
    let batch_request = AppCommandRequest::audio_information();
    let batch = run_attempt(
        client,
        arguments,
        timestamp,
        APP_COMMAND_0300_PATH,
        "batch-0300",
        &batch_request,
        output,
    )?;
    let mut any_usable = batch.usable();

    // 0301 is a compatibility observation only. It is tried only after a
    // successfully parsed, wholly empty 0300 batch reply.
    if should_attempt_0301(&batch) {
        let compatibility = run_attempt(
            client,
            arguments,
            timestamp,
            APP_COMMAND_0301_PATH,
            "batch-0301-compatibility",
            &batch_request,
            output,
        )?;
        any_usable |= compatibility.usable();
    }

    // A parsed batch can be incomplete even though it is not empty. Retain
    // its results, then request only absent commands individually at 0300.
    if batch.parsed {
        for command in missing_requested_commands(&batch.returned_requested) {
            let request = single_command_request(command);
            let scope = format!("single-0300:{command}");
            any_usable |= run_attempt(
                client,
                arguments,
                timestamp,
                APP_COMMAND_0300_PATH,
                &scope,
                &request,
                output,
            )?
            .usable();
        }
    }
    Ok(any_usable)
}

fn run_attempt(
    client: &AppCommandHttpClient,
    arguments: &Arguments,
    timestamp: u64,
    path: &str,
    scope: &str,
    request: &AppCommandRequest,
    output: &mut impl Write,
) -> io::Result<AttemptObservation> {
    let endpoint = format!("http://{}:{}{path}", arguments.host, arguments.port);
    let request_xml = request
        .to_xml()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    writeln!(
        output,
        "receiver={} model={} firmware={} timestamp={timestamp} endpoint={} request_scope={scope} request_xml={}",
        one_line(&arguments.host),
        one_line(&arguments.model),
        one_line(&arguments.firmware),
        one_line(&endpoint),
        one_line(&request_xml)
    )?;

    match execute_at(client, path, request) {
        Ok(http) => render_response(arguments, timestamp, &endpoint, scope, http, output),
        Err(error) => {
            writeln!(
                output,
                "receiver={} model={} firmware={} timestamp={timestamp} endpoint={} request_scope={scope} error={}",
                one_line(&arguments.host),
                one_line(&arguments.model),
                one_line(&arguments.firmware),
                one_line(&endpoint),
                one_line(&error.to_string())
            )?;
            Ok(AttemptObservation::default())
        }
    }
}

fn execute_at(
    client: &AppCommandHttpClient,
    path: &str,
    request: &AppCommandRequest,
) -> io::Result<RawHttpResponse> {
    if path == APP_COMMAND_0300_PATH {
        client.execute(request)
    } else {
        let xml = request
            .to_xml()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        client.execute_xml_at(path, &xml)
    }
}

fn render_response(
    arguments: &Arguments,
    timestamp: u64,
    endpoint: &str,
    scope: &str,
    http: RawHttpResponse,
    output: &mut impl Write,
) -> io::Result<AttemptObservation> {
    writeln!(
        output,
        "receiver={} model={} firmware={} timestamp={timestamp} endpoint={} request_scope={scope} status_code={} status={} raw_response={}",
        one_line(&arguments.host),
        one_line(&arguments.model),
        one_line(&arguments.firmware),
        one_line(endpoint),
        http.status_code,
        one_line(&http.status_line),
        one_line(&http.body)
    )?;
    if !(200..300).contains(&http.status_code) {
        writeln!(
            output,
            "endpoint={} request_scope={scope} error=non-success-http-status",
            one_line(endpoint)
        )?;
        return Ok(AttemptObservation::default());
    }
    match parse_app_command_response(&http.body) {
        Ok(response) => {
            render_parsed_response(arguments, timestamp, endpoint, scope, &response, output)
        }
        Err(error) => {
            writeln!(
                output,
                "endpoint={} request_scope={scope} parse_error={}",
                one_line(endpoint),
                one_line(&error.to_string())
            )?;
            Ok(AttemptObservation::default())
        }
    }
}

fn render_parsed_response(
    arguments: &Arguments,
    timestamp: u64,
    endpoint: &str,
    scope: &str,
    response: &AppCommandResponse,
    output: &mut impl Write,
) -> io::Result<AttemptObservation> {
    for (command, parameter) in response.parameters() {
        writeln!(
            output,
            "receiver={} model={} firmware={} timestamp={timestamp} endpoint={} request_scope={scope} command={} field={} attributes={} value={}",
            one_line(&arguments.host),
            one_line(&arguments.model),
            one_line(&arguments.firmware),
            one_line(endpoint),
            one_line(command),
            one_line(&parameter.name),
            render_attributes(&parameter.attributes),
            one_line(&parameter.value)
        )?;
    }
    let returned_requested = requested_commands_in(response);
    writeln!(
        output,
        "endpoint={} request_scope={scope} parsed=true returned_requested={} usable={}",
        one_line(endpoint),
        returned_requested.join(","),
        !returned_requested.is_empty()
    )?;
    Ok(AttemptObservation {
        parsed: true,
        empty_response: response.commands.is_empty(),
        returned_requested,
    })
}

fn requested_commands_in(response: &AppCommandResponse) -> Vec<String> {
    REQUESTED_COMMANDS
        .iter()
        .filter(|name| response.command(name).is_some())
        .map(|name| (*name).to_owned())
        .collect()
}

fn missing_requested_commands(returned: &[String]) -> Vec<&'static str> {
    REQUESTED_COMMANDS
        .iter()
        .copied()
        .filter(|name| !returned.iter().any(|returned| returned == name))
        .collect()
}

fn should_attempt_0301(batch: &AttemptObservation) -> bool {
    batch.parsed && batch.empty_response
}

fn single_command_request(command: &str) -> AppCommandRequest {
    let query = AppCommandRequest::audio_information()
        .queries()
        .iter()
        .find(|query| query.name() == command)
        .expect("requested command is in the fixed batch")
        .clone();
    AppCommandRequest::new(vec![query]).expect("single built-in query is valid")
}

fn render_attributes(attributes: &std::collections::BTreeMap<String, String>) -> String {
    attributes
        .iter()
        .map(|(name, value)| format!("{}:{}", one_line(name), one_line(value)))
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_arguments(args: impl Iterator<Item = String>) -> Result<Arguments, String> {
    let mut args = args;
    let host = args.next().ok_or("HOST is required")?;
    let timeout_ms = args
        .next()
        .ok_or("TIMEOUT-MS is required")?
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or("TIMEOUT-MS must be a positive integer")?;
    let model = args.next().ok_or("MODEL is required")?;
    let firmware = args.next().ok_or("FIRMWARE is required")?;
    validate_metadata("MODEL", &model)?;
    validate_metadata("FIRMWARE", &firmware)?;
    let port = match args.next() {
        Some(value) => value
            .parse::<u16>()
            .map_err(|_| "PORT must be an integer")?,
        None => 8080,
    };
    if port == 0 {
        return Err("PORT must be greater than zero".to_owned());
    }
    if args.next().is_some() {
        return Err("too many arguments".to_owned());
    }
    Ok(Arguments {
        host,
        timeout: Duration::from_millis(timeout_ms),
        model,
        firmware,
        port,
    })
}

fn validate_metadata(label: &str, value: &str) -> Result<(), String> {
    if value.chars().any(char::is_control) {
        return Err(format!("{label} must not contain control characters"));
    }
    Ok(())
}

fn usage(message: &str) -> ! {
    eprintln!(
        "{message}\nusage: x3800h-http-info-probe HOST TIMEOUT-MS MODEL FIRMWARE [PORT]\n       PORT defaults to 8080"
    );
    std::process::exit(2)
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn one_line(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect(),
            '\t' => "\\t".chars().collect(),
            value if value.is_control() => format!("\\u{{{:04x}}}", value as u32).chars().collect(),
            value => vec![value],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn response(xml: &str) -> AppCommandResponse {
        parse_app_command_response(xml).unwrap()
    }

    #[test]
    fn plans_retries_for_only_missing_commands() {
        let partial = response(
            "<rx><cmd><name>GetAudioInfo</name></cmd><cmd><name>GetVideoInfo</name></cmd></rx>",
        );
        assert_eq!(
            requested_commands_in(&partial),
            ["GetVideoInfo", "GetAudioInfo"]
        );
        assert_eq!(
            missing_requested_commands(&requested_commands_in(&partial)),
            ["GetInputSignal", "GetActiveSpeaker", "GetAudyssyInfo"]
        );
    }

    #[test]
    fn empty_and_complete_responses_have_expected_usability() {
        let empty = response("<rx/>");
        let complete = response("<rx><cmd><name>GetInputSignal</name></cmd></rx>");
        assert!(requested_commands_in(&empty).is_empty());
        assert!(AttemptObservation {
            parsed: true,
            empty_response: false,
            returned_requested: requested_commands_in(&complete)
        }
        .usable());
        assert!(!AttemptObservation {
            parsed: true,
            empty_response: true,
            returned_requested: vec![]
        }
        .usable());
    }

    #[test]
    fn compatibility_attempt_requires_a_fully_empty_batch() {
        assert!(should_attempt_0301(&AttemptObservation {
            parsed: true,
            empty_response: true,
            returned_requested: vec![],
        }));
        assert!(!should_attempt_0301(&AttemptObservation {
            parsed: true,
            empty_response: false,
            returned_requested: vec![],
        }));
    }

    #[test]
    fn renders_every_attribute_and_control_value() {
        let mut attributes = std::collections::BTreeMap::new();
        attributes.insert("control".to_owned(), "2".to_owned());
        attributes.insert("firmware-field".to_owned(), "odd".to_owned());
        assert_eq!(
            render_attributes(&attributes),
            "control:2,firmware-field:odd"
        );
        assert_eq!(one_line("a\r\nb\u{0001}"), "a\\r\\nb\\u{0001}");
    }

    #[test]
    fn rejects_zero_port_and_control_metadata() {
        assert!(parse_arguments(
            ["host", "100", "model", "firmware", "0"]
                .into_iter()
                .map(str::to_owned)
        )
        .is_err());
        assert!(parse_arguments(
            ["host", "100", "model\n", "firmware"]
                .into_iter()
                .map(str::to_owned)
        )
        .is_err());
    }

    #[test]
    fn loopback_partial_batch_retries_only_missing_commands() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let mut requests = Vec::new();
            for _ in 0..4 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut bytes = [0_u8; 4096];
                let length = stream.read(&mut bytes).unwrap();
                let request = String::from_utf8_lossy(&bytes[..length]).into_owned();
                requests.push(request.clone());
                let body = if requests.len() == 1 {
                    "<rx><cmd><name>GetAudioInfo</name><param name=\"signal\" control=\"9\" extra=\"kept\">PCM</param></cmd><cmd><name>GetVideoInfo</name></cmd></rx>"
                } else {
                    "<rx><cmd><name>GetInputSignal</name><param name=\"x\" control=\"7\">value</param></cmd></rx>"
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
            requests
        });
        let arguments = Arguments {
            host: "127.0.0.1".to_owned(),
            timeout: Duration::from_secs(1),
            model: "AVR-X3800H".to_owned(),
            firmware: "test".to_owned(),
            port,
        };
        let client = AppCommandHttpClient::new(
            ReceiverEndpoint {
                host: arguments.host.clone(),
                port,
            },
            arguments.timeout,
        )
        .unwrap();
        let mut output = Vec::new();
        assert!(run_probe(&client, &arguments, &mut output).unwrap());
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 4);
        assert!(requests[0].contains("GetAudioInfo") && requests[0].contains("GetAudyssyInfo"));
        for command in ["GetInputSignal", "GetActiveSpeaker", "GetAudyssyInfo"] {
            assert_eq!(
                requests
                    .iter()
                    .skip(1)
                    .filter(|request| request.contains(command))
                    .count(),
                1
            );
        }
        assert!(!requests
            .iter()
            .skip(1)
            .any(|request| request.contains("GetVideoInfo") || request.contains("GetAudioInfo")));
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("attributes=control:9,extra:kept"));
    }
}
