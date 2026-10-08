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
