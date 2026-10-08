//! `server.yaml`: absent is valid, an unknown key or a half-made Agent endpoint is
//! refused by name, and the sample file loads.

mod support;

use denon_avr_api_server::settings::{
    self, AgentEndpointSettings, Settings, DEFAULT_AGENT_DIRECTORY,
};
use std::path::PathBuf;
use support::TempDir;

fn parsed(text: &str) -> Result<Settings, String> {
    settings::parse(text)
}

fn error_of(text: &str) -> String {
    parsed(text).expect_err("the file is refused")
}

#[test]
fn the_settings_file_may_be_absent() {
    let root = TempDir::new("set");
    let loaded = settings::load(&root.0.join("server.yaml")).unwrap();
    assert_eq!(loaded, Settings::default());
    assert!(loaded.agent_config().is_none());
}

#[test]
fn an_empty_file_or_one_of_comments_is_the_same_as_none() {
    for text in ["", "\n", "# nothing here\n", "---\n"] {
        assert_eq!(parsed(text).unwrap(), Settings::default(), "{text:?}");
    }
}

#[test]
fn the_agent_endpoint_takes_its_directory_and_the_uids_it_admits() {
    let loaded = parsed("agent_endpoint:\n  directory: /srv/agent\n  uids: [503, 504]\n").unwrap();
    assert_eq!(
        loaded.agent_endpoint,
        Some(AgentEndpointSettings {
            directory: PathBuf::from("/srv/agent"),
            uids: vec![503, 504],
        })
    );
    let config = loaded.agent_config().unwrap();
    assert_eq!(config.directory, PathBuf::from("/srv/agent"));
    assert_eq!(config.uids, vec![503, 504]);
    assert_eq!(config.mode, 0o600, "the socket stays private to its owner");
    assert_eq!(config.socket(), PathBuf::from("/srv/agent/agent.sock"));
}

#[test]
fn the_default_directory_is_used_when_none_is_given() {
    let loaded = parsed("agent_endpoint:\n  uids: [503]\n").unwrap();
    assert_eq!(
        loaded.agent_config().unwrap().directory,
        PathBuf::from(DEFAULT_AGENT_DIRECTORY)
    );
    assert_eq!(DEFAULT_AGENT_DIRECTORY, "/Users/Shared/Denon AVR Remote");
}

#[test]
fn unknown_settings_keys_are_refused() {
    // A typo at the top, a typo inside the endpoint, and the socket's mode, which is
    // not a setting: each is refused and named.
    for (text, key) in [
        ("agent_endpont:\n  uids: [503]\n", "agent_endpont"),
        ("agent_endpoint:\n  uid: [503]\n", "uid"),
        ("agent_endpoint:\n  uids: [503]\n  mode: 0666\n", "mode"),
        ("listen: 127.0.0.1\n", "listen"),
    ] {
        let message = error_of(text);
        assert!(message.contains(key), "{key} is not named in {message:?}");
    }
}

#[test]
fn an_agent_endpoint_with_no_uids_is_refused() {
    for text in [
        "agent_endpoint:\n  uids: []\n",
        "agent_endpoint:\n  directory: /srv/agent\n",
        "agent_endpoint: {}\n",
        "agent_endpoint:\n",
    ] {
        let message = error_of(text);
        assert!(message.contains("uids"), "{text:?} gave {message:?}");
    }
}

#[test]
fn a_directory_that_is_not_an_absolute_path_is_refused() {
    for directory in ["agent", "./agent", "~/agent", "''"] {
        let text = format!("agent_endpoint:\n  directory: {directory}\n  uids: [503]\n");
        let message = error_of(&text);
        assert!(message.contains("directory"), "{directory}: {message:?}");
    }
}

#[test]
fn a_uid_that_is_not_a_whole_number_is_refused() {
    for uids in ["[-1]", "[root]", "[1.5]", "503", "[99999999999]"] {
        let text = format!("agent_endpoint:\n  uids: {uids}\n");
        assert!(parsed(&text).is_err(), "{uids} was accepted");
    }
}

#[test]
fn an_error_does_not_repeat_what_the_file_said() {
    // A value is the owner's text and a path is the machine's; the message names the
    // fault and where it is.
    let message = error_of("agent_endpoint:\n  directory: relative/secret-place\n  uids: [503]\n");
    assert!(!message.contains("secret-place"), "{message}");
}

#[test]
fn the_sample_settings_file_loads() {
    let sample = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples/server.yaml");
    let loaded = settings::load(&sample).expect("the sample loads");
    let config = loaded
        .agent_config()
        .expect("the sample configures an Agent endpoint");
    assert_eq!(config.directory, PathBuf::from(DEFAULT_AGENT_DIRECTORY));
    assert!(!config.uids.is_empty());
}

#[test]
fn a_file_too_large_to_be_settings_or_one_that_cannot_be_read_is_an_error() {
    let root = TempDir::new("set");
    let big = root.0.join("big.yaml");
    std::fs::write(&big, vec![b' '; 70 * 1024]).unwrap();
    assert!(settings::load(&big).is_err());
    // A directory where the file should be.
    let directory = root.0.join("server.yaml");
    std::fs::create_dir(&directory).unwrap();
    assert!(settings::load(&directory).is_err());
}
