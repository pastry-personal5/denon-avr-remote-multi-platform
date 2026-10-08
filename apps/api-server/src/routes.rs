//! The routes. Each endpoint's router is built from the route table, so a
//! resource that is not in an endpoint's table does not exist on it.

use crate::endpoint::EndpointContext;
use crate::pipeline::{authenticate, json_response, respond, ApiFailure, Authenticated, ConnInfo};
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::from_fn_with_state;
use axum::response::Response;
use axum::routing::{get, MethodRouter};
use axum::{Extension, Router};
use denon_avr_api_contract::routes::served_to;
use denon_avr_api_contract::{ApiError, HealthDto, RouteId};
use denon_avr_application::{AccessRefusal, EndpointKind, ReceiverReads, RefusalReason};
use std::sync::Arc;

type Context = Arc<EndpointContext>;

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
    match id {
        RouteId::Health => Some(get(health)),
        RouteId::Receivers
        | RouteId::State
        | RouteId::Sources
        | RouteId::StateEvents
        | RouteId::OperationEvents
        | RouteId::Submit
        | RouteId::DryRun
        | RouteId::Operation
        | RouteId::Cancel
        | RouteId::Discover
        | RouteId::AdHoc
        | RouteId::ConfigGet
        | RouteId::ConfigPut
        | RouteId::QuickSelectNames
        | RouteId::HttpInformation
        | RouteId::Refresh
        | RouteId::PolicyGet
        | RouteId::PolicyReload
        | RouteId::Audit
        | RouteId::TokenIssue
        | RouteId::TokenList
        | RouteId::TokenRevoke => None,
    }
}

async fn health(
    State(context): State<Context>,
    Extension(who): Extension<Authenticated>,
) -> Result<Response, ApiFailure> {
    let handle = context.service.handle(who.principal)?;
    let service = handle.health().await?;
    let server = (context.kind == EndpointKind::Operator).then(|| context.server_health.clone());
    Ok(json_response(
        StatusCode::OK,
        &HealthDto::new(&service, server),
    ))
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
