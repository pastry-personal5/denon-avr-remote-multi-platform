//! The two event streams: names, payloads, and the parser a client reads them
//! with.
//!
//! The server writes the streams with its web framework's server-sent-events
//! response, so this crate holds the vocabulary and the reader, not a writer. A
//! stream is a sequence of events separated by blank lines; a line starting with a
//! colon is a comment (the server's keep-alive), and an event whose name this
//! reader does not know is skipped, so the stream can grow.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// The longest event, or line, a reader accepts.
pub const MAX_EVENT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventName {
    /// A complete receiver state, on the state stream.
    State,
    /// One of the caller's operations, on the operation stream.
    Operation,
    /// The reader fell behind and missed operation events.
    Missed,
    /// The stream is closing, and why.
    End,
}

impl EventName {
    pub const ALL: [Self; 4] = [Self::State, Self::Operation, Self::Missed, Self::End];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::State => "state",
            Self::Operation => "operation",
            Self::Missed => "missed",
            Self::End => "end",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|event| event.as_str() == name)
    }
}

/// Why a stream is closing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    /// The service closed the session, so the state subscription ended. A client
    /// that subscribes again reaches the receiver afresh.
    SessionClosed,
    /// The token the stream was opened with was revoked.
    Revoked,
    /// The server is shutting down.
    Shutdown,
    /// An agent's state stream reached its maximum age. The client subscribes
    /// again.
    MaxAge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndPayload {
    pub reason: EndReason,
}

/// One event of a known name, with its data as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub name: EventName,
    pub data: String,
}

impl Event {
    /// The data as a value of the payload type.
    pub fn decode<T: DeserializeOwned>(&self) -> Result<T, ParseError> {
        serde_json::from_str(&self.data).map_err(|why| ParseError::Payload(why.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// An event, or a line, longer than [`MAX_EVENT_BYTES`].
    TooLarge,
    /// A line that is not UTF-8.
    NotText,
    /// Data that is not the payload its event names.
    Payload(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge => f.write_str("an event is larger than the reader accepts"),
            Self::NotText => f.write_str("an event line is not text"),
            Self::Payload(why) => write!(f, "an event's data could not be read: {why}"),
        }
    }
}

impl std::error::Error for ParseError {}

/// Reads a stream a chunk at a time. Chunks may split an event, a line, or a
/// `\r\n` pair anywhere.
#[derive(Debug, Default)]
pub struct Parser {
    /// Bytes of the line being read.
    line: Vec<u8>,
    /// Whether the last byte seen was a `\r`, so a following `\n` is part of it.
    after_cr: bool,
    name: Option<String>,
    data: Vec<String>,
    data_bytes: usize,
}

impl Parser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed the next bytes. Returns the events they completed, in order.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Event>, ParseError> {
        let mut events = Vec::new();
        for &byte in chunk {
            match byte {
                b'\n' if self.after_cr => self.after_cr = false,
                b'\n' | b'\r' => {
                    self.after_cr = byte == b'\r';
                    if let Some(event) = self.end_line()? {
                        events.push(event);
                    }
                }
                _ => {
                    self.after_cr = false;
                    if self.line.len() >= MAX_EVENT_BYTES {
                        return Err(ParseError::TooLarge);
                    }
                    self.line.push(byte);
                }
            }
        }
        Ok(events)
    }

    fn end_line(&mut self) -> Result<Option<Event>, ParseError> {
        let bytes = std::mem::take(&mut self.line);
        let line = String::from_utf8(bytes).map_err(|_| ParseError::NotText)?;
        if line.is_empty() {
            return Ok(self.dispatch());
        }
        if line.starts_with(':') {
            return Ok(None);
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line.as_str(), ""),
        };
        match field {
            "event" => self.name = Some(value.to_owned()),
            "data" => {
                self.data_bytes += value.len() + 1;
                if self.data_bytes > MAX_EVENT_BYTES {
                    return Err(ParseError::TooLarge);
                }
                self.data.push(value.to_owned());
            }
            // `id`, `retry`, and anything new: this reader keeps none of it.
            _ => {}
        }
        Ok(None)
    }

    fn dispatch(&mut self) -> Option<Event> {
        let name = self.name.take();
        let data = std::mem::take(&mut self.data);
        self.data_bytes = 0;
        if data.is_empty() {
            return None;
        }
        // An event with no name is a message, which this stream never sends.
        let name = EventName::parse(name.as_deref()?)?;
        Some(Event {
            name,
            data: data.join("\n"),
        })
    }
}
