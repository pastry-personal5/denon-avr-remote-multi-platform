//! Bounded read-only X3800H AppCommand information reader.

use crate::AppCommandHttpClient;
use denon_avr_domain::{HttpInformationSnapshot, ReceiverEndpoint};
use denon_avr_protocol::app_command::{AppCommandQuery, AppCommandRequest, APP_COMMAND_0301_PATH};
use denon_avr_protocol::http_information::{missing_information_commands, parse_http_information};
use std::io;
use std::time::Duration;

pub const X3800H_HTTP_PORT: u16 = 8080;

pub struct HttpInformationHttpClient {
    client: AppCommandHttpClient,
}
impl HttpInformationHttpClient {
    pub fn new(host: impl Into<String>, timeout: Duration) -> io::Result<Self> {
        Ok(Self {
            client: AppCommandHttpClient::new(
                ReceiverEndpoint {
                    host: host.into(),
                    port: X3800H_HTTP_PORT,
                },
                timeout,
            )?,
        })
    }
    pub fn read(&self, generation: u64) -> io::Result<HttpInformationSnapshot> {
        let batch = AppCommandRequest::audio_information();
        let mut response = self.client.execute_and_parse(&batch)?.response;
        // Firmware that returns an entirely empty 0300 batch is compatible with
        // 0301. Never use 0301 for a partial result: retain usable facts first.
        if response.commands.is_empty() {
            if let Ok(exchange) = self
                .client
                .execute_and_parse_at(APP_COMMAND_0301_PATH, &batch)
            {
                response = exchange.response;
            }
        }
        for command in missing_information_commands(&response) {
            if let Ok(exchange) = self.client.execute_and_parse(&single(command)) {
                response.commands.extend(exchange.response.commands);
            }
        }
        if response.commands.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "receiver returned no HTTP information commands",
            ));
        }
        Ok(parse_http_information(&response, generation))
    }
}
fn single(command: &str) -> AppCommandRequest {
    match command {
        "GetInputSignal" => AppCommandRequest::input_signal(),
        "GetActiveSpeaker" => AppCommandRequest::active_speaker(),
        "GetVideoInfo" => AppCommandRequest::video_info(),
        "GetAudioInfo" => AppCommandRequest::audio_info(),
        "GetAudyssyInfo" => AppCommandRequest::audyssey_info(),
        _ => AppCommandRequest::new(vec![
            AppCommandQuery::new(command, ["unused"]).expect("known read-only command")
        ])
        .expect("one query"),
    }
}
