//! The event streams and the waits, on both endpoints: what they carry, who sees
//! what, and every way one ends. A stream holds a lease that keeps a receiver
//! connected, so most of these tests are about that lease being given back.

mod support;

use denon_avr_api_contract::events::{EventName, Parser};
use denon_avr_api_contract::paths::request as path;
use denon_avr_api_server::Limits;
use denon_avr_application::{OperatorAdmin, Principal, ServiceConfig};
use denon_avr_domain::FieldIssue;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use support::*;

const ADDRESS: &str = "connecting to 192.168.1.20:23";

fn options(change: impl FnOnce(&mut Limits)) -> Options {
    let mut options = Options::default();
    change(&mut options.limits);
    options
}

/// Each endpoint with a token that it accepts.
async fn endpoints(fixture: &Fixture) -> Vec<(&'static str, PathBuf, String)> {
    vec![
        (
            "operator",
            fixture.operator_socket().to_owned(),
            fixture.operator_token(),
        ),
        (
            "agent",
            fixture.agent_socket().to_owned(),
            fixture.agent_token("openclaw").await,
        ),
    ]
}

async fn open(socket: &Path, path: &str, token: &str) -> Stream {
    match Stream::open(socket, path, token).await {
        Ok(stream) => stream,
        Err(reply) => panic!("the stream was refused: {} {}", reply.status, reply.text()),
    }
}

async fn submit_mute(socket: &Path, token: &str) -> Value {
    let body = serde_json::to_vec(&json!({ "intent": { "kind": "mute", "value": "on" } })).unwrap();
    let reply = request(
        socket,
        "POST",
        &path::submit(&living_room()),
        Some(token),
        Some(&body),
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    reply.json()
}

/// The ids of the operations whose events arrive until the stream is quiet.
async fn operation_ids(stream: &mut Stream) -> BTreeSet<u64> {
    let mut ids = BTreeSet::new();
    while let Next::Event(event) = stream.next(Duration::from_millis(400)).await {
        if event.name == EventName::Operation {
            ids.insert(event.decode::<Value>().unwrap()["id"].as_u64().unwrap());
        }
    }
    ids
}

async fn released(fixture: &Fixture) {
    wait_for("the receiver to be released", || {
        fixture.connector.open_sessions() == 0
    })
    .await;
}

/// Revoke the Agent token issued under `label`, as the Operator does.
async fn revoke(fixture: &Fixture, label: &str) {
    let operator = fixture.operator_token();
    let socket = fixture.operator_socket();
    let list = get(socket, &path::tokens(), &operator).await.json();
    let id = list["tokens"]
        .as_array()
        .unwrap()
        .iter()
        .find(|token| token["label"] == label && token["revoked"].is_null())
        .expect("an active token")["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let reply = request(socket, "DELETE", &path::token(&id), Some(&operator), None).await;
    assert_eq!(reply.status, 200, "{}", reply.text());
}

#[tokio::test]
async fn the_first_event_is_the_full_state_then_one_per_change() {
    let fixture = Fixture::start(Options::default()).await;
    let room = living_room();
    for (name, socket, token) in endpoints(&fixture).await {
        let mut stream = open(&socket, &path::state_events(&room), &token).await;
        assert_eq!(stream.header("content-type"), Some("text/event-stream"));
        assert_eq!(stream.header("cache-control"), Some("no-cache"));

        let first = stream.event_named(EventName::State).await;
        let snapshot = get(&socket, &path::state(&room), &token).await;
        assert_eq!(first.decode::<Value>().unwrap(), snapshot.json(), "{name}");

        fixture.connector.latest().turn_volume(-30.0);
        let second = stream.event_named(EventName::State).await;
        assert_ne!(second.data, first.data, "{name}: the volume changed");
        let snapshot = get(&socket, &path::state(&room), &token).await;
        assert_eq!(second.decode::<Value>().unwrap(), snapshot.json(), "{name}");

        assert!(
            stream.stays_quiet(Duration::from_millis(300)).await,
            "{name}: an event with no change"
        );
        fixture.connector.latest().turn_volume(-35.0);
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn operation_events_reach_only_their_owner() {
    let fixture = Fixture::start(Options::default()).await;
    let operator = fixture.operator_token();
    let alice = fixture.agent_token("alice").await;
    let bob = fixture.agent_token("bob").await;
    let operator_socket = fixture.operator_socket().to_owned();
    let agent_socket = fixture.agent_socket().to_owned();

    let mut operators = open(&operator_socket, &path::operation_events(), &operator).await;
    let mut alices = open(&agent_socket, &path::operation_events(), &alice).await;
    let mut bobs = open(&agent_socket, &path::operation_events(), &bob).await;

    let hers = submit_mute(&agent_socket, &alice).await["id"]
        .as_u64()
        .unwrap();
    let theirs = submit_mute(&operator_socket, &operator).await["id"]
        .as_u64()
        .unwrap();
    assert_ne!(hers, theirs);

    assert_eq!(
        operation_ids(&mut operators).await,
        BTreeSet::from([hers, theirs]),
        "the Operator sees every operation"
    );
    assert_eq!(
        operation_ids(&mut alices).await,
        BTreeSet::from([hers]),
        "an agent sees its own"
    );
    assert!(
        operation_ids(&mut bobs).await.is_empty(),
        "an agent never sees another's"
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn an_operation_stream_holds_no_lease_on_a_receiver() {
    let fixture = Fixture::start(Options::default()).await;
    let mut streams = Vec::new();
    for (_, socket, token) in endpoints(&fixture).await {
        streams.push(open(&socket, &path::operation_events(), &token).await);
    }
    // Longer than the receiver's idle time.
    for stream in &mut streams {
        assert!(stream.stays_quiet(Duration::from_millis(500)).await);
    }
    assert_eq!(
        fixture
            .connector
            .opens
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(fixture.connector.open_sessions(), 0);
    fixture.shutdown().await;
}

#[tokio::test]
async fn no_address_reaches_an_agent_in_a_state_stream() {
    let fixture = Fixture::start(Options::default()).await;
    let room = living_room();
    let mut seen = Vec::new();
    for (name, socket, token) in endpoints(&fixture).await {
        let mut stream = open(&socket, &path::state_events(&room), &token).await;
        stream.event_named(EventName::State).await;
        // The receiver's own text, as a failed query leaves it.
        fixture.connector.latest().states.send_modify(|state| {
            state.main_zone.volume.last_issue = Some(FieldIssue {
                message: ADDRESS.into(),
            });
            state
                .diagnostics
                .push(format!("malformed: MVMAX {ADDRESS}"));
            state.revision.0 += 1;
        });
        stream.event_named(EventName::State).await;
        seen.push((name, String::from_utf8_lossy(&stream.body).into_owned()));
    }
    let operator = &seen[0].1;
    let agent = &seen[1].1;
    // The Operator is told, so the check on the agent's stream can fail.
    assert!(operator.contains("192.168.1.20"), "{operator}");
    for needle in ["192.168", "MVMAX", "malformed", "connecting"] {
        assert!(
            !agent.contains(needle),
            "an agent was told {needle:?}: {agent}"
        );
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_slow_client_sees_the_newest_state_and_never_a_backlog() {
    let fixture = Fixture::start(Options::default()).await;
    let room = living_room();
    for (name, socket, token) in endpoints(&fixture).await {
        let mut stream = open(&socket, &path::state_events(&room), &token).await;
        stream.event_named(EventName::State).await;

        // The client reads nothing while the receiver changes two hundred times.
        let session = fixture.connector.latest();
        for step in 0..200 {
            session.turn_volume(-60.0 + f64::from(step % 40) * 0.5);
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        session.turn_volume(-25.0);

        let mut events = Vec::new();
        while let Next::Event(event) = stream.next(Duration::from_millis(400)).await {
            events.push(event);
        }
        let newest = get(&socket, &path::state(&room), &token).await.json();
        let last = events.last().expect("events arrived");
        assert_eq!(
            last.decode::<Value>().unwrap(),
            newest,
            "{name}: the last event is the newest state"
        );
        assert!(
            events.len() < 100,
            "{name}: {} events for 201 changes is a backlog",
            events.len()
        );
        session.turn_volume(-35.0);
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_missed_operation_event_is_reported_and_the_operation_can_be_read() {
    let fixture = Fixture::start(options(|limits| {
        limits.send_timeout = Duration::from_secs(60);
    }))
    .await;
    let operator = fixture.operator_token();
    let socket = fixture.operator_socket().to_owned();
    let mut stream = open(&socket, &path::operation_events(), &operator).await;

    // The client reads nothing while more operations finish than the service keeps
    // events for. An operation identical to one still running is taken for a retry
    // of it, so each round of distinct intents is allowed to finish before the next.
    let mut last = 0;
    for _round in 0..4 {
        for step in 0..100 {
            let body = serde_json::to_vec(&json!({
                "intent": { "kind": "volume", "level": { "half_steps": -150 + step } }
            }))
            .unwrap();
            let reply = request(
                &socket,
                "POST",
                &path::submit(&living_room()),
                Some(&operator),
                Some(&body),
            )
            .await;
            assert_eq!(reply.status, 200, "{}", reply.text());
            last = reply.json()["id"].as_u64().unwrap();
        }
        let finished = get(&socket, &path::operation(last, Some(10_000)), &operator).await;
        assert_eq!(
            finished.json()["status"],
            "completed",
            "{}",
            finished.text()
        );
    }
    assert!(last >= 300, "only {last} operations were made");

    let mut missed = false;
    while let Next::Event(event) = stream.next(Duration::from_millis(500)).await {
        if event.name == EventName::Missed {
            assert_eq!(event.data, "{}");
            missed = true;
        }
    }
    assert!(missed, "a reader that fell behind is told so");
    let read = get(&socket, &path::operation(last, None), &operator).await;
    assert_eq!(read.status, 200, "{}", read.text());
    assert_eq!(read.json()["id"].as_u64(), Some(last));
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_keep_alive_is_written_every_fifteen_seconds() {
    // The defaults are what the design says; the next test shortens the interval to
    // watch it work without waiting a quarter of a minute.
    let limits = Limits::default();
    assert_eq!(limits.keep_alive, Duration::from_secs(15));
    assert_eq!(limits.send_timeout, Duration::from_secs(10));
    assert_eq!(limits.agent_stream_max_age, Duration::from_secs(600));
    assert_eq!(limits.streams_per_principal, 4);
}

#[tokio::test]
async fn a_quiet_stream_is_written_to_at_the_keep_alive_interval() {
    let fixture = Fixture::start(options(|limits| {
        limits.keep_alive = Duration::from_millis(300);
    }))
    .await;
    let operator = fixture.operator_token();
    let mut stream = open(
        fixture.operator_socket(),
        &path::operation_events(),
        &operator,
    )
    .await;
    assert!(stream.stays_quiet(Duration::from_millis(1_300)).await);
    assert!(
        stream.comments() >= 3,
        "{} keep-alive lines in {:?}",
        stream.comments(),
        String::from_utf8_lossy(&stream.body)
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_stalled_client_is_dropped_after_ten_seconds_and_the_receiver_is_released() {
    // Ten seconds is the default; this test uses a third of a second of it.
    let fixture = Fixture::start(options(|limits| {
        limits.send_timeout = Duration::from_millis(300);
    }))
    .await;
    let room = living_room();
    let operator = fixture.operator_token();
    let stream = open(
        fixture.operator_socket(),
        &path::state_events(&room),
        &operator,
    )
    .await;
    assert_eq!(fixture.connector.open_sessions(), 1);

    // The client never reads. With the receiver changing, the socket's buffer
    // fills, an event waits to be taken, and the wait times out.
    let session = fixture.connector.latest();
    for step in 0..300 {
        session.turn_volume(-60.0 + f64::from(step % 40) * 0.5);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    released(&fixture).await;
    drop(stream);
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_stalled_client_on_a_quiet_receiver_is_released_when_the_stream_reaches_its_maximum_age()
{
    // The send timeout is left at its ten seconds: no event is produced, so it
    // could never fire, and the release below comes before it would.
    let fixture = Fixture::start(options(|limits| {
        limits.agent_stream_max_age = Duration::from_millis(600);
    }))
    .await;
    let room = living_room();
    let agent = fixture.agent_token("openclaw").await;
    let stalled = open(fixture.agent_socket(), &path::state_events(&room), &agent).await;
    assert_eq!(fixture.connector.open_sessions(), 1);

    let started = Instant::now();
    released(&fixture).await;
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "released after {:?}",
        started.elapsed()
    );
    drop(stalled);

    // A client that does read is told why, and that it should subscribe again.
    let mut reading = open(fixture.agent_socket(), &path::state_events(&room), &agent).await;
    reading.event_named(EventName::State).await;
    let end = reading.last_event().await.expect("an end event");
    assert_eq!(end_reason(&end), "max_age");
    fixture.shutdown().await;
}

#[tokio::test]
async fn an_operator_state_stream_has_no_maximum_age() {
    let fixture = Fixture::start(options(|limits| {
        limits.agent_stream_max_age = Duration::from_millis(300);
    }))
    .await;
    let room = living_room();
    let operator = fixture.operator_token();
    let mut stream = open(
        fixture.operator_socket(),
        &path::state_events(&room),
        &operator,
    )
    .await;
    stream.event_named(EventName::State).await;
    assert!(stream.stays_quiet(Duration::from_millis(1_200)).await);
    fixture.connector.latest().turn_volume(-30.0);
    stream.event_named(EventName::State).await;
    fixture.shutdown().await;
}

#[tokio::test]
async fn the_parser_reads_what_the_server_writes() {
    let fixture = Fixture::start(Options::default()).await;
    let room = living_room();
    let agent = fixture.agent_token("openclaw").await;
    let socket = fixture.agent_socket().to_owned();

    let mut states = open(&socket, &path::state_events(&room), &agent).await;
    let mut operations = open(&socket, &path::operation_events(), &agent).await;
    states.event_named(EventName::State).await;
    fixture.connector.latest().turn_volume(-30.0);
    states.event_named(EventName::State).await;
    submit_mute(&socket, &agent).await;
    let mut names = vec![];
    while let Next::Event(event) = operations.next(Duration::from_millis(400)).await {
        names.push(event.name);
    }
    assert!(names.iter().all(|name| *name == EventName::Operation));
    assert!(!names.is_empty());
    revoke(&fixture, "openclaw").await;
    let ended = states.last_event().await.expect("an end event");
    assert_eq!(end_reason(&ended), "revoked");
    let ended = operations.last_event().await.expect("an end event");
    assert_eq!(end_reason(&ended), "revoked");

    // The bytes of each stream, as the server wrote them, one byte at a time.
    let read = |body: &[u8]| {
        let mut parser = Parser::new();
        let mut events = Vec::new();
        for byte in body {
            events.extend(parser.push(&[*byte]).expect("the bytes parse"));
        }
        for event in &events {
            assert!(event.decode::<Value>().is_ok(), "{}", event.data);
        }
        events.iter().map(|event| event.name).collect::<Vec<_>>()
    };

    // The state, the volume's change, the mute's change when it arrived in time,
    // and the end.
    let from_states = read(&states.body);
    assert_eq!(from_states.last(), Some(&EventName::End));
    assert!(from_states.len() >= 3, "{from_states:?}");
    assert!(from_states[..from_states.len() - 1]
        .iter()
        .all(|name| *name == EventName::State));

    let mut expected = names.clone();
    expected.push(EventName::End);
    assert_eq!(read(&operations.body), expected);
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_client_that_disconnects_releases_its_lease_at_once() {
    let fixture = Fixture::start(Options::default()).await;
    let room = living_room();
    for (name, socket, token) in endpoints(&fixture).await {
        let mut stream = open(&socket, &path::state_events(&room), &token).await;
        stream.event_named(EventName::State).await;
        assert_eq!(fixture.connector.open_sessions(), 1, "{name}");

        let started = Instant::now();
        drop(stream);
        released(&fixture).await;
        // The receiver's idle time is 200 ms; the keep-alive is 15 s, so a server
        // that waited for a write to fail would be far over this.
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{name}: released after {:?}",
            started.elapsed()
        );
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn the_fifth_stream_of_one_principal_is_429() {
    let fixture = Fixture::start(Options::default()).await;
    let room = living_room();
    let operator = fixture.operator_token();
    let agent = fixture.agent_token("openclaw").await;
    let socket = fixture.operator_socket().to_owned();

    // Three operation streams and a state stream: four of either kind.
    let mut held = Vec::new();
    for _ in 0..3 {
        held.push(open(&socket, &path::operation_events(), &operator).await);
    }
    held.push(open(&socket, &path::state_events(&room), &operator).await);
    assert_eq!(
        fixture
            .connector
            .opens
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );

    for refused in [path::operation_events(), path::state_events(&room)] {
        let reply = Stream::open(&socket, &refused, &operator)
            .await
            .err()
            .expect("the fifth stream is refused");
        assert_eq!(reply.status, 429, "{}", reply.text());
        assert_eq!(reply.code().as_deref(), Some("too_many_streams"));
    }
    // A refused state stream opened no connection to the receiver.
    assert_eq!(
        fixture
            .connector
            .opens
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );

    // The cap is the principal's: an agent has its own four.
    let agent_stream = open(fixture.agent_socket(), &path::operation_events(), &agent).await;

    // Closing one gives its place back.
    drop(held.pop());
    let mut again = None;
    for _ in 0..100 {
        match Stream::open(&socket, &path::operation_events(), &operator).await {
            Ok(stream) => {
                again = Some(stream);
                break;
            }
            Err(reply) => assert_eq!(reply.status, 429),
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        again.is_some(),
        "a closed stream's place was never given back"
    );
    drop((held, agent_stream));
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_session_closed_by_the_service_ends_the_stream_and_a_new_stream_connects_again() {
    let fixture = Fixture::start(Options::default()).await;
    let room = living_room();
    let operator = fixture.operator_token();
    let socket = fixture.operator_socket().to_owned();
    let mut stream = open(&socket, &path::state_events(&room), &operator).await;
    stream.event_named(EventName::State).await;

    // The saved entry's host changes, which retires the session behind it.
    let mut config = locked(&fixture.config.0).clone();
    config.receivers.get_mut("living-room").unwrap().host = "192.0.2.11".into();
    fixture
        .service
        .handle(Principal::Operator)
        .unwrap()
        .save_configuration(&config)
        .await
        .unwrap();

    let end = stream.last_event().await.expect("an end event");
    assert_eq!(end_reason(&end), "session_closed");

    let mut again = open(&socket, &path::state_events(&room), &operator).await;
    again.event_named(EventName::State).await;
    assert_eq!(
        fixture
            .connector
            .opens
            .load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the new stream reached the receiver afresh"
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn shutdown_ends_every_stream_with_an_end_event() {
    let fixture = Fixture::start(Options::default()).await;
    let room = living_room();
    let mut streams = Vec::new();
    for (_, socket, token) in endpoints(&fixture).await {
        streams.push(open(&socket, &path::state_events(&room), &token).await);
        streams.push(open(&socket, &path::operation_events(), &token).await);
    }
    streams[0].event_named(EventName::State).await;
    streams[2].event_named(EventName::State).await;

    let reads = async {
        let mut reasons = Vec::new();
        for stream in &mut streams {
            let last = stream.last_event().await.expect("an end event");
            reasons.push(end_reason(&last));
        }
        reasons
    };
    let (reasons, ()) = tokio::join!(reads, fixture.shutdown());
    assert_eq!(reasons, ["shutdown"; 4]);
}

#[tokio::test]
async fn a_wait_returns_when_the_operation_finishes_and_otherwise_at_the_cap() {
    let fixture = Fixture::start(Options {
        max_operation_wait: Some(Duration::from_millis(1_500)),
        ..Options::default()
    })
    .await;
    let agent = fixture.agent_token("openclaw").await;
    let socket = fixture.agent_socket().to_owned();
    fixture.connector.hold_operations();
    let id = submit_mute(&socket, &agent).await["id"].as_u64().unwrap();

    // Held in the session, so the wait ends at the service's cap, not at the
    // caller's ten seconds.
    let started = Instant::now();
    let waited = get(&socket, &path::operation(id, Some(10_000)), &agent).await;
    let took = started.elapsed();
    assert_eq!(waited.status, 200, "{}", waited.text());
    assert_ne!(waited.json()["status"], "completed");
    assert!(
        took > Duration::from_millis(1_200) && took < Duration::from_secs(4),
        "the wait took {took:?}"
    );

    // The same operation, released while a wait is open, ends the wait at once.
    let waiting = {
        let (socket, agent) = (socket.clone(), agent.clone());
        tokio::spawn(async move {
            let started = Instant::now();
            let reply = get(&socket, &path::operation(id, Some(10_000)), &agent).await;
            (reply, started.elapsed())
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    fixture.connector.release_operations();
    let (reply, took) = waiting.await.unwrap();
    assert_eq!(reply.json()["status"], "completed", "{}", reply.text());
    assert!(
        took < Duration::from_millis(1_400),
        "the wait took {took:?}"
    );
    fixture.shutdown().await;
}

#[test]
fn the_request_timeout_exceeds_the_services_wait_cap() {
    assert!(Limits::default().request_timeout > ServiceConfig::default().max_operation_wait);
}

#[tokio::test]
async fn a_revoked_token_ends_its_streams_and_waits_within_a_second_and_refuses_its_next_request() {
    let fixture = Fixture::start(Options::default()).await;
    let room = living_room();
    let operator = fixture.operator_token();
    let alice = fixture.agent_token("alice").await;
    let agent_socket = fixture.agent_socket().to_owned();
    let operator_socket = fixture.operator_socket().to_owned();

    let mut states = open(&agent_socket, &path::state_events(&room), &alice).await;
    let mut operations = open(&agent_socket, &path::operation_events(), &alice).await;
    let mut operators = open(&operator_socket, &path::state_events(&room), &operator).await;
    states.event_named(EventName::State).await;
    operators.event_named(EventName::State).await;

    fixture.connector.hold_operations();
    let id = submit_mute(&agent_socket, &alice).await["id"]
        .as_u64()
        .unwrap();
    let waiting = {
        let (socket, alice) = (agent_socket.clone(), alice.clone());
        tokio::spawn(async move { get(&socket, &path::operation(id, Some(20_000)), &alice).await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;

    let revoked_at = Instant::now();
    revoke(&fixture, "alice").await;

    let second = Duration::from_secs(1);
    let end = tokio::time::timeout(second, states.last_event())
        .await
        .expect("the state stream ended within a second")
        .expect("an end event");
    assert_eq!(end_reason(&end), "revoked");
    let end = tokio::time::timeout(second, operations.last_event())
        .await
        .expect("the operation stream ended within a second")
        .expect("an end event");
    assert_eq!(end_reason(&end), "revoked");
    let waited = tokio::time::timeout(second, waiting)
        .await
        .expect("the wait ended within a second")
        .unwrap();
    assert_eq!(waited.status, 401, "{}", waited.text());
    assert!(revoked_at.elapsed() < Duration::from_secs(2));

    let next = get(&agent_socket, &path::health(), &alice).await;
    assert_eq!(next.status, 401);

    // The Operator's stream did not notice.
    fixture.connector.latest().turn_volume(-30.0);
    operators.event_named(EventName::State).await;
    fixture.connector.release_operations();
    fixture.shutdown().await;
}

#[tokio::test]
async fn get_state_takes_one_snapshot_and_drops_its_lease() {
    let fixture = Fixture::start(Options::default()).await;
    let room = living_room();
    for (name, socket, token) in endpoints(&fixture).await {
        let reply = get(&socket, &path::state(&room), &token).await;
        assert_eq!(reply.status, 200, "{name}: {}", reply.text());
        // The receiver is freed after its idle time, not held by the request.
        released(&fixture).await;
    }
    fixture.shutdown().await;
}
