//! One connection for each request, and one for each event stream.
//!
//! A Unix socket is cheap to open, and a connection kept between requests can be
//! reset by a server that restarted, so nothing is reused. A request says
//! `Connection: close`, which also keeps the server's connection cap for streams
//! and for other clients.

use crate::mapping::{error_response, UNAVAILABLE};
use crate::{Audience, ControlError};
use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::header::{HeaderValue, AUTHORIZATION, CONNECTION, CONTENT_LENGTH, CONTENT_TYPE, HOST};
use hyper::{Method, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::PathBuf;
use std::time::Duration;
use tokio::net::UnixStream;
use tokio::task::AbortHandle;

pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// The most a plain answer may be. The longest the server writes is a configuration.
const MAX_ANSWER: usize = 4 * 1024 * 1024;

/// Stops a task when dropped, which closes the connection it was driving.
pub(crate) struct Abort(AbortHandle);

impl Abort {
    /// Guard a task that was spawned and is not awaited.
    pub(crate) fn of<T>(task: &tokio::task::JoinHandle<T>) -> Self {
        Self(task.abort_handle())
    }
}

impl Drop for Abort {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The credential. It prints as `<redacted>` and is in no error.
#[derive(Clone)]
pub struct Token(String);

impl Token {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(crate) fn header(&self) -> Option<HeaderValue> {
        let mut value = HeaderValue::from_str(&format!("Bearer {}", self.0)).ok()?;
        value.set_sensitive(true);
        Some(value)
    }

    /// Whether the text can be sent as a bearer token at all.
    pub(crate) fn is_well_formed(&self) -> bool {
        !self.0.is_empty() && self.0.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

impl From<String> for Token {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for Token {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// What to ask for.
pub(crate) struct Call {
    method: Method,
    path: String,
    body: Option<Vec<u8>>,
    if_match: Option<String>,
    timeout: Duration,
}

impl Call {
    fn new(method: Method, path: String) -> Self {
        Self {
            method,
            path,
            body: None,
            if_match: None,
            timeout: REQUEST_TIMEOUT,
        }
    }

    pub(crate) fn get(path: String) -> Self {
        Self::new(Method::GET, path)
    }

    pub(crate) fn post(path: String, body: &impl Serialize) -> Result<Self, ControlError> {
        let mut call = Self::new(Method::POST, path);
        call.body = Some(encode(body)?);
        Ok(call)
    }

    /// A `POST` that carries nothing.
    pub(crate) fn post_empty(path: String) -> Self {
        Self::new(Method::POST, path)
    }

    pub(crate) fn put(path: String, body: &impl Serialize) -> Result<Self, ControlError> {
        let mut call = Self::new(Method::PUT, path);
        call.body = Some(encode(body)?);
        Ok(call)
    }

    pub(crate) fn delete(path: String) -> Self {
        Self::new(Method::DELETE, path)
    }

    pub(crate) fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub(crate) fn if_match(mut self, etag: String) -> Self {
        self.if_match = Some(etag);
        self
    }
}

fn encode(body: &impl Serialize) -> Result<Vec<u8>, ControlError> {
    serde_json::to_vec(body)
        .map_err(|_| ControlError::InvalidRequest("the request could not be written".into()))
}

/// A successful answer.
pub(crate) struct Answer {
    pub(crate) etag: Option<String>,
    pub(crate) body: Bytes,
}

pub(crate) struct Transport {
    pub(crate) socket: PathBuf,
    pub(crate) token: Token,
    pub(crate) audience: Audience,
}

impl Transport {
    /// The error for a server that cannot be reached. The Operator is told where
    /// it looked; an agent is told only what the design says.
    pub(crate) fn unreachable(&self) -> ControlError {
        match self.audience {
            Audience::Operator => ControlError::Unavailable(format!(
                "{UNAVAILABLE} (no answer at {})",
                self.socket.display()
            )),
            Audience::Agent => ControlError::Unavailable(UNAVAILABLE.into()),
        }
    }

    async fn connect(
        &self,
    ) -> Result<(hyper::client::conn::http1::SendRequest<Full<Bytes>>, Abort), ControlError> {
        let stream = tokio::time::timeout(CONNECT_TIMEOUT, UnixStream::connect(&self.socket))
            .await
            .map_err(|_| self.unreachable())?
            .map_err(|_| self.unreachable())?;
        let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|_| self.unreachable())?;
        let task = tokio::spawn(async move {
            let _ = connection.await;
        });
        Ok((sender, Abort(task.abort_handle())))
    }

    fn request(
        &self,
        call: &Call,
        streaming: bool,
    ) -> Result<hyper::Request<Full<Bytes>>, ControlError> {
        let body = call.body.clone().unwrap_or_default();
        let sends_body = matches!(call.method, Method::POST | Method::PUT);
        let mut request = hyper::Request::builder()
            .method(call.method.clone())
            .uri(call.path.as_str())
            .body(Full::new(Bytes::from(body.clone())))
            .map_err(|_| ControlError::InvalidRequest("the request could not be formed".into()))?;
        let headers = request.headers_mut();
        headers.insert(HOST, HeaderValue::from_static("denon-avr"));
        if let Some(value) = self.token.header() {
            headers.insert(AUTHORIZATION, value);
        }
        if !streaming {
            headers.insert(CONNECTION, HeaderValue::from_static("close"));
        }
        if call.body.is_some() {
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        }
        if sends_body {
            headers.insert(CONTENT_LENGTH, HeaderValue::from(body.len()));
        }
        if let Some(etag) = &call.if_match {
            if let Ok(value) = HeaderValue::from_str(etag) {
                headers.insert(hyper::header::IF_MATCH, value);
            }
        }
        Ok(request)
    }

    /// Send a request and read the whole answer. Anything but a success is the
    /// port's error for it.
    pub(crate) async fn answer(&self, call: Call) -> Result<Answer, ControlError> {
        let exchange = async {
            let (mut sender, _connection) = self.connect().await?;
            let response = sender
                .send_request(self.request(&call, false)?)
                .await
                .map_err(|_| self.unreachable())?;
            let (head, body) = response.into_parts();
            let body = Limited::new(body, MAX_ANSWER)
                .collect()
                .await
                .map_err(|_| self.unreachable())?
                .to_bytes();
            if head.status.is_success() {
                let etag = head
                    .headers
                    .get(hyper::header::ETAG)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned);
                Ok(Answer { etag, body })
            } else {
                Err(error_response(head.status, &body))
            }
        };
        tokio::time::timeout(call.timeout, exchange)
            .await
            .map_err(|_| self.unreachable())?
    }

    /// Send a request and read its answer as `T`.
    pub(crate) async fn json<T: DeserializeOwned>(&self, call: Call) -> Result<T, ControlError> {
        let answer = self.answer(call).await?;
        serde_json::from_slice(&answer.body).map_err(crate::mapping::unreadable)
    }

    /// Open an event stream. The response is returned once its head has arrived
    /// and is a success; the connection is kept for as long as the guard lives.
    pub(crate) async fn open_stream(
        &self,
        path: String,
    ) -> Result<(Response<Incoming>, Abort), ControlError> {
        let call = Call::get(path);
        let opening = async {
            let (mut sender, connection) = self.connect().await?;
            let response = sender
                .send_request(self.request(&call, true)?)
                .await
                .map_err(|_| self.unreachable())?;
            if response.status() == StatusCode::OK {
                return Ok((response, connection));
            }
            let (head, body) = response.into_parts();
            let body = Limited::new(body, MAX_ANSWER)
                .collect()
                .await
                .map_err(|_| self.unreachable())?
                .to_bytes();
            Err(error_response(head.status, &body))
        };
        tokio::time::timeout(REQUEST_TIMEOUT, opening)
            .await
            .map_err(|_| self.unreachable())?
    }
}
