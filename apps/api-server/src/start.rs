//! Starting the server: the lock that makes it the only one, the Operator
//! endpoint and, when configured, the Agent endpoint, and shutting them down.
//!
//! The order is the point. The `run/` directory is made private first. The lock is
//! taken before any socket is touched, so a second server stops there and a stale
//! socket is removed only by the server that holds the lock. A path that is not a
//! socket of this server's own is never removed.

use crate::endpoint::{self, EndpointContext};
use denon_avr_api_contract::receivers::{AgentEndpointDto, ServerHealthDto, TokenStoreDto};
use denon_avr_api_contract::EndpointPaths;
use denon_avr_application::{ControlService, EndpointKind, SharedTokenStore};
use denon_avr_infrastructure::ensure_private_directory;
use std::fmt;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UnixListener;
use tokio::sync::{watch, Semaphore};
use tokio::task::JoinHandle;
use tracing::{info, warn};

/// The longest a socket path may be, as the platform's `sun_path` allows (it holds
/// the terminating NUL as well).
#[cfg(target_os = "macos")]
const SUN_PATH_LEN: usize = 104;
#[cfg(not(target_os = "macos"))]
const SUN_PATH_LEN: usize = 108;

/// The caps and timeouts. Every connection is anonymous until it authenticates, so
/// the head and body are bounded before anything is read from them.
#[derive(Debug, Clone)]
pub struct Limits {
    pub operator_connections: usize,
    pub agent_connections: usize,
    /// How long a client has to send a request's head, and to send the next one
    /// on a connection that is kept open.
    pub head_timeout: Duration,
    pub max_head_bytes: usize,
    /// How long a request body may take to arrive.
    pub body_timeout: Duration,
    pub body_limit: usize,
    pub config_body_limit: usize,
    /// The longest a request that is not a stream may take to answer: the
    /// service's longest wait, and a margin.
    pub request_timeout: Duration,
    /// Event streams one principal may hold open.
    pub streams_per_principal: usize,
    /// How long shutdown waits for open connections to finish.
    pub grace: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            operator_connections: 16,
            agent_connections: 32,
            head_timeout: Duration::from_secs(5),
            max_head_bytes: 16 * 1024,
            body_timeout: Duration::from_secs(10),
            body_limit: 16 * 1024,
            config_body_limit: 1024 * 1024,
            request_timeout: Duration::from_secs(35),
            streams_per_principal: 4,
            grace: Duration::from_secs(5),
        }
    }
}

/// Where the Agent endpoint is and whom it admits. The settings file produces
/// this; the endpoint's code takes it as given.
#[derive(Debug, Clone)]
pub struct AgentEndpointConfig {
    /// The directory the socket is in.
    pub directory: PathBuf,
    /// The uids admitted. A connection from any other is closed unread.
    pub uids: Vec<u32>,
    /// The socket's mode. The directory and the peer's uid decide who gets in; the
    /// mode only has to let the admitted account connect.
    pub mode: u32,
}

impl AgentEndpointConfig {
    pub const SOCKET_NAME: &'static str = "agent.sock";

    pub fn socket(&self) -> PathBuf {
        self.directory.join(Self::SOCKET_NAME)
    }
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub paths: EndpointPaths,
    pub agent: Option<AgentEndpointConfig>,
    pub limits: Limits,
}

/// Why the server did not start.
#[derive(Debug)]
pub enum StartError {
    /// Another server holds the lock.
    AlreadyRunning,
    SocketPathTooLong {
        len: usize,
        max: usize,
    },
    Directory {
        path: PathBuf,
        why: String,
    },
    Bind {
        path: PathBuf,
        why: String,
    },
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning => f.write_str("another Control API server is already running"),
            Self::SocketPathTooLong { len, max } => write!(
                f,
                "a socket path is {len} bytes long and the longest this system allows is {max}; \
                 use a shorter data directory"
            ),
            Self::Directory { path, why } => write!(f, "{}: {why}", path.display()),
            Self::Bind { path, why } => write!(f, "cannot bind {}: {why}", path.display()),
        }
    }
}

impl std::error::Error for StartError {}

pub struct Server;

impl Server {
    /// Start serving. `tokens` holds the Operator's token, so it is required;
    /// the Agent endpoint is created only when it is configured and the store can
    /// issue tokens.
    pub async fn start(
        service: Arc<ControlService>,
        tokens: SharedTokenStore,
        config: ServerConfig,
    ) -> Result<RunningServer, StartError> {
        let paths = &config.paths;
        ensure_private_directory(&paths.run_directory).map_err(|why| StartError::Directory {
            path: paths.run_directory.clone(),
            why,
        })?;
        let lock = take_lock(&paths.lock_file)?;
        // The lock file was created by this process, so its owner is this uid.
        let server_uid = std::fs::metadata(&paths.lock_file)
            .map_err(|error| StartError::Directory {
                path: paths.lock_file.clone(),
                why: error.to_string(),
            })?
            .uid();

        check_path_length(&paths.operator_socket)?;
        let mut agent_off = Some("no agent endpoint is configured".to_owned());
        let mut agent = None;
        if let Some(configured) = &config.agent {
            if let Some(fault) = tokens.fault() {
                warn!(%fault, "the Agent endpoint is off: the token store cannot be used");
                agent_off = Some("the token store cannot be used".to_owned());
            } else if configured.uids.is_empty() {
                agent_off = Some("the Agent endpoint admits no uid".to_owned());
            } else {
                let socket = configured.socket();
                check_path_length(&socket)?;
                agent_off = None;
                agent = Some((configured.clone(), socket));
            }
        }

        // Only now is anything removed or created at a socket path.
        let mut created = Vec::new();
        let bound = (|| -> Result<_, StartError> {
            remove_stale(&paths.operator_socket, server_uid)?;
            let operator = bind(&paths.operator_socket, 0o600)?;
            created.push(paths.operator_socket.clone());
            let agent_listener = match &agent {
                Some((configured, socket)) => {
                    remove_stale(socket, server_uid)?;
                    let listener = bind(socket, configured.mode)?;
                    created.push(socket.clone());
                    Some(listener)
                }
                None => None,
            };
            Ok((operator, agent_listener))
        })();
        let (operator_listener, agent_listener) = match bound {
            Ok(listeners) => listeners,
            Err(error) => {
                for path in &created {
                    let _ = std::fs::remove_file(path);
                }
                return Err(error);
            }
        };

        let server_health = ServerHealthDto {
            agent_endpoint: match &agent_off {
                None => AgentEndpointDto::On,
                Some(reason) => AgentEndpointDto::Off {
                    reason: reason.clone(),
                },
            },
            token_store: if tokens.fault().is_some() {
                TokenStoreDto::Unavailable
            } else {
                TokenStoreDto::Ok
            },
        };
        let (shutdown, shutdown_rx) = watch::channel(false);
        let limits = Arc::new(config.limits.clone());
        let mut tasks = Vec::new();
        let mut connections = Vec::new();

        let mut start = |kind, listener: UnixListener, admitted: Vec<u32>, capacity: usize| {
            let semaphore = Arc::new(Semaphore::new(capacity));
            connections.push((Arc::clone(&semaphore), capacity));
            let context = Arc::new(EndpointContext {
                kind,
                service: Arc::clone(&service),
                tokens: Arc::clone(&tokens),
                limits: Arc::clone(&limits),
                admitted_uids: admitted,
                server_health: server_health.clone(),
                shutdown: shutdown_rx.clone(),
            });
            tasks.push(endpoint::spawn(listener, context, semaphore));
        };
        start(
            EndpointKind::Operator,
            operator_listener,
            vec![server_uid],
            limits.operator_connections,
        );
        if let (Some(listener), Some((configured, _))) = (agent_listener, &agent) {
            start(
                EndpointKind::Agent,
                listener,
                configured.uids.clone(),
                limits.agent_connections,
            );
        }
        info!(socket = %paths.operator_socket.display(), "the Control API server is serving");
        Ok(RunningServer {
            operator_socket: paths.operator_socket.clone(),
            agent_socket: agent.map(|(_, socket)| socket),
            service,
            shutdown,
            tasks,
            connections,
            lock: Some(lock),
            grace: limits.grace,
            server_health,
        })
    }
}

/// A server that is serving. Shut it down with [`RunningServer::shutdown`]; if it
/// is dropped instead, its sockets are removed and its connections told to stop.
pub struct RunningServer {
    operator_socket: PathBuf,
    agent_socket: Option<PathBuf>,
    service: Arc<ControlService>,
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
    connections: Vec<(Arc<Semaphore>, usize)>,
    lock: Option<std::fs::File>,
    grace: Duration,
    server_health: ServerHealthDto,
}

impl RunningServer {
    pub fn operator_socket(&self) -> &Path {
        &self.operator_socket
    }

    pub fn agent_socket(&self) -> Option<&Path> {
        self.agent_socket.as_deref()
    }

    /// What the Operator's health response says about the server itself.
    pub fn server_health(&self) -> &ServerHealthDto {
        &self.server_health
    }

    /// Stop accepting, let open connections finish (within the grace period), close
    /// the control service and with it every receiver session, remove the sockets,
    /// and release the lock.
    pub async fn shutdown(mut self) {
        let _ = self.shutdown.send(true);
        for task in std::mem::take(&mut self.tasks) {
            let _ = task.await;
        }
        for (semaphore, capacity) in &self.connections {
            let all = u32::try_from(*capacity).unwrap_or(u32::MAX);
            // Every connection returns its permit when it ends.
            let _ = tokio::time::timeout(self.grace, semaphore.acquire_many(all)).await;
        }
        self.service.shutdown().await;
        self.remove_sockets();
        self.lock.take();
    }

    fn remove_sockets(&self) {
        let _ = std::fs::remove_file(&self.operator_socket);
        if let Some(socket) = &self.agent_socket {
            let _ = std::fs::remove_file(socket);
        }
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        if self.lock.is_some() {
            self.remove_sockets();
        }
    }
}

fn take_lock(path: &Path) -> Result<std::fs::File, StartError> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map_err(|error| StartError::Directory {
            path: path.to_owned(),
            why: error.to_string(),
        })?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(StartError::AlreadyRunning),
        Err(std::fs::TryLockError::Error(error)) => Err(StartError::Directory {
            path: path.to_owned(),
            why: error.to_string(),
        }),
    }
}

fn check_path_length(path: &Path) -> Result<(), StartError> {
    let len = path.as_os_str().as_bytes().len();
    if len >= SUN_PATH_LEN {
        return Err(StartError::SocketPathTooLong {
            len,
            max: SUN_PATH_LEN - 1,
        });
    }
    Ok(())
}

/// Remove what a killed server left at `path`, and nothing else. Only a socket
/// owned by this server's uid is removed; a file, a link, or someone else's socket
/// is an error and is left alone, because the unlink must not follow a link
/// planted in a directory other users can write to.
fn remove_stale(path: &Path, server_uid: u32) -> Result<(), StartError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_socket() && metadata.uid() == server_uid {
                std::fs::remove_file(path).map_err(|error| StartError::Directory {
                    path: path.to_owned(),
                    why: error.to_string(),
                })
            } else {
                Err(StartError::Directory {
                    path: path.to_owned(),
                    why: "something other than this server's socket is at the socket path; \
                          it was left alone"
                        .into(),
                })
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(StartError::Directory {
            path: path.to_owned(),
            why: error.to_string(),
        }),
    }
}

fn bind(path: &Path, mode: u32) -> Result<UnixListener, StartError> {
    let fail = |why: String| StartError::Bind {
        path: path.to_owned(),
        why,
    };
    let listener = UnixListener::bind(path).map_err(|error| fail(error.to_string()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|error| fail(error.to_string()))?;
    Ok(listener)
}
