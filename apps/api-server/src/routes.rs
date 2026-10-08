//! The routes. Each endpoint's router is built from the route table, so a
//! resource that is not in an endpoint's table does not exist on it.
//!
//! A handler takes the principal the pipeline authenticated, asks the control
//! service for a handle bound to it, and calls the port. It never decides who may
//! do what: the handle does, and the Agent endpoint has only the shared routes.

use crate::endpoint::EndpointContext;
use crate::pipeline::{authenticate, json_response, respond, ApiFailure, Authenticated, ConnInfo};
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::from_fn_with_state;
use axum::response::Response;
use axum::routing::{delete, get, post, put, MethodRouter};
use axum::{Extension, Router};
use denon_avr_api_contract::admin::{
    AdHocRequest, AdHocResponse, AuditPageDto, ConfigDto, ConfigRequest, DiscoverRequest,
    DiscoveredDto, DiscoveredListDto, HttpInformationDto, IssuedTokenDto, PolicyDto,
    QuickSelectNamesDto, ReadinessDto, TokenDto, TokenIssueRequest, TokenListDto,
};
use denon_avr_api_contract::requests::{
    AgentDryRun, DryRunRequest, OperatorDryRun, OperatorDryRunRequest, SubmitRequest,
};
use denon_avr_api_contract::routes::served_to;
use denon_avr_api_contract::{
    parse_receiver_id, AgentSourcesView, AgentStateView, ApiError, HealthDto, OperationDto,
    OperatorSourcesView, OperatorStateView, ReceiverSummaryDto, ReceiversDto, RouteId,
};
use denon_avr_application::{
    AccessRefusal, AgentLabel, AuditCursor, AuditQuery, EndpointKind, OperationControl,
    OperatorAdmin, ReceiverReads, RefusalReason, TokenId,
};
use denon_avr_domain::{ConfiguredReceivers, OperationId, ReceiverId};
use http_body_util::{BodyExt, Limited};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;

type Context = Arc<EndpointContext>;
type Reply = Result<Response, ApiFailure>;

/// The longest a discovery may listen, whatever the caller asks for.
const MAX_DISCOVERY: Duration = Duration::from_secs(30);

/// The router for one endpoint.
pub(crate) fn router(context: Context) -> Router {
    let operator = context.kind == EndpointKind::Operator;
    let mut router: Router<Context> = Router::new();
    for route in served_to(operator) {
        if let Some(handler) = handler_for(route.id) {
            router = router.route(route.pattern, handler);
        }
    }
    router
        .fallback(not_found)
        // After the routes and the fallback, so it wraps every one of them.
        .layer(from_fn_with_state(Arc::clone(&context), authenticate))
        .with_state(context)
}

/// The handler of each row of the table. There is no wildcard arm, so a new row
/// does not compile until it is listed here.
fn handler_for(id: RouteId) -> Option<MethodRouter<Context>> {
    Some(match id {
        RouteId::Health => get(health),
        RouteId::Receivers => get(receivers),
        RouteId::State => get(state),
        RouteId::Sources => get(sources),
        RouteId::Submit => post(submit),
        RouteId::DryRun => post(dry_run),
        RouteId::Operation => get(operation),
        RouteId::Cancel => post(cancel),
        RouteId::Discover => post(discover),
        RouteId::AdHoc => post(ad_hoc),
        RouteId::ConfigGet => get(config_get),
        RouteId::ConfigPut => put(config_put),
        RouteId::QuickSelectNames => get(quick_select_names),
        RouteId::HttpInformation => get(http_information),
        RouteId::Refresh => post(refresh),
        RouteId::PolicyGet => get(policy),
        RouteId::PolicyReload => post(policy_reload),
        RouteId::Audit => get(audit),
        RouteId::TokenIssue => post(token_issue),
        RouteId::TokenList => get(token_list),
        RouteId::TokenRevoke => delete(token_revoke),
        // The event streams are not served yet.
        RouteId::StateEvents | RouteId::OperationEvents => return None,
    })
}

// ---- Reading a request ----

fn bad(message: impl Into<String>) -> ApiFailure {
    ApiError::server(400, "invalid_request", message).into()
}

fn receiver(text: &str) -> Result<ReceiverId, ApiFailure> {
    parse_receiver_id(text).map_err(|why| bad(format!("not a receiver id: {why}")))
}

fn operation_id(text: &str) -> Result<OperationId, ApiFailure> {
    text.parse::<u64>()
        .map(OperationId)
        .map_err(|_| bad("an operation id is a whole number"))
}

/// A body as the contract type `T`, or why not. The message names a field and
/// never repeats what the caller sent.
async fn body<T: DeserializeOwned>(
    context: &EndpointContext,
    request: Request,
    limit: usize,
) -> Result<T, ApiFailure> {
    let is_json = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"))
        });
    if !is_json {
        return Err(ApiError::server(
            415,
            "unsupported_media_type",
            "a request body is application/json",
        )
        .into());
    }
    let collected = tokio::time::timeout(
        context.limits.body_timeout,
        Limited::new(request.into_body(), limit).collect(),
    )
    .await
    .map_err(|_| ApiError::server(408, "request_timeout", "the request body took too long"))?
    .map_err(|error| {
        if error
            .downcast_ref::<http_body_util::LengthLimitError>()
            .is_some()
        {
            ApiError::server(413, "payload_too_large", "the request body is too large")
        } else {
            ApiError::server(400, "invalid_request", "the request body could not be read")
        }
    })?;
    serde_json::from_slice(&collected.to_bytes()).map_err(|error| bad(describe(&error)))
}

/// A body that may be empty, which then reads as the type's default.
async fn optional_body<T: DeserializeOwned + Default>(
    context: &EndpointContext,
    request: Request,
    limit: usize,
) -> Result<T, ApiFailure> {
    let empty = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        == Some("0");
    if empty {
        return Ok(T::default());
    }
    body(context, request, limit).await
}

/// What is wrong with a body, without quoting it.
fn describe(error: &serde_json::Error) -> String {
    use serde_json::error::Category;
    let at = format!("line {}, column {}", error.line(), error.column());
    if error.classify() != Category::Data {
        return format!("the body is not valid JSON ({at})");
    }
    let text = error.to_string();
    // A field's name is what the caller got wrong, so it is named, cut short. A
    // value, a variant included, is the caller's text and is never repeated.
    for prefix in ["unknown field `", "missing field `"] {
        if let Some(rest) = text.strip_prefix(prefix) {
            if let Some(end) = rest.find('`') {
                let name: String = rest[..end].chars().take(64).collect();
                return format!("{}{name}` ({at})", prefix);
            }
        }
    }
    format!("a field has the wrong type or a value the contract does not accept ({at})")
}

fn ok<T: serde::Serialize>(value: &T) -> Reply {
    Ok(json_response(StatusCode::OK, value))
}

fn handle(
    context: &EndpointContext,
    who: Authenticated,
) -> Result<Arc<denon_avr_application::ServiceHandle>, ApiFailure> {
    Ok(context.service.handle(who.principal)?)
}

// ---- Both endpoints ----

async fn health(State(context): State<Context>, Extension(who): Extension<Authenticated>) -> Reply {
    let service = handle(&context, who)?.health().await?;
    let server = (context.kind == EndpointKind::Operator).then(|| context.server_health.clone());
    ok(&HealthDto::new(&service, server))
}

async fn receivers(
    State(context): State<Context>,
    Extension(who): Extension<Authenticated>,
) -> Reply {
    let list = handle(&context, who)?.receivers().await?;
    ok(&ReceiversDto {
        receivers: list.iter().map(ReceiverSummaryDto::from).collect(),
    })
}

/// A receiver's state: one snapshot of its subscription, which is dropped, so the
/// receiver's idle clock starts again.
async fn state(
    State(context): State<Context>,
    Extension(who): Extension<Authenticated>,
    Path(id): Path<String>,
) -> Reply {
    let id = receiver(&id)?;
    let subscription = handle(&context, who)?.state(&id).await?;
    let latest = subscription.latest();
    drop(subscription);
    match context.kind {
        EndpointKind::Operator => ok(&OperatorStateView::from(&latest)),
        EndpointKind::Agent => ok(&AgentStateView::from(&latest)),
    }
}

async fn sources(
    State(context): State<Context>,
    Extension(who): Extension<Authenticated>,
    Path(id): Path<String>,
) -> Reply {
    let id = receiver(&id)?;
    let catalog = handle(&context, who)?.source_catalog(&id).await?;
    match context.kind {
        EndpointKind::Operator => ok(&OperatorSourcesView::from(&catalog)),
        EndpointKind::Agent => ok(&AgentSourcesView::from(&catalog)),
    }
}

async fn submit(
    State(context): State<Context>,
    Extension(who): Extension<Authenticated>,
    Path(id): Path<String>,
    request: Request,
) -> Reply {
    let id = receiver(&id)?;
    let limit = context.limits.body_limit;
    let submission = body::<SubmitRequest>(&context, request, limit)
        .await?
        .into_submission()
        .map_err(bad)?;
    let snapshot = handle(&context, who)?.submit(&id, submission).await?;
    ok(&OperationDto::from(&snapshot))
}

/// A dry run. An agent's request has no `as_agent`; the Operator's may name one.
async fn dry_run(
    State(context): State<Context>,
    Extension(who): Extension<Authenticated>,
    Path(id): Path<String>,
    request: Request,
) -> Reply {
    let id = receiver(&id)?;
    let limit = context.limits.body_limit;
    let handle = handle(&context, who)?;
    match context.kind {
        EndpointKind::Agent => {
            let intent = body::<DryRunRequest>(&context, request, limit)
                .await?
                .intent
                .try_into()
                .map_err(bad)?;
            let result = handle.dry_run(&id, intent).await?;
            ok(&AgentDryRun::from(&result))
        }
        EndpointKind::Operator => {
            let request = body::<OperatorDryRunRequest>(&context, request, limit).await?;
            let agent = request.agent().map_err(bad)?;
            let intent = request.intent.try_into().map_err(bad)?;
            let result = match agent {
                Some(label) => handle.dry_run_as(label, &id, intent).await?,
                None => handle.dry_run(&id, intent).await?,
            };
            ok(&OperatorDryRun::from(&result))
        }
    }
}

#[derive(Deserialize)]
struct WaitQuery {
    wait_ms: Option<u64>,
}

async fn operation(
    State(context): State<Context>,
    Extension(who): Extension<Authenticated>,
    Path(id): Path<String>,
    Query(query): Query<WaitQuery>,
) -> Reply {
    let id = operation_id(&id)?;
    let wait = query.wait_ms.map(Duration::from_millis);
    let snapshot = handle(&context, who)?.operation(id, wait).await?;
    ok(&OperationDto::from(&snapshot))
}

async fn cancel(
    State(context): State<Context>,
    Extension(who): Extension<Authenticated>,
    Path(id): Path<String>,
) -> Reply {
    let id = operation_id(&id)?;
    let snapshot = handle(&context, who)?.cancel(id).await?;
    ok(&OperationDto::from(&snapshot))
}

// ---- The Operator's resources ----

async fn discover(
    State(context): State<Context>,
    Extension(who): Extension<Authenticated>,
    request: Request,
) -> Reply {
    let limit = context.limits.body_limit;
    let wanted = optional_body::<DiscoverRequest>(&context, request, limit).await?;
    let timeout = wanted
        .timeout_ms
        .map(Duration::from_millis)
        .unwrap_or(denon_avr_infrastructure::discovery_ssdp::DEFAULT_DISCOVERY_TIMEOUT)
        .min(MAX_DISCOVERY);
    let found = handle(&context, who)?.discover(timeout).await?;
    ok(&DiscoveredListDto {
        receivers: found.iter().map(DiscoveredDto::from).collect(),
    })
}

async fn ad_hoc(
    State(context): State<Context>,
    Extension(who): Extension<Authenticated>,
    request: Request,
) -> Reply {
    let limit = context.limits.body_limit;
    let identity = body::<AdHocRequest>(&context, request, limit).await?.into();
    let id = handle(&context, who)?.register_ad_hoc(identity).await?;
    ok(&AdHocResponse {
        id: id.as_str().to_owned(),
    })
}

/// The configuration's `ETag`: the SHA-256 of its canonical JSON, in quotes. A
/// client treats it as opaque.
fn etag_of(config: &ConfiguredReceivers) -> String {
    let bytes = serde_json::to_vec(&ConfigDto::from(config)).unwrap_or_default();
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("\"{hex}\"")
}

fn with_etag(mut response: Response, etag: &str) -> Response {
    if let Ok(value) = HeaderValue::from_str(etag) {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
}

async fn config_get(
    State(context): State<Context>,
    Extension(who): Extension<Authenticated>,
) -> Reply {
    let config = handle(&context, who)?.configuration().await?;
    Ok(with_etag(
        json_response(StatusCode::OK, &ConfigDto::from(&config)),
        &etag_of(&config),
    ))
}

/// Replace the configuration, if it is still the one the caller read. Two clients
/// that read the same file and each write it back would otherwise lose one edit.
async fn config_put(
    State(context): State<Context>,
    Extension(who): Extension<Authenticated>,
    request: Request,
) -> Reply {
    let Some(wanted) = request.headers().get(header::IF_MATCH).cloned() else {
        return Err(ApiError::server(
            428,
            "precondition_required",
            "send the ETag of the configuration you read in If-Match",
        )
        .into());
    };
    let handle = handle(&context, who)?;
    // One write at a time, from the comparison to the save.
    let _one_at_a_time = context.config_lock.lock().await;
    let current = handle.configuration().await?;
    if wanted.as_bytes() != etag_of(&current).as_bytes() {
        return Err(ApiError::server(
            412,
            "precondition_failed",
            "the configuration has changed since you read it; read it again",
        )
        .into());
    }
    let limit = context.limits.config_body_limit;
    let replacement: ConfiguredReceivers = body::<ConfigRequest>(&context, request, limit)
        .await?
        .try_into()
        .map_err(bad)?;
    handle.save_configuration(&replacement).await?;
    let saved = handle.configuration().await?;
    Ok(with_etag(
        json_response(StatusCode::OK, &ConfigDto::from(&saved)),
        &etag_of(&saved),
    ))
}

async fn quick_select_names(
    State(context): State<Context>,
    Extension(who): Extension<Authenticated>,
    Path(id): Path<String>,
) -> Reply {
    let id = receiver(&id)?;
    let names = handle(&context, who)?.quick_select_names(&id).await?;
    ok(&QuickSelectNamesDto::from(&names))
}

async fn http_information(
    State(context): State<Context>,
    Extension(who): Extension<Authenticated>,
    Path(id): Path<String>,
) -> Reply {
    let id = receiver(&id)?;
    let info = handle(&context, who)?.http_information(&id).await?;
    ok(&HttpInformationDto::from(&info))
}

async fn refresh(
    State(context): State<Context>,
    Extension(who): Extension<Authenticated>,
    Path(id): Path<String>,
) -> Reply {
    let id = receiver(&id)?;
    let readiness = handle(&context, who)?.refresh(&id).await?;
    ok(&ReadinessDto::from(&readiness))
}

async fn policy(State(context): State<Context>, Extension(who): Extension<Authenticated>) -> Reply {
    let view = handle(&context, who)?.policy().await?;
    ok(&PolicyDto::from(&view))
}

async fn policy_reload(
    State(context): State<Context>,
    Extension(who): Extension<Authenticated>,
) -> Reply {
    let view = handle(&context, who)?.reload_policy().await?;
    ok(&PolicyDto::from(&view))
}

#[derive(Deserialize)]
struct AuditParameters {
    limit: Option<usize>,
    cursor: Option<u64>,
}

async fn audit(
    State(context): State<Context>,
    Extension(who): Extension<Authenticated>,
    Query(parameters): Query<AuditParameters>,
) -> Reply {
    let mut query = AuditQuery::new(parameters.limit.unwrap_or(50));
    if let Some(cursor) = parameters.cursor {
        query = query.after(AuditCursor::from_seq(cursor));
    }
    let page = handle(&context, who)?.audit(query).await?;
    ok(&AuditPageDto::from(&page))
}

async fn token_issue(
    State(context): State<Context>,
    Extension(who): Extension<Authenticated>,
    request: Request,
) -> Reply {
    let limit = context.limits.body_limit;
    let wanted = body::<TokenIssueRequest>(&context, request, limit).await?;
    let label = AgentLabel::new(wanted.label).map_err(bad)?;
    let issued = handle(&context, who)?.issue_token(label).await?;
    Ok(json_response(
        StatusCode::CREATED,
        &IssuedTokenDto::from(&issued),
    ))
}

async fn token_list(
    State(context): State<Context>,
    Extension(who): Extension<Authenticated>,
) -> Reply {
    let tokens = handle(&context, who)?.tokens().await?;
    ok(&TokenListDto {
        tokens: tokens.iter().map(TokenDto::from).collect(),
    })
}

async fn token_revoke(
    State(context): State<Context>,
    Extension(who): Extension<Authenticated>,
    Path(id): Path<String>,
) -> Reply {
    let id = TokenId::new(id).map_err(bad)?;
    let record = handle(&context, who)?.revoke_token(id).await?;
    ok(&TokenDto::from(&record))
}

/// A path that is not a resource of this endpoint. On the Agent endpoint it is
/// audited, with the route pattern the agent was after when there is one.
async fn not_found(
    State(context): State<Context>,
    Extension(who): Extension<Authenticated>,
    Extension(connection): Extension<ConnInfo>,
    request: Request,
) -> Response {
    if context.kind == EndpointKind::Agent {
        let path = request.uri().path().to_owned();
        let resource = denon_avr_api_contract::routes::TABLE
            .iter()
            .find(|route| fits(route.pattern, &path))
            .map(|route| route.pattern.to_owned());
        context
            .service
            .record_refusal(AccessRefusal {
                endpoint: EndpointKind::Agent,
                reason: RefusalReason::ResourceNotServed,
                principal: Some(who.principal),
                peer_uid: connection.peer_uid,
                resource,
            })
            .await;
    }
    respond(ApiError::server(404, "not_found", "no such resource"))
}

/// Whether `path` fits a route pattern, `{id}` standing for one whole segment.
fn fits(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();
    pattern.len() == path.len()
        && pattern
            .iter()
            .zip(&path)
            .all(|(p, q)| (p.starts_with('{') && !q.is_empty()) || p == q)
}
