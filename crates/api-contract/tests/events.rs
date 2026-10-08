//! The event vocabulary and the parser a client reads a stream with.

use denon_avr_api_contract::events::{
    EndPayload, EndReason, Event, EventName, ParseError, Parser, MAX_EVENT_BYTES,
};

const STREAM: &str = concat!(
    ": connected\n",
    "\n",
    "event: state\n",
    "data: {\"receiver\":\"living-room\"}\n",
    "\n",
    ": keep-alive\r\n",
    "\r\n",
    "event: operation\r\n",
    "data: {\"id\":1,\r\n",
    "data: \"status\":\"completed\"}\r\n",
    "\r\n",
    "event: from_the_future\n",
    "data: {\"x\":1}\n",
    "\n",
    "data: a message with no name\n",
    "\n",
    "id: 7\n",
    "retry: 100\n",
    "event: missed\n",
    "data:{}\n",
    "\n",
    "event: end\r",
    "data: {\"reason\":\"max_age\"}\r",
    "\r",
);

fn expected() -> Vec<(EventName, String)> {
    vec![
        (EventName::State, "{\"receiver\":\"living-room\"}".into()),
        (
            EventName::Operation,
            "{\"id\":1,\n\"status\":\"completed\"}".into(),
        ),
        (EventName::Missed, "{}".into()),
        (EventName::End, "{\"reason\":\"max_age\"}".into()),
    ]
}

fn events_of(chunks: &[&[u8]]) -> Vec<(EventName, String)> {
    let mut parser = Parser::new();
    let mut events = Vec::new();
    for chunk in chunks {
        events.extend(parser.push(chunk).unwrap());
    }
    events.into_iter().map(|e| (e.name, e.data)).collect()
}

#[test]
fn the_parser_reads_a_hand_written_frame_for_every_event() {
    assert_eq!(events_of(&[STREAM.as_bytes()]), expected());

    let end = |reason: &str| {
        let frame = format!("event: end\ndata: {{\"reason\":\"{reason}\"}}\n\n");
        let events = Parser::new().push(frame.as_bytes()).unwrap();
        events[0].decode::<EndPayload>().unwrap().reason
    };
    assert_eq!(end("session_closed"), EndReason::SessionClosed);
    assert_eq!(end("revoked"), EndReason::Revoked);
    assert_eq!(end("shutdown"), EndReason::Shutdown);
    assert_eq!(end("max_age"), EndReason::MaxAge);
    let event = Event {
        name: EventName::End,
        data: "{\"reason\":\"from_the_future\"}".into(),
    };
    assert!(matches!(
        event.decode::<EndPayload>(),
        Err(ParseError::Payload(_))
    ));

    for name in EventName::ALL {
        assert_eq!(EventName::parse(name.as_str()), Some(name));
    }
    assert_eq!(EventName::parse("State"), None);
}

#[test]
fn the_sse_parser_reassembles_events_split_at_every_byte() {
    let bytes = STREAM.as_bytes();
    // One byte at a time.
    let singles: Vec<&[u8]> = bytes.chunks(1).collect();
    assert_eq!(events_of(&singles), expected());
    // Split once at every offset, including between a CR and its LF.
    for at in 0..=bytes.len() {
        let (a, b) = bytes.split_at(at);
        assert_eq!(events_of(&[a, b]), expected(), "split at {at}");
    }
    // Two splits at every pair of offsets that matter: each line end.
    let ends: Vec<usize> = bytes
        .iter()
        .enumerate()
        .filter(|(_, b)| **b == b'\n' || **b == b'\r')
        .map(|(i, _)| i)
        .collect();
    for &first in &ends {
        for &second in &ends {
            if second > first {
                let parts = [&bytes[..first], &bytes[first..second], &bytes[second..]];
                assert_eq!(events_of(&parts), expected(), "{first},{second}");
            }
        }
    }
}

#[test]
fn an_event_over_a_megabyte_is_refused() {
    let mut parser = Parser::new();
    parser.push(b"event: state\n").unwrap();
    // One enormous line.
    let line = vec![b'x'; MAX_EVENT_BYTES + 1];
    let mut huge = b"data: ".to_vec();
    huge.extend_from_slice(&line);
    assert_eq!(parser.push(&huge), Err(ParseError::TooLarge));

    // Many lines that together exceed it.
    let mut parser = Parser::new();
    parser.push(b"event: state\n").unwrap();
    let chunk = format!("data: {}\n", "y".repeat(60_000));
    let mut refused = false;
    for _ in 0..40 {
        if parser.push(chunk.as_bytes()).is_err() {
            refused = true;
            break;
        }
    }
    assert!(refused, "forty lines of 60 KB were accepted");

    // A line that is not text.
    assert_eq!(
        Parser::new().push(b"data: \xff\xfe\n\n"),
        Err(ParseError::NotText)
    );
}

#[test]
fn a_comment_or_an_unknown_event_leaves_the_parser_ready_for_the_next() {
    let mut parser = Parser::new();
    assert!(parser.push(b": hello\n\n: again\n\n").unwrap().is_empty());
    assert!(parser.push(b"event: nope\ndata: 1\n\n").unwrap().is_empty());
    let events = parser.push(b"event: missed\ndata: {}\n\n").unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].name, EventName::Missed);
}
