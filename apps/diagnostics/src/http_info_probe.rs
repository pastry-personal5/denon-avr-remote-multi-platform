//! Diagnostic-only HTTP information probe.
//!
//! Protocol construction/parsing and socket I/O live in the canonical library
//! layers. This binary is limited to argument parsing and presentation.
use denon_avr_domain::ReceiverEndpoint;
use denon_avr_infrastructure::AppCommandHttpClient;
use std::env;
use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn main() -> io::Result<()> {
    let (host, timeout, model, firmware, port) = arguments();
    let endpoint = ReceiverEndpoint {
        host: host.clone(),
        port,
    };
    let client = AppCommandHttpClient::new(endpoint, timeout)?;
    let exchange = client.get_audio_information()?;
    let response = &exchange.http;
    let parsed = &exchange.response;
    let timestamp = epoch_seconds();
    println!(
        "receiver={host} model={model} firmware={firmware} timestamp={timestamp} endpoint=http://{host}:{port}/goform/AppCommand0300.xml status_code={} status={}",
        response.status_code, response.status_line
    );
    println!("raw_response={}", one_line(&response.body));
    for (command, parameter) in parsed.parameters() {
        println!(
            "receiver={host} model={model} firmware={firmware} timestamp={timestamp} command={command} field={} control={} value={}",
            parameter.name,
            parameter.control().unwrap_or(""),
            parameter.trimmed_value()
        );
    }
    print_control_summary(
        &host,
        &model,
        &firmware,
        timestamp,
        parsed,
        "GetInputSignal",
    );
    print_control_summary(
        &host,
        &model,
        &firmware,
        timestamp,
        parsed,
        "GetActiveSpeaker",
    );
    Ok(())
}

fn print_control_summary(
    host: &str,
    model: &str,
    firmware: &str,
    timestamp: u64,
    response: &denon_avr_protocol::AppCommandResponse,
    command_name: &str,
) {
    let Some(command) = response.command(command_name) else {
        return;
    };
    let values = command
        .parameters_with_control(2)
        .map(|parameter| parameter.trimmed_value())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "receiver={host} model={model} firmware={firmware} timestamp={timestamp} command={command_name} control=2 selected={values}"
    );
}

fn arguments() -> (String, Duration, String, String, u16) {
    let mut args = env::args().skip(1);
    let host = args.next().unwrap_or_else(|| usage("HOST is required"));
    let timeout_ms = args
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| usage("TIMEOUT-MS must be a positive integer"));
    let model = args.next().unwrap_or_else(|| usage("MODEL is required"));
    let firmware = args.next().unwrap_or_else(|| usage("FIRMWARE is required"));
    let port = args
        .next()
        .map(|value| {
            value
                .parse::<u16>()
                .unwrap_or_else(|_| usage("PORT must be an integer"))
        })
        .unwrap_or(8080);
    if args.next().is_some() {
        usage("too many arguments");
    }
    (
        host,
        Duration::from_millis(timeout_ms),
        model,
        firmware,
        port,
    )
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
        .replace('\r', "\\r")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_control_values_without_interpreting_them() {
        assert_eq!(one_line("a\r\nb"), "a\\r\\nb");
    }
}
