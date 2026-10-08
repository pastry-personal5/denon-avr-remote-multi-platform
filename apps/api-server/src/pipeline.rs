//! What every request passes through, in order, before a handler sees it.
//!
//! The order is the design: the peer was admitted by its uid when the connection
//! was accepted; then the credential is checked, and only then is anything said
//! about what the path is. A caller that has not proved who it is learns nothing
//! about which resources exist: a missing, unknown, revoked, or wrong-kind
//! credential all get the same `401`, whatever the path.
//!
//! This is a layer on the router, added after the routes and the fallback, so it
//! wraps every one of them, a path that matches nothing and a method that is not
//! allowed included.

use crate::endpoint::EndpointContext;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Extension;
use denon_avr_api_contract::ApiError;
use denon_avr_application::{
    AccessRefusal, ControlError, Credential, EndpointKind, Principal, RefusalReason, TokenId,
};
use serde::Serialize;
use std::sync::Arc;

/// What the accept loop learned about the connection.
#[derive(Debug, Clone, Copy)]
pub struct ConnInfo {
    pub peer_uid: Option<u32>,
}

/// Who a request is from, once it has been authenticated.
#[derive(Debug, Clone)]
pub struct Authenticated {
    pub principal: Principal,
    /// The Agent token it presented, so a stream can end when it is revoked.
    pub token: Option<TokenId>,
}

/// A failure a handler returns: a status and the body that goes with it.
#[derive(Debug)]
pub struct ApiFailure(pub Box<ApiError>);

impl From<ControlError> for ApiFailure {
    fn from(error: ControlError) -> Self {
        Self(Box::new(ApiError::from(&error)))
    }
}

impl From<ApiError> for ApiFailure {
    fn from(error: ApiError) -> Self {
        Self(Box::new(error))
    }
}

impl IntoResponse for ApiFailure {
    fn into_response(self) -> Response {
        respond(*self.0)
    }
}

/// An error as a response: its status, its JSON body, and `Retry-After` when it
/// asks for one.
pub fn respond(error: ApiError) -> Response {
    let status = StatusCode::from_u16(error.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let retry_after = error.retry_after_secs();
    let mut response = json_response(status, &error.body);
    if let Some(seconds) = retry_after {
        if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
    }
    response
}

pub fn json_response<T: Serialize>(status: StatusCode, value: &T) -> Response {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    response
}

/// The query parameters that would put a credential in a URL, where it is logged
/// and kept. They are refused whatever else the request is.
const CREDENTIAL_PARAMETERS: [&str; 6] = [
    "token",
    "access_token",
    "api_key",
    "apikey",
    "authorization",
    "bearer",
];

fn credential_in_query(query: Option<&str>) -> bool {
    query.is_some_and(|query| {
        query.split('&').any(|pair| {
            let name = pair
                .split('=')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            CREDENTIAL_PARAMETERS.contains(&name.as_str())
        })
    })
}

/// How a request's credential came out.
enum Presented {
    None,
    /// More than one `Authorization` header, or one that is not a bearer token.
    Malformed,
    Token(String),
}

fn presented(request: &Request) -> Presented {
    let mut values = request.headers().get_all(header::AUTHORIZATION).iter();
    let Some(first) = values.next() else {
        return Presented::None;
    };
    if values.next().is_some() {
        return Presented::Malformed;
    }
    let Ok(text) = first.to_str() else {
        return Presented::Malformed;
    };
    let mut parts = text.splitn(2, ' ');
    match (parts.next(), parts.next()) {
        (Some(scheme), Some(token))
            if scheme.eq_ignore_ascii_case("bearer") && !token.is_empty() =>
        {
            Presented::Token(token.to_owned())
        }
        _ => Presented::Malformed,
    }
}

/// Authenticate a request, enforce the body and time limits, and pass it on.
pub async fn authenticate(
    State(context): State<Arc<EndpointContext>>,
    Extension(connection): Extension<ConnInfo>,
    mut request: Request,
    next: Next,
) -> Response {
    // A credential in a URL is refused before anything else is looked at.
    if credential_in_query(request.uri().query()) {
        return respond(ApiError::server(
            400,
            "invalid_request",
            "a credential belongs in the Authorization header and never in the URL",
        ));
    }

    let refuse = |reason: RefusalReason| {
        let context = Arc::clone(&context);
        async move {
            context
                .service
                .record_refusal(AccessRefusal {
                    endpoint: context.kind,
                    reason,
                    principal: None,
                    peer_uid: connection.peer_uid,
                    resource: None,
                })
                .await;
            respond(ApiError::unauthenticated())
        }
    };

    let token = match presented(&request) {
        Presented::None => return refuse(RefusalReason::NoCredential).await,
        Presented::Malformed => return refuse(RefusalReason::UnknownCredential).await,
        Presented::Token(token) => token,
    };
    let authenticated = match (context.kind, context.tokens.authenticate(&token)) {
        (EndpointKind::Operator, Credential::OperatorToken) => Authenticated {
            principal: Principal::Operator,
            token: None,
        },
        (EndpointKind::Agent, Credential::Agent(label, id)) => Authenticated {
            principal: Principal::Agent(label),
            token: Some(id),
        },
        (EndpointKind::Agent, Credential::OperatorToken) => {
            return refuse(RefusalReason::OperatorTokenOnAgentEndpoint).await
        }
        (EndpointKind::Operator, Credential::Agent(..)) => {
            return refuse(RefusalReason::AgentTokenOnOperatorEndpoint).await
        }
        (_, Credential::Unknown) => return refuse(RefusalReason::UnknownCredential).await,
    };

    if let Some(failure) = check_body(&context, &request) {
        return respond(failure);
    }
    request.extensions_mut().insert(authenticated);

    // Time is allowed for the handler to answer, which for an event stream is until
    // the stream begins. The stream itself, once it is a body, is not timed.
    let response =
        match tokio::time::timeout(context.limits.request_timeout, next.run(request)).await {
            Ok(response) => response,
            Err(_) => respond(ApiError::server(
                408,
                "request_timeout",
                "the request took too long to answer",
            )),
        };
    tidy(response)
}

/// What the declared length of a body says, before a byte of it is read.
fn check_body(context: &EndpointContext, request: &Request) -> Option<ApiError> {
    let headers = request.headers();
    if headers.contains_key(header::TRANSFER_ENCODING) {
        return Some(ApiError::server(
            411,
            "length_required",
            "a request body must declare its length; chunked bodies are not accepted",
        ));
    }
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .map(str::parse::<usize>);
    let sends_body = matches!(*request.method(), Method::POST | Method::PUT);
    let length = match declared {
        Some(Ok(length)) => length,
        Some(Err(_)) => {
            return Some(ApiError::server(
                400,
                "invalid_request",
                "the body length is not a number",
            ))
        }
        None if sends_body => {
            return Some(ApiError::server(
                411,
                "length_required",
                "a request body must declare its length",
            ))
        }
        None => 0,
    };
    let limit = if *request.method() == Method::PUT && request.uri().path() == "/v1/config" {
        context.limits.config_body_limit
    } else {
        context.limits.body_limit
    };
    (length > limit).then(|| {
        ApiError::server(
            413,
            "payload_too_large",
            format!("a request body may be at most {limit} bytes"),
        )
    })
}

/// A response in the contract's form: a `405` the router made on its own becomes
/// the same JSON error as any other.
fn tidy(response: Response) -> Response {
    if response.status() == StatusCode::METHOD_NOT_ALLOWED {
        let allow = response.headers().get(header::ALLOW).cloned();
        let mut replaced = respond(ApiError::server(
            405,
            "method_not_allowed",
            "that method is not allowed on this resource",
        ));
        if let Some(allow) = allow {
            replaced.headers_mut().insert(header::ALLOW, allow);
        }
        return replaced;
    }
    // An error axum made on its own, such as a path segment that is not text, is
    // plain text. The contract's errors are JSON.
    let json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/json"));
    if response.status().is_client_error() && !json {
        let status = response.status().as_u16();
        return respond(ApiError::server(
            status,
            "invalid_request",
            "the request could not be understood",
        ));
    }
    response
}
