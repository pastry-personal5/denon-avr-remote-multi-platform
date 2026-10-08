//! One endpoint: the socket's accept loop and each connection it admits.
//!
//! The loop reads the peer's credentials before it reads a byte, counts the
//! connection against the endpoint's cap, and serves it with hyper's HTTP/1
//! handler, whose settings are the pipeline's first limits. The router it serves is
//! built from the route table for this endpoint alone.

use crate::pipeline::ConnInfo;
use crate::start::Limits;
use axum::Extension;
use denon_avr_api_contract::ServerHealthDto;
use denon_avr_application::{
    AccessRefusal, ControlService, EndpointKind, RefusalReason, SharedTokenStore,
};
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

/// What every request on an endpoint needs.
pub struct EndpointContext {
    pub kind: EndpointKind,
    pub service: Arc<ControlService>,
    pub tokens: SharedTokenStore,
    pub limits: Arc<Limits>,
    /// The uids whose connections are served.
    pub admitted_uids: Vec<u32>,
    pub server_health: ServerHealthDto,
    pub shutdown: watch::Receiver<bool>,
}

/// Whether a peer is served. The uid is read from the connection by the operating
/// system; a peer whose uid could not be read is not.
pub fn admits(admitted: &[u32], peer_uid: Option<u32>) -> bool {
    peer_uid.is_some_and(|uid| admitted.contains(&uid))
}

pub(crate) fn spawn(
    listener: UnixListener,
    context: Arc<EndpointContext>,
    connections: Arc<Semaphore>,
) -> JoinHandle<()> {
    tokio::spawn(accept_loop(listener, context, connections))
}

async fn accept_loop(
    listener: UnixListener,
    context: Arc<EndpointContext>,
    connections: Arc<Semaphore>,
) {
    let router = crate::routes::router(Arc::clone(&context));
    loop {
        let accepted = tokio::select! {
            biased;
            () = stopped(context.shutdown.clone()) => break,
            accepted = listener.accept() => accepted,
        };
        let stream = match accepted {
            Ok((stream, _)) => stream,
            Err(error) => {
                warn!(%error, "accepting a connection");
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let peer_uid = stream.peer_cred().ok().map(|credentials| credentials.uid());
        if !admits(&context.admitted_uids, peer_uid) {
            // Closed before a byte is read.
            drop(stream);
            let context = Arc::clone(&context);
            tokio::spawn(async move {
                context
                    .service
                    .record_refusal(AccessRefusal {
                        endpoint: context.kind,
                        reason: RefusalReason::PeerNotAdmitted,
                        principal: None,
                        peer_uid,
                        resource: None,
                    })
                    .await;
            });
            continue;
        }
        let Ok(permit) = Arc::clone(&connections).try_acquire_owned() else {
            debug!("a connection was closed: the endpoint is at its cap");
            continue;
        };
        tokio::spawn(serve_connection(
            stream,
            Arc::clone(&context),
            router.clone(),
            permit,
            peer_uid,
        ));
    }
}

async fn serve_connection(
    stream: UnixStream,
    context: Arc<EndpointContext>,
    router: axum::Router,
    permit: OwnedSemaphorePermit,
    peer_uid: Option<u32>,
) {
    let _permit = permit;
    let app = router.layer(Extension(ConnInfo { peer_uid }));
    let service = TowerToHyperService::new(app);
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(context.limits.head_timeout)
        .max_buf_size(context.limits.max_head_bytes)
        .keep_alive(true);
    let connection = builder.serve_connection(TokioIo::new(stream), service);
    tokio::pin!(connection);
    tokio::select! {
        result = connection.as_mut() => {
            if let Err(error) = result {
                debug!(%error, "a connection ended with an error");
            }
        }
        () = stopped(context.shutdown.clone()) => {
            connection.as_mut().graceful_shutdown();
            let _ = tokio::time::timeout(context.limits.grace, connection).await;
        }
    }
}

/// Resolves when the server is told to stop. A function of its own so that the
/// guard `wait_for` returns is dropped inside it and never held across an await.
pub(crate) async fn stopped(mut shutdown: watch::Receiver<bool>) {
    let _ = shutdown.wait_for(|stop| *stop).await;
}
