//! The Agent endpoint's directory: a real directory, the server's own, and not
//! writable by anyone else, or the endpoint is not created. The Operator endpoint
//! serves either way, and its health response says why the Agent endpoint is off.

mod support;

use denon_avr_api_server::AgentEndpointConfig;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use support::*;

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn config(root: &TempDir, directory: PathBuf) -> AgentEndpointConfig {
    AgentEndpointConfig {
        directory,
        uids: vec![std::fs::metadata(&root.0).unwrap().uid()],
        mode: 0o600,
    }
}

async fn started(agent: impl FnOnce(&TempDir) -> AgentEndpointConfig) -> Fixture {
    let root = Arc::new(TempDir::new("dir"));
    let agent = agent(&root);
    Fixture::start_in(
        root,
        Options {
            agent_config: Some(agent),
            ..Options::default()
        },
    )
    .await
    .expect("the server starts whatever the Agent endpoint's directory is")
}

/// The Operator's view of the Agent endpoint: its state, and why when it is off.
async fn agent_endpoint(fixture: &Fixture) -> (String, String) {
    let reply = get(
        fixture.operator_socket(),
        "/v1/health",
        &fixture.operator_token(),
    )
    .await;
    assert_eq!(
        reply.status,
        200,
        "the Operator endpoint serves: {}",
        reply.text()
    );
    let health = reply.json();
    let endpoint = &health["server"]["agent_endpoint"];
    (
        endpoint["state"].as_str().unwrap().to_owned(),
        endpoint["reason"].as_str().unwrap_or("").to_owned(),
    )
}

#[tokio::test]
async fn the_agent_endpoint_is_not_created_without_settings() {
    let fixture = Fixture::start(Options {
        agent_endpoint: false,
        ..Options::default()
    })
    .await;
    assert!(fixture.server.as_ref().unwrap().agent_socket().is_none());
    let (state, reason) = agent_endpoint(&fixture).await;
    assert_eq!(state, "off");
    assert_eq!(reason, "no agent endpoint is configured");
    fixture.shutdown().await;
}

#[tokio::test]
async fn the_agent_endpoint_refuses_an_unsafe_directory() {
    type Make = fn(&TempDir) -> PathBuf;
    let cases: [(&str, &str, Make); 5] = [
        ("a link to a real directory", "is a link", |root| {
            let real = root.0.join("real");
            std::fs::create_dir(&real).unwrap();
            std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
            let link = root.0.join("pointer");
            std::os::unix::fs::symlink(&real, &link).unwrap();
            link
        }),
        ("writable by its group", "writable", |root| {
            let directory = root.0.join("group");
            std::fs::create_dir(&directory).unwrap();
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o770)).unwrap();
            directory
        }),
        ("writable by everyone", "writable", |root| {
            let directory = root.0.join("world");
            std::fs::create_dir(&directory).unwrap();
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o777)).unwrap();
            directory
        }),
        (
            "a file where the directory should be",
            "not a directory",
            |root| {
                let file = root.0.join("file");
                std::fs::write(&file, b"x").unwrap();
                file
            },
        ),
        ("a relative path", "absolute", |_| PathBuf::from("agent")),
    ];
    for (what, expected, make) in cases {
        let mut unsafe_directory = PathBuf::new();
        let fixture = started(|root| {
            unsafe_directory = make(root);
            config(root, unsafe_directory.clone())
        })
        .await;
        assert!(
            fixture.server.as_ref().unwrap().agent_socket().is_none(),
            "{what}: an Agent endpoint was created"
        );
        let (state, reason) = agent_endpoint(&fixture).await;
        assert_eq!(state, "off", "{what}");
        assert!(reason.contains(expected), "{what}: {reason:?}");
        // Nothing was put in the directory that was refused, or behind the link.
        for place in [
            unsafe_directory.clone(),
            unsafe_directory.join("agent.sock"),
        ] {
            assert!(
                std::fs::symlink_metadata(&place)
                    .map(|m| !m.file_type().is_socket_like())
                    .unwrap_or(true),
                "{what}: a socket was made at {}",
                place.display()
            );
        }
        fixture.shutdown().await;
    }
}

trait SocketLike {
    fn is_socket_like(&self) -> bool;
}

impl SocketLike for std::fs::FileType {
    fn is_socket_like(&self) -> bool {
        use std::os::unix::fs::FileTypeExt;
        self.is_socket()
    }
}

#[tokio::test]
async fn a_directory_that_is_wider_than_0700_but_not_writable_is_accepted() {
    // The rule is "nobody else can write to it", not "exactly 0700".
    let fixture = started(|root| {
        let directory = root.0.join("shared");
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();
        config(root, directory)
    })
    .await;
    assert_eq!(agent_endpoint(&fixture).await.0, "on");
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_missing_directory_is_created_0700_and_its_socket_is_0600() {
    let mut directory = PathBuf::new();
    let fixture = started(|root| {
        directory = root.0.join("not-yet").join("agent");
        config(root, directory.clone())
    })
    .await;
    assert_eq!(mode(&directory), 0o700);
    assert_eq!(
        mode(directory.parent().unwrap()),
        0o700,
        "parents are private too"
    );
    assert_eq!(mode(&directory.join("agent.sock")), 0o600);
    assert_eq!(agent_endpoint(&fixture).await.0, "on");

    // And it serves: an Agent token is accepted there.
    let token = fixture.agent_token("openclaw").await;
    let reply = get(fixture.agent_socket(), "/v1/health", &token).await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    fixture.shutdown().await;
}

#[tokio::test]
async fn an_agent_endpoint_with_no_uids_is_refused() {
    let fixture = started(|root| AgentEndpointConfig {
        uids: Vec::new(),
        ..config(root, root.0.join("agent"))
    })
    .await;
    let (state, reason) = agent_endpoint(&fixture).await;
    assert_eq!(state, "off");
    assert!(reason.contains("no uid"), "{reason:?}");
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_socket_left_by_a_killed_server_is_reclaimed_and_anything_else_at_its_path_is_not() {
    // A stale socket in the directory is removed and rebound.
    let fixture = started(|root| {
        let directory = root.0.join("stale");
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        drop(std::os::unix::net::UnixListener::bind(directory.join("agent.sock")).unwrap());
        config(root, directory)
    })
    .await;
    assert_eq!(agent_endpoint(&fixture).await.0, "on");
    fixture.shutdown().await;

    // A file at the socket's path is left alone, and the endpoint is off.
    let mut path = PathBuf::new();
    let fixture = started(|root| {
        let directory = root.0.join("occupied");
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        path = directory.join("agent.sock");
        std::fs::write(&path, b"mine").unwrap();
        config(root, directory)
    })
    .await;
    let (state, reason) = agent_endpoint(&fixture).await;
    assert_eq!(state, "off");
    assert!(reason.contains("left alone"), "{reason:?}");
    assert_eq!(std::fs::read(&path).unwrap(), b"mine");
    fixture.shutdown().await;
}
