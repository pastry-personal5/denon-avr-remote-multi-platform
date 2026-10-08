//! The route table.
//!
//! Every resource of the Control API, with its method, its path pattern, and who
//! is served it. The server builds each endpoint's router from this table, and a
//! `match` on [`RouteId`] with no wildcard arm means a row without a handler does
//! not compile. The Agent endpoint's router holds only the rows marked
//! [`Audience::Both`], so a resource not in its table does not exist on it.

/// An HTTP method, named here so the contract needs no HTTP crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
        }
    }
}

/// Who a resource is served to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    /// Both endpoints: the Operator's, and the Agent's.
    Both,
    /// The Operator endpoint alone.
    Operator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RouteId {
    Health,
    Receivers,
    State,
    Sources,
    StateEvents,
    OperationEvents,
    Submit,
    DryRun,
    Operation,
    Cancel,
    Discover,
    AdHoc,
    ConfigGet,
    ConfigPut,
    QuickSelectNames,
    HttpInformation,
    Refresh,
    PolicyGet,
    PolicyReload,
    Audit,
    TokenIssue,
    TokenList,
    TokenRevoke,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    pub id: RouteId,
    pub method: Method,
    /// In the router's syntax: `{id}` is one path segment.
    pub pattern: &'static str,
    pub audience: Audience,
}

const fn route(id: RouteId, method: Method, pattern: &'static str, audience: Audience) -> Route {
    Route {
        id,
        method,
        pattern,
        audience,
    }
}

use Audience::{Both, Operator};
use Method::{Delete, Get, Post, Put};

pub const TABLE: &[Route] = &[
    route(RouteId::Health, Get, "/v1/health", Both),
    route(RouteId::Receivers, Get, "/v1/receivers", Both),
    route(RouteId::State, Get, "/v1/receivers/{id}/state", Both),
    route(RouteId::Sources, Get, "/v1/receivers/{id}/sources", Both),
    route(RouteId::StateEvents, Get, "/v1/receivers/{id}/events", Both),
    route(RouteId::OperationEvents, Get, "/v1/operations/events", Both),
    route(RouteId::Submit, Post, "/v1/receivers/{id}/operations", Both),
    route(
        RouteId::DryRun,
        Post,
        "/v1/receivers/{id}/operations/dry-run",
        Both,
    ),
    route(RouteId::Operation, Get, "/v1/operations/{id}", Both),
    route(RouteId::Cancel, Post, "/v1/operations/{id}/cancel", Both),
    route(RouteId::Discover, Post, "/v1/receivers/discover", Operator),
    route(RouteId::AdHoc, Post, "/v1/receivers/ad-hoc", Operator),
    route(RouteId::ConfigGet, Get, "/v1/config", Operator),
    route(RouteId::ConfigPut, Put, "/v1/config", Operator),
    route(
        RouteId::QuickSelectNames,
        Get,
        "/v1/receivers/{id}/quick-select-names",
        Operator,
    ),
    route(
        RouteId::HttpInformation,
        Get,
        "/v1/receivers/{id}/http-information",
        Operator,
    ),
    route(
        RouteId::Refresh,
        Post,
        "/v1/receivers/{id}/refresh",
        Operator,
    ),
    route(RouteId::PolicyGet, Get, "/v1/policy", Operator),
    route(RouteId::PolicyReload, Post, "/v1/policy/reload", Operator),
    route(RouteId::Audit, Get, "/v1/audit", Operator),
    route(RouteId::TokenIssue, Post, "/v1/tokens", Operator),
    route(RouteId::TokenList, Get, "/v1/tokens", Operator),
    route(RouteId::TokenRevoke, Delete, "/v1/tokens/{id}", Operator),
];

/// The rows an endpoint serves: all of them for the Operator, the shared ones for
/// an Agent.
pub fn served_to(operator: bool) -> impl Iterator<Item = &'static Route> {
    TABLE
        .iter()
        .filter(move |route| operator || route.audience == Audience::Both)
}
