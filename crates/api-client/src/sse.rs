//! Reading an event stream, and the two port values built on it.

use crate::mapping::{unreadable, UNAVAILABLE};
use crate::transport::Abort;
use crate::Audience;
use denon_avr_api_contract::events::{EndPayload, EndReason, Event, EventName, Parser};
use denon_avr_api_contract::{AgentStateView, OperationDto, OperatorStateView};
use denon_avr_application::ports::BoxFuture;
use denon_avr_application::{
    ControlError, OperationEvent, OperationEventSource, OperationSnapshot,
};
use denon_avr_domain::ReceiverState;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use std::collections::VecDeque;

/// Why the stream could not be read any further.
#[derive(Debug)]
pub(crate) enum StreamFault {
    /// The connection broke.
    Broken,
    /// The server wrote something the contract does not allow.
    Malformed,
}

/// A stream's events as they arrive. Dropping it closes the connection.
pub(crate) struct EventReader {
    body: Incoming,
    parser: Parser,
    queue: VecDeque<Event>,
    finished: bool,
    _connection: Abort,
}

impl EventReader {
    pub(crate) fn new(body: Incoming, connection: Abort) -> Self {
        Self {
            body,
            parser: Parser::new(),
            queue: VecDeque::new(),
            finished: false,
            _connection: connection,
        }
    }

    /// The next event, or `None` once the stream has ended. Cancelling it loses
    /// nothing: what was read is kept for the next call.
    pub(crate) async fn next(&mut self) -> Result<Option<Event>, StreamFault> {
        loop {
            if let Some(event) = self.queue.pop_front() {
                return Ok(Some(event));
            }
            if self.finished {
                return Ok(None);
            }
            match self.body.frame().await {
                None => self.finished = true,
                Some(Err(_)) => return Err(StreamFault::Broken),
                Some(Ok(frame)) => {
                    if let Ok(data) = frame.into_data() {
                        let events = self
                            .parser
                            .push(&data)
                            .map_err(|_| StreamFault::Malformed)?;
                        self.queue.extend(events);
                    }
                }
            }
        }
    }
}

/// A state event's data as the state it describes, by the view this client
/// was given.
pub(crate) fn decode_state(
    audience: Audience,
    event: &Event,
) -> Result<ReceiverState, ControlError> {
    match audience {
        Audience::Operator => event
            .decode::<OperatorStateView>()
            .map_err(unreadable)?
            .into_state()
            .map_err(unreadable),
        Audience::Agent => event
            .decode::<AgentStateView>()
            .map_err(unreadable)?
            .into_state()
            .map_err(unreadable),
    }
}

/// The port's reading of a stream that ended for `reason`.
pub(crate) fn ended(event: &Event) -> ControlError {
    match event.decode::<EndPayload>().map(|end| end.reason) {
        Ok(EndReason::Revoked) => ControlError::Forbidden,
        _ => ControlError::Unavailable(UNAVAILABLE.into()),
    }
}

/// The caller's operation events, read from the stream that carries them.
pub(crate) struct RemoteOperationEvents {
    pub(crate) reader: EventReader,
}

impl OperationEventSource for RemoteOperationEvents {
    fn next(&mut self) -> BoxFuture<'_, Option<OperationEvent>> {
        Box::pin(async move {
            loop {
                let event = match self.reader.next().await {
                    Ok(Some(event)) => event,
                    // The stream ended, or broke: the port reports both as an end.
                    Ok(None) | Err(_) => return None,
                };
                match event.name {
                    EventName::Operation => {
                        // An event that cannot be read ends the stream; a state of
                        // an operation is never guessed.
                        return event
                            .decode::<OperationDto>()
                            .ok()
                            .and_then(|dto| OperationSnapshot::try_from(dto).ok())
                            .map(OperationEvent::Update);
                    }
                    EventName::Missed => return Some(OperationEvent::Missed),
                    EventName::End => return None,
                    EventName::State => {}
                }
            }
        })
    }
}
