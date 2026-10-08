//! The Control API server.
//!
//! It hosts the in-process control service behind two endpoints, each a Unix
//! socket: the Operator endpoint, which serves the whole API to the Operator's
//! token, and the Agent endpoint, which serves only the resources an agent may use
//! to an Agent's token. A request is admitted by the peer's uid, authenticated, and
//! routed in that order, so a caller learns nothing before it has proved who it is.
//! Everything it does for a receiver it does through the control-service port.

#[cfg(not(unix))]
compile_error!("the Control API server is built for Unix only; macOS is the supported platform");

pub mod endpoint;
pub mod pipeline;
pub mod routes;
pub mod start;

pub use start::{AgentEndpointConfig, Limits, RunningServer, Server, ServerConfig, StartError};
