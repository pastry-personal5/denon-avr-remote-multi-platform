//! The two event streams, and what ends a stream or a wait early.
//!
//! A stream's body is fed by a channel of one item. The task that holds the
//! subscription sends each event with a timeout and ends the stream when it
//! expires, which drops the subscription and with it the lease that keeps a
//! receiver connected. Without that, a client that stopped reading would hold the
//! receiver's single control connection for as long as it stayed connected.
//!
//! Four things end a stream before its client does: the service closing the
//! session under it, its token being revoked, the server shutting down, and, for an
//! Agent's state stream, reaching its maximum age. Each is told to the client as an
//! `end` event first, so the client knows whether to subscribe again.

use crate::endpoint::{stopped, EndpointContext};
use crate::pipeline::{ApiFailure, Authenticated};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use denon_avr_api_contract::events::{EndPayload, EndReason, EventName};
use denon_avr_api_contract::{AgentStateView, ApiError, OperationDto, OperatorStateView};
use denon_avr_application::{
    EndpointKind, OperationEvent, OperationEvents, Principal, StateSubscription, TokenId,
};
use denon_avr_domain::ReceiverState;
use serde::Serialize;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::mpsc;

type Item = Result<Event, Infallible>;

/// How many streams each principal holds open on one endpoint.
#[derive(Default)]
pub struct Streams {
    open: Mutex<HashMap<String, usize>>,
}

impl Streams {
    /// A place for one more stream of `who`, or the `429` that says there is none.
    pub fn open(
        self: &Arc<Self>,
        who: &Authenticated,
        limit: usize,
    ) -> Result<StreamSlot, ApiFailure> {
        let key = match &who.principal {
            Principal::Operator => "operator".to_owned(),
            Principal::Agent(label) => format!("agent:{}", label.as_str()),
        };
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        let count = open.entry(key.clone()).or_insert(0);
        if *count >= limit {
            return Err(ApiError::server(
                429,
                "too_many_streams",
                format!("at most {limit} event streams may be open at once"),
            )
            .into());
        }
        *count += 1;
        Ok(StreamSlot {
            streams: Arc::clone(self),
            key,
        })
    }
}

/// One of a principal's streams. Dropping it frees the place.
pub struct StreamSlot {
    streams: Arc<Streams>,
    key: String,
}

impl Drop for StreamSlot {
    fn drop(&mut self) {
        let mut open = self
            .streams
            .open
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = open.get_mut(&self.key) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                open.remove(&self.key);
            }
        }
    }
}

/// Resolves when the Agent token a request came in with is revoked, and never for
/// the Operator's. It looks again each time the store says a revocation happened.
pub async fn revoked(context: &EndpointContext, token: Option<&TokenId>) {
    let Some(id) = token else {
        return std::future::pending().await;
    };
    // Subscribed before the first look, so a revocation in between is not missed.
    let mut changes = context.tokens.changes();
    loop {
        if !context.tokens.is_active(id) {
            return;
        }
        if changes.changed().await.is_err() {
            return std::future::pending().await;
        }
    }
}

fn event(name: EventName, payload: &impl Serialize) -> Event {
    let data = serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_owned());
    Event::default().event(name.as_str()).data(data)
}

fn state_event(kind: EndpointKind, state: &ReceiverState) -> Event {
    match kind {
        EndpointKind::Operator => event(EventName::State, &OperatorStateView::from(state)),
        EndpointKind::Agent => event(EventName::State, &AgentStateView::from(state)),
    }
}

/// The data of an event with nothing to say.
#[derive(Serialize)]
struct Empty {}

/// Hand an event to the body, or report that the client is not taking events: it
/// has gone, or it has not read for `timeout`.
async fn send(tx: &mpsc::Sender<Item>, event: Event, timeout: Duration) -> bool {
    matches!(
        tokio::time::timeout(timeout, tx.send(Ok(event))).await,
        Ok(Ok(()))
    )
}

fn response(rx: mpsc::Receiver<Item>, keep_alive: Duration) -> Response {
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(keep_alive))
        .into_response()
}

/// Stream a receiver's state: the whole state first, then the newest state after
/// each change. The subscription coalesces, so a client that reads slowly sees the
/// latest state and never a backlog.
pub fn state_stream(
    context: Arc<EndpointContext>,
    who: Authenticated,
    subscription: StateSubscription,
    slot: StreamSlot,
) -> Response {
    let (tx, rx) = mpsc::channel(1);
    let keep_alive = context.limits.keep_alive;
    tokio::spawn(async move {
        let _slot = slot;
        // Moved in, so it is dropped when the pump returns and the lease with it,
        // before the closing event is written.
        let reason = pump_state(&context, &who, subscription, &tx).await;
        if let Some(reason) = reason {
            let end = event(EventName::End, &EndPayload { reason });
            let _ = send(&tx, end, context.limits.send_timeout).await;
        }
    });
    response(rx, keep_alive)
}

async fn pump_state(
    context: &EndpointContext,
    who: &Authenticated,
    mut subscription: StateSubscription,
    tx: &mpsc::Sender<Item>,
) -> Option<EndReason> {
    let timeout = context.limits.send_timeout;
    let kind = context.kind;
    if !send(tx, state_event(kind, &subscription.latest()), timeout).await {
        return None;
    }
    // An Agent's stream has a maximum age, which is what frees the receiver from a
    // client that is connected and not reading when the receiver is quiet. The
    // Operator's subscription is meant to be held.
    let age = async {
        match kind {
            EndpointKind::Agent => tokio::time::sleep(context.limits.agent_stream_max_age).await,
            EndpointKind::Operator => std::future::pending().await,
        }
    };
    tokio::pin!(age);
    loop {
        tokio::select! {
            biased;
            () = tx.closed() => return None,
            () = stopped(context.shutdown.clone()) => return Some(EndReason::Shutdown),
            () = revoked(context, who.token.as_ref()) => return Some(EndReason::Revoked),
            () = &mut age => return Some(EndReason::MaxAge),
            changed = subscription.changed() => match changed {
                Ok(state) => {
                    if !send(tx, state_event(kind, &state), timeout).await {
                        return None;
                    }
                }
                Err(_) => return Some(EndReason::SessionClosed),
            },
        }
    }
}

/// Stream the caller's operations: every operation's, for the Operator. The stream
/// holds no lease on a receiver.
pub fn operation_stream(
    context: Arc<EndpointContext>,
    who: Authenticated,
    events: OperationEvents,
    slot: StreamSlot,
) -> Response {
    let (tx, rx) = mpsc::channel(1);
    let keep_alive = context.limits.keep_alive;
    tokio::spawn(async move {
        let _slot = slot;
        let reason = pump_operations(&context, &who, events, &tx).await;
        if let Some(reason) = reason {
            let end = event(EventName::End, &EndPayload { reason });
            let _ = send(&tx, end, context.limits.send_timeout).await;
        }
    });
    response(rx, keep_alive)
}

async fn pump_operations(
    context: &EndpointContext,
    who: &Authenticated,
    mut events: OperationEvents,
    tx: &mpsc::Sender<Item>,
) -> Option<EndReason> {
    let timeout = context.limits.send_timeout;
    loop {
        tokio::select! {
            biased;
            () = tx.closed() => return None,
            () = stopped(context.shutdown.clone()) => return Some(EndReason::Shutdown),
            () = revoked(context, who.token.as_ref()) => return Some(EndReason::Revoked),
            next = events.next() => {
                let sent = match next {
                    Some(OperationEvent::Update(snapshot)) => {
                        send(tx, event(EventName::Operation, &OperationDto::from(&snapshot)), timeout).await
                    }
                    Some(OperationEvent::Missed) => {
                        send(tx, event(EventName::Missed, &Empty {}), timeout).await
                    }
                    // The service stopped producing events: it is shutting down.
                    None => return Some(EndReason::Shutdown),
                };
                if !sent {
                    return None;
                }
            }
        }
    }
}
