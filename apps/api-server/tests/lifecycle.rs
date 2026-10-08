//! Starting, being the only server, leaving the socket directory as it should be,
//! and the limits on a socket path.

mod support;

use denon_avr_api_contract::EndpointPaths;
use denon_avr_api_server::endpoint::admits;
use denon_avr_api_server::{Limits, Server, ServerConfig, StartError};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use support::*;

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn config(paths: &EndpointPaths) -> ServerConfig {
    ServerConfig {
        paths: paths.clone(),
        agent: None,
        limits: Limits::default(),
    }
}

#[tokio::test]
async fn a_second_server_refuses_with_already_running() {
    let first = Fixture::start(Options::default()).await;
    let second = Server::start(
        first.service.clone(),
        first.tokens.clone(),
        config(&first.paths),
    )
    .await;
    assert!(
        matches!(second, Err(StartError::AlreadyRunning)),
        "{:?}",
        second.err()
    );

    // The first is untouched: its socket is where it was and it answers.
    let token = first.operator_token();
    assert!(first.operator_socket().exists());
    assert_eq!(
        get(first.operator_socket(), "/v1/health", &token)
            .await
            .status,
        200
    );
    first.shutdown().await;
}

#[tokio::test]
async fn a_stale_socket_is_not_removed_while_another_server_holds_the_lock() {
    let first = Fixture::start(Options::default()).await;
    let socket = first.operator_socket().to_owned();
    let inode = std::fs::metadata(&socket).unwrap().len();
    for _ in 0..3 {
        let refused = Server::start(
            first.service.clone(),
            first.tokens.clone(),
            config(&first.paths),
        )
        .await;
        assert!(matches!(refused, Err(StartError::AlreadyRunning)));
        assert!(socket.exists(), "the running server's socket was removed");
    }
    assert_eq!(std::fs::metadata(&socket).unwrap().len(), inode);
    let token = first.operator_token();
    assert_eq!(get(&socket, "/v1/health", &token).await.status, 200);
    first.shutdown().await;
}

#[tokio::test]
async fn a_socket_left_by_a_killed_server_is_reclaimed() {
    let root = Arc::new(TempDir::new("kill"));
    let socket = EndpointPaths::under(&root.0).operator_socket;
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_denon-avr-api-server"))
        .arg("--data-dir")
        .arg(&root.0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the server binary starts");
    wait_for("the server's socket", || socket.exists()).await;

    // SIGKILL: no shutdown runs, so the socket stays.
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(socket.exists(), "a killed server leaves its socket behind");

    // The next start reclaims it.
    let fixture = Fixture::start_in(
        root,
        Options {
            agent_endpoint: false,
            ..Options::default()
        },
    )
    .await
    .expect("a stale socket is reclaimed");
    let token = fixture.operator_token();
    assert_eq!(
        get(fixture.operator_socket(), "/v1/health", &token)
            .await
            .status,
        200
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_stale_path_that_is_not_our_socket_is_not_removed() {
    // A regular file.
    let root = Arc::new(TempDir::new("file"));
    let paths = EndpointPaths::under(&root.0);
    std::fs::create_dir_all(&paths.run_directory).unwrap();
    std::fs::set_permissions(&paths.run_directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(&paths.operator_socket, "precious").unwrap();
    let error = Fixture::start_in(
        Arc::clone(&root),
        Options {
            agent_endpoint: false,
            ..Options::default()
        },
    )
    .await
    .err()
    .expect("a file at the socket path stops the start");
    assert!(matches!(error, StartError::Directory { .. }), "{error}");
    assert_eq!(
        std::fs::read_to_string(&paths.operator_socket).unwrap(),
        "precious"
    );

    // A symlink, to somewhere that matters.
    let root = Arc::new(TempDir::new("link"));
    let paths = EndpointPaths::under(&root.0);
    let target = root.0.join("target.txt");
    std::fs::write(&target, "precious").unwrap();
    std::fs::create_dir_all(&paths.run_directory).unwrap();
    std::fs::set_permissions(&paths.run_directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(&target, &paths.operator_socket).unwrap();
    let error = Fixture::start_in(
        Arc::clone(&root),
        Options {
            agent_endpoint: false,
            ..Options::default()
        },
    )
    .await
    .err()
    .expect("a link at the socket path stops the start");
    assert!(matches!(error, StartError::Directory { .. }), "{error}");
    assert!(std::fs::symlink_metadata(&paths.operator_socket)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "precious");
}

/// A data directory under `/tmp` whose operator socket path is `length` bytes.
fn directory_for_a_socket_path_of(length: usize) -> Arc<TempDir> {
    let base = TempDir::new("len");
    let tail = "/run/operator.sock".len();
    let current = base.0.as_os_str().len() + tail;
    assert!(
        current <= length,
        "the base path is already long: {current}"
    );
    let padded = Path::new(&format!(
        "{}{}",
        base.0.display(),
        "x".repeat(length - current)
    ))
    .to_path_buf();
    std::fs::remove_dir_all(&base.0).unwrap();
    std::fs::create_dir_all(&padded).unwrap();
    Arc::new(TempDir(padded))
}

#[tokio::test]
async fn a_path_over_the_limit_fails_with_its_length_and_the_limit() {
    let max = if cfg!(target_os = "macos") { 103 } else { 107 };
    let root = directory_for_a_socket_path_of(max + 1);
    assert_eq!(
        EndpointPaths::under(&root.0)
            .operator_socket
            .as_os_str()
            .len(),
        max + 1
    );
    let error = Fixture::start_in(
        Arc::clone(&root),
        Options {
            agent_endpoint: false,
            ..Options::default()
        },
    )
    .await
    .err()
    .expect("a path one byte too long is refused");
    match error {
        StartError::SocketPathTooLong { len, max: limit } => {
            assert_eq!((len, limit), (max + 1, max));
            let text = StartError::SocketPathTooLong { len, max: limit }.to_string();
            assert!(
                text.contains(&len.to_string()) && text.contains(&limit.to_string()),
                "{text}"
            );
        }
        other => panic!("{other}"),
    }
}

#[tokio::test]
async fn a_path_exactly_at_the_limit_binds() {
    let max = if cfg!(target_os = "macos") { 103 } else { 107 };
    let root = directory_for_a_socket_path_of(max);
    let fixture = Fixture::start_in(
        root,
        Options {
            agent_endpoint: false,
            ..Options::default()
        },
    )
    .await
    .expect("a path at the limit binds");
    assert_eq!(fixture.operator_socket().as_os_str().len(), max);
    let token = fixture.operator_token();
    assert_eq!(
        get(fixture.operator_socket(), "/v1/health", &token)
            .await
            .status,
        200
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn the_directories_are_created_0700_and_the_data_directory_is_left_alone() {
    let root = Arc::new(TempDir::new("modes"));
    std::fs::set_permissions(&root.0, std::fs::Permissions::from_mode(0o755)).unwrap();
    let fixture = Fixture::start_in(
        root,
        Options {
            agent_endpoint: false,
            ..Options::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        mode(&fixture.root.0),
        0o755,
        "the data directory is not this server's to change"
    );
    assert_eq!(mode(&fixture.paths.run_directory), 0o700);
    assert_eq!(mode(&fixture.paths.credentials_directory), 0o700);
    assert_eq!(mode(&fixture.paths.lock_file), 0o600);
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_wider_run_or_credentials_directory_is_an_error() {
    let root = TempDir::new("wide");
    let paths = EndpointPaths::under(&root.0);
    std::fs::create_dir_all(&paths.run_directory).unwrap();
    std::fs::set_permissions(&paths.run_directory, std::fs::Permissions::from_mode(0o755)).unwrap();
    let tokens = Arc::new(
        denon_avr_infrastructure::FileTokenStore::open(&paths.credentials_directory).unwrap(),
    );
    let service = Arc::new(denon_avr_application::ControlService::new(
        FakeConnector::new(-35.0),
        Arc::new(FakeConfig(Default::default())),
        Arc::new(FakeDiscovery::new(Vec::new())),
        Default::default(),
    ));
    let error = Server::start(service, tokens, config(&paths))
        .await
        .err()
        .unwrap();
    assert!(matches!(error, StartError::Directory { .. }), "{error}");
    assert!(error.to_string().contains("755"), "{error}");

    // The credentials directory is the token store's to check, and it does.
    let root = TempDir::new("wide-credentials");
    let paths = EndpointPaths::under(&root.0);
    std::fs::create_dir_all(&paths.credentials_directory).unwrap();
    std::fs::set_permissions(
        &paths.credentials_directory,
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert!(denon_avr_infrastructure::FileTokenStore::open(&paths.credentials_directory).is_err());
}

#[tokio::test]
async fn the_operator_socket_is_0600() {
    let fixture = Fixture::start(Options::default()).await;
    assert_eq!(mode(fixture.operator_socket()), 0o600);
    assert_eq!(mode(fixture.agent_socket()), 0o600);
    fixture.shutdown().await;
}

#[test]
fn admission_compares_the_peer_uid_with_the_endpoints_list() {
    assert!(admits(&[501], Some(501)));
    assert!(admits(&[501, 502], Some(502)));
    assert!(!admits(&[501], Some(502)));
    assert!(!admits(&[501], Some(0)), "root is not special");
    assert!(!admits(&[], Some(501)));
    assert!(
        !admits(&[501], None),
        "a peer whose uid is unknown is not admitted"
    );
}

#[test]
fn default_limits_are_the_documented_ones() {
    let limits = Limits::default();
    assert_eq!(limits.operator_connections, 16);
    assert_eq!(limits.agent_connections, 32);
    assert_eq!(limits.head_timeout, Duration::from_secs(5));
    assert_eq!(limits.max_head_bytes, 16 * 1024);
    assert_eq!(limits.body_timeout, Duration::from_secs(10));
    assert_eq!(limits.body_limit, 16 * 1024);
    assert_eq!(limits.config_body_limit, 1024 * 1024);
    assert_eq!(limits.request_timeout, Duration::from_secs(35));
    assert_eq!(limits.streams_per_principal, 4);
}

// ---- Shutdown, with the real binary ----

/// The binary, running on a data directory of its own.
struct Running {
    child: std::process::Child,
    root: Arc<TempDir>,
    socket: std::path::PathBuf,
}

impl Running {
    async fn start(parent_pipe: bool) -> Self {
        let root = Arc::new(TempDir::new("bin"));
        let paths = EndpointPaths::under(&root.0);
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_denon-avr-api-server"));
        command.arg("--data-dir").arg(&root.0);
        if parent_pipe {
            command.arg("--exit-with-parent");
        }
        let child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the server binary starts");
        let socket = paths.operator_socket;
        wait_for("the server's socket", || socket.exists()).await;
        Self {
            child,
            root,
            socket,
        }
    }

    fn token(&self) -> String {
        std::fs::read_to_string(EndpointPaths::under(&self.root.0).operator_token)
            .unwrap()
            .trim()
            .to_owned()
    }

    fn signal(&self, name: &str) {
        let status = std::process::Command::new("kill")
            .arg(format!("-{name}"))
            .arg(self.child.id().to_string())
            .status()
            .expect("kill runs");
        assert!(status.success());
    }

    /// Wait for the process to exit.
    async fn exit(&mut self, within: Duration) -> std::process::ExitStatus {
        let deadline = std::time::Instant::now() + within;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the server did not exit within {within:?}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Whether the lock is free, which it is only when no server holds it.
    fn lock_is_free(&self) -> bool {
        let lock = EndpointPaths::under(&self.root.0).lock_file;
        std::fs::File::open(lock)
            .map(|file| file.try_lock().is_ok())
            .unwrap_or(true)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test]
async fn end_of_input_in_parent_pipe_mode_shuts_down_removes_the_sockets_and_frees_the_lock() {
    let mut server = Running::start(true).await;
    assert!(!server.lock_is_free(), "a running server holds the lock");

    // The parent goes away: the pipe it held closes.
    drop(server.child.stdin.take());
    let status = server.exit(Duration::from_secs(10)).await;
    assert!(status.success(), "{status:?}");
    assert!(!server.socket.exists(), "the socket is removed");
    assert!(server.lock_is_free(), "the lock is released");
}

#[tokio::test]
async fn a_standalone_server_ignores_standard_input() {
    let mut server = Running::start(false).await;
    drop(server.child.stdin.take());
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    assert!(
        server.child.try_wait().unwrap().is_none(),
        "the server stopped when its standard input closed"
    );
    let token = server.token();
    assert_eq!(get(&server.socket, "/v1/health", &token).await.status, 200);

    server.signal("TERM");
    assert!(server.exit(Duration::from_secs(10)).await.success());
}

#[tokio::test]
async fn sigterm_and_sigint_shut_down_the_same_way() {
    for name in ["TERM", "INT"] {
        let mut server = Running::start(false).await;
        server.signal(name);
        let status = server.exit(Duration::from_secs(10)).await;
        assert!(status.success(), "SIG{name}: {status:?}");
        assert!(!server.socket.exists(), "SIG{name}: the socket is removed");
        assert!(server.lock_is_free(), "SIG{name}: the lock is released");
    }
}

#[tokio::test]
async fn a_second_signal_exits_at_once() {
    use tokio::io::AsyncWriteExt;
    let mut server = Running::start(false).await;
    let token = server.token();

    // A request whose body never arrives keeps its connection busy, so the first
    // signal's shutdown has something to wait for.
    let mut busy = tokio::net::UnixStream::connect(&server.socket)
        .await
        .unwrap();
    let head = format!(
        "POST /v1/receivers/living-room/operations HTTP/1.1\r\nHost: dar\r\n\
         Authorization: Bearer {token}\r\nContent-Type: application/json\r\n\
         Content-Length: 100\r\n\r\n{{"
    );
    busy.write_all(head.as_bytes()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    server.signal("TERM");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        server.child.try_wait().unwrap().is_none(),
        "the first signal waits for the busy connection"
    );

    let started = std::time::Instant::now();
    server.signal("TERM");
    let status = server.exit(Duration::from_secs(3)).await;
    assert_eq!(status.code(), Some(128 + 15), "{status:?}");
    assert!(started.elapsed() < Duration::from_secs(2));
    drop(busy);
}

#[tokio::test]
async fn a_shutdown_while_a_client_streams_ends_the_stream_with_an_end_event() {
    let mut server = Running::start(false).await;
    let token = server.token();
    let mut stream = Stream::open(&server.socket, "/v1/operations/events", &token)
        .await
        .map_err(|reply| reply.text())
        .expect("the stream opens");

    server.signal("TERM");
    let end = stream.last_event().await.expect("an end event");
    assert_eq!(end_reason(&end), "shutdown");
    assert!(server.exit(Duration::from_secs(10)).await.success());
}

#[tokio::test]
async fn shutdown_waits_for_operations_in_flight_before_closing_sessions() {
    let fixture = Fixture::start(Options::default()).await;
    let agent = fixture.agent_token("openclaw").await;
    let socket = fixture.agent_socket().to_owned();
    fixture.connector.hold_operations();
    let body = br#"{"intent":{"kind":"mute","value":"on"}}"#;
    let made = request(
        &socket,
        "POST",
        "/v1/receivers/living-room/operations",
        Some(&agent),
        Some(body),
    )
    .await;
    assert_eq!(made.status, 200, "{}", made.text());
    let id = made.json()["id"].as_u64().unwrap();
    let operation = format!("/v1/operations/{id}");
    for _ in 0..200 {
        if get(&socket, &operation, &agent).await.json()["status"] == "in_session" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let session = fixture.connector.latest();
    let connector = fixture.connector.clone();

    let shutdown = tokio::spawn(fixture.shutdown());
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(
        !shutdown.is_finished(),
        "shutdown did not wait for the operation"
    );
    assert!(
        !session.closed.load(std::sync::atomic::Ordering::SeqCst),
        "the session was closed under an operation in flight"
    );

    connector.release_operations();
    tokio::time::timeout(Duration::from_secs(10), shutdown)
        .await
        .expect("shutdown finishes once the operation has")
        .unwrap();
    assert!(session.closed.load(std::sync::atomic::Ordering::SeqCst));
}
