//! Tests of the YAML configuration adapter through both repositories, against
//! real files in a scratch directory.

use super::schema::LegacyConfigFile;
use super::*;
use denon_avr_domain::{ReceiverIdentity, SoundModeFavorite};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// A scratch directory per call, so backups beside the file are visible.
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        // The clock alone is not unique: tests run in parallel, several
        // share a name, and a coarse clock gives two of them the same
        // directory, so one deletes the other's file.
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "denon-config-{}-{stamp}-{unique}-{name}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }
    fn file(&self) -> PathBuf {
        self.dir.join("denon-avr-remote.yaml")
    }
    fn backups(&self) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(&self.dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".v3.bak"))
            .collect();
        names.sort();
        names
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

const SINGLE_RECEIVER: &str = "receiver:\n  host: 192.0.2.10\n  model: AVR-X3800H\n  friendly_name: living-room\nsound_mode_favorites:\n  living-room:\n    - DTS NEURAL:X\n";

fn two_receivers() -> ConfiguredReceivers {
    ConfiguredReceivers {
        current: Some("living-room".into()),
        receivers: BTreeMap::from([
            (
                "living-room".into(),
                ReceiverIdentity {
                    host: "192.0.2.10".into(),
                    model: Some("AVR-X3800H".into()),
                    friendly_name: Some("Living Room".into()),
                },
            ),
            ("den".into(), ReceiverIdentity::ad_hoc("192.0.2.11")),
        ]),
        sound_mode_favorites: BTreeMap::from([(
            "living-room".into(),
            BTreeSet::from([
                SoundModeFavorite::new("DTS NEURAL:X").unwrap(),
                SoundModeFavorite::new("DOLBY SURROUND").unwrap(),
            ]),
        )]),
    }
}

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

#[test]
fn the_multi_receiver_schema_round_trips_through_both_repositories() {
    let scratch = Scratch::new("round-trip");
    let repository = YamlConfigRepository::new(scratch.file());
    let config = two_receivers();

    ConfigRepository::save(&repository, &config).unwrap();
    let stored = fs::read_to_string(scratch.file()).unwrap();
    assert!(stored.starts_with("version: 2\n"), "{stored}");
    assert!(stored.contains("current: living-room"));
    assert!(stored.contains("den:"));
    assert!(stored.contains("DOLBY SURROUND") && stored.contains("DTS NEURAL:X"));
    assert_eq!(ConfigRepository::load(&repository).unwrap(), config);
    assert_eq!(
        block_on(AsyncConfigRepository::load(&repository)).unwrap(),
        config
    );

    // The asynchronous save writes the same bytes.
    let other = Scratch::new("round-trip-async");
    let async_repository = YamlConfigRepository::new(other.file());
    block_on(AsyncConfigRepository::save(&async_repository, &config)).unwrap();
    assert_eq!(fs::read_to_string(other.file()).unwrap(), stored);
}

#[test]
fn an_empty_configuration_can_be_saved_and_a_missing_file_is_empty() {
    let scratch = Scratch::new("empty");
    let repository = YamlConfigRepository::new(scratch.file());
    assert_eq!(
        ConfigRepository::load(&repository).unwrap(),
        ConfiguredReceivers::default()
    );
    ConfigRepository::save(&repository, &ConfiguredReceivers::default()).unwrap();
    assert_eq!(
        ConfigRepository::load(&repository).unwrap(),
        ConfiguredReceivers::default()
    );
}

#[test]
fn the_single_receiver_file_is_read_as_one_entry_named_by_its_friendly_name() {
    let scratch = Scratch::new("single");
    fs::write(scratch.file(), SINGLE_RECEIVER).unwrap();
    let repository = YamlConfigRepository::new(scratch.file());
    let config = ConfigRepository::load(&repository).unwrap();
    assert_eq!(config.current.as_deref(), Some("living-room"));
    assert_eq!(config.receivers.len(), 1);
    assert_eq!(config.receivers["living-room"].host, "192.0.2.10");
    assert!(config.is_sound_mode_favorite("DTS NEURAL:X"));
    // Reading does not rewrite anything.
    assert_eq!(fs::read_to_string(scratch.file()).unwrap(), SINGLE_RECEIVER);
    assert!(scratch.backups().is_empty());
}

#[test]
fn an_unnamed_single_receiver_is_named_default() {
    let scratch = Scratch::new("unnamed");
    fs::write(scratch.file(), "receiver:\n  host: 192.0.2.10\n").unwrap();
    let config = ConfigRepository::load(&YamlConfigRepository::new(scratch.file())).unwrap();
    assert_eq!(config.current.as_deref(), Some("default"));
    assert!(config.receivers.contains_key("default"));
}

#[test]
fn the_first_rewrite_keeps_the_old_file_once_and_later_saves_add_no_backup() {
    let scratch = Scratch::new("backup-once");
    fs::write(scratch.file(), SINGLE_RECEIVER).unwrap();
    let repository = YamlConfigRepository::new(scratch.file());
    let mut config = ConfigRepository::load(&repository).unwrap();

    ConfigRepository::save(&repository, &config).unwrap();
    assert_eq!(scratch.backups(), vec!["denon-avr-remote.yaml.v3.bak"]);
    let backup = scratch.dir.join("denon-avr-remote.yaml.v3.bak");
    assert_eq!(fs::read_to_string(&backup).unwrap(), SINGLE_RECEIVER);
    assert!(fs::read_to_string(scratch.file())
        .unwrap()
        .starts_with("version: 2\n"));

    // Later saves find the new schema and keep nothing more.
    config
        .receivers
        .insert("den".into(), ReceiverIdentity::ad_hoc("192.0.2.11"));
    ConfigRepository::save(&repository, &config).unwrap();
    block_on(AsyncConfigRepository::save(&repository, &config)).unwrap();
    assert_eq!(scratch.backups(), vec!["denon-avr-remote.yaml.v3.bak"]);
    assert_eq!(fs::read_to_string(&backup).unwrap(), SINGLE_RECEIVER);
}

#[test]
fn the_asynchronous_save_also_keeps_the_old_file_once() {
    let scratch = Scratch::new("backup-async");
    fs::write(scratch.file(), SINGLE_RECEIVER).unwrap();
    let repository = YamlConfigRepository::new(scratch.file());
    let config = block_on(AsyncConfigRepository::load(&repository)).unwrap();
    block_on(AsyncConfigRepository::save(&repository, &config)).unwrap();
    block_on(AsyncConfigRepository::save(&repository, &config)).unwrap();
    assert_eq!(scratch.backups(), vec!["denon-avr-remote.yaml.v3.bak"]);
    assert_eq!(
        fs::read_to_string(scratch.dir.join("denon-avr-remote.yaml.v3.bak")).unwrap(),
        SINGLE_RECEIVER
    );
}

#[test]
fn an_existing_backup_is_never_overwritten() {
    let scratch = Scratch::new("backup-kept");
    let earlier = scratch.dir.join("denon-avr-remote.yaml.v3.bak");
    fs::write(&earlier, "an earlier backup").unwrap();
    fs::write(scratch.file(), SINGLE_RECEIVER).unwrap();
    let repository = YamlConfigRepository::new(scratch.file());
    let config = ConfigRepository::load(&repository).unwrap();

    ConfigRepository::save(&repository, &config).unwrap();

    assert_eq!(fs::read_to_string(&earlier).unwrap(), "an earlier backup");
    assert_eq!(
        scratch.backups(),
        vec![
            "denon-avr-remote.yaml.v3.bak",
            "denon-avr-remote.yaml.v3.bak.1"
        ]
    );
    assert_eq!(
        fs::read_to_string(scratch.dir.join("denon-avr-remote.yaml.v3.bak.1")).unwrap(),
        SINGLE_RECEIVER
    );
}

#[test]
fn a_new_file_needs_no_backup() {
    let scratch = Scratch::new("no-backup");
    let repository = YamlConfigRepository::new(scratch.file());
    ConfigRepository::save(&repository, &two_receivers()).unwrap();
    assert!(scratch.backups().is_empty());
}

#[test]
fn a_file_this_release_cannot_read_is_kept_before_it_is_replaced() {
    let scratch = Scratch::new("unreadable");
    fs::write(scratch.file(), "this is not a configuration").unwrap();
    let repository = YamlConfigRepository::new(scratch.file());
    ConfigRepository::save(&repository, &two_receivers()).unwrap();
    assert_eq!(
        fs::read_to_string(scratch.dir.join("denon-avr-remote.yaml.v3.bak")).unwrap(),
        "this is not a configuration"
    );
}

/// Save over `old` through each repository in turn, with a fresh file each
/// time, and hand the stored text and the backups to `check`.
fn save_over(old: &str, check: impl Fn(&Scratch)) {
    for asynchronous in [false, true] {
        let scratch = Scratch::new(if asynchronous { "async" } else { "sync" });
        fs::write(scratch.file(), old).unwrap();
        let repository = YamlConfigRepository::new(scratch.file());
        if asynchronous {
            block_on(AsyncConfigRepository::save(&repository, &two_receivers())).unwrap();
        } else {
            ConfigRepository::save(&repository, &two_receivers()).unwrap();
        }
        check(&scratch);
    }
}

fn assert_kept_before_replacing(old: &str) {
    save_over(old, |scratch| {
        assert_eq!(scratch.backups(), vec!["denon-avr-remote.yaml.v3.bak"]);
        assert_eq!(
            fs::read_to_string(scratch.dir.join("denon-avr-remote.yaml.v3.bak")).unwrap(),
            old,
            "the backup is the old file, byte for byte"
        );
        assert!(fs::read_to_string(scratch.file())
            .unwrap()
            .starts_with("version: 2\n"));
    });
}

#[test]
fn a_file_in_a_version_this_release_does_not_know_is_kept_before_it_is_replaced() {
    // It has the new schema's keys, so the keys alone do not make it readable.
    assert_kept_before_replacing("version: 3\nreceivers:\n  den:\n    host: 192.0.2.1\n");
    assert_kept_before_replacing("version: 1\nreceivers:\n  den:\n    host: 192.0.2.1\n");
}

#[test]
fn a_hand_edited_current_file_this_release_cannot_read_is_kept_before_it_is_replaced() {
    // A misspelt key.
    assert_kept_before_replacing(
        "version: 2\nreceivers:\n  den:\n    host: 192.0.2.1\n    hots: 192.0.2.2\n",
    );
    // A current receiver that is not configured.
    assert_kept_before_replacing(
        "version: 2\ncurrent: gone\nreceivers:\n  den:\n    host: 192.0.2.1\n",
    );
    // A duplicate name, which is refused rather than merged.
    assert_kept_before_replacing(
        "version: 2\nreceivers:\n  den:\n    host: 192.0.2.1\n  den:\n    host: 192.0.2.2\n",
    );
}

#[test]
fn a_readable_current_file_is_replaced_without_a_backup() {
    let old = "version: 2\ncurrent: den\nreceivers:\n  den:\n    host: 192.0.2.1\n";
    save_over(old, |scratch| assert!(scratch.backups().is_empty()));
}

#[test]
fn a_blank_file_has_nothing_to_keep() {
    for old in ["", "\n  \n"] {
        save_over(old, |scratch| {
            assert!(scratch.backups().is_empty());
            assert!(fs::read_to_string(scratch.file())
                .unwrap()
                .starts_with("version: 2\n"));
        });
    }
}

#[test]
fn a_rejected_save_leaves_the_old_file_and_makes_no_backup() {
    let scratch = Scratch::new("rejected-save");
    fs::write(scratch.file(), SINGLE_RECEIVER).unwrap();
    let repository = YamlConfigRepository::new(scratch.file());
    let mut config = ConfigRepository::load(&repository).unwrap();
    config.current = Some("missing".into());

    assert!(ConfigRepository::save(&repository, &config).is_err());
    assert!(block_on(AsyncConfigRepository::save(&repository, &config)).is_err());
    assert_eq!(fs::read_to_string(scratch.file()).unwrap(), SINGLE_RECEIVER);
    assert!(scratch.backups().is_empty());
}

fn assert_rejected_by_both(text: &str, expected: &str) {
    let scratch = Scratch::new("rejected-read");
    fs::write(scratch.file(), text).unwrap();
    let repository = YamlConfigRepository::new(scratch.file());
    let sync = ConfigRepository::load(&repository).unwrap_err();
    let asynchronous = block_on(AsyncConfigRepository::load(&repository)).unwrap_err();
    assert!(sync.to_string().contains(expected), "{sync} for {text:?}");
    assert_eq!(sync, asynchronous, "both repositories reject alike");
}

#[test]
fn duplicate_receiver_names_are_rejected_not_merged() {
    assert_rejected_by_both(
        "version: 2\ncurrent: den\nreceivers:\n  den:\n    host: 192.0.2.1\n  den:\n    host: 192.0.2.2\n",
        "duplicate",
    );
}

#[test]
fn duplicate_top_level_keys_are_rejected() {
    assert_rejected_by_both(
        "version: 2\nreceivers:\n  den:\n    host: 192.0.2.1\nreceivers:\n  den:\n    host: 192.0.2.2\n",
        "duplicate",
    );
}

#[test]
fn reserved_and_empty_names_are_rejected_on_read_and_on_save() {
    assert_rejected_by_both(
        "version: 2\nreceivers:\n  \"adhoc:192.0.2.1\":\n    host: 192.0.2.1\n",
        "reserved",
    );
    assert_rejected_by_both(
        "version: 2\nreceivers:\n  \"\":\n    host: 192.0.2.1\n",
        "must not be empty",
    );
    let scratch = Scratch::new("reserved-save");
    let repository = YamlConfigRepository::new(scratch.file());
    let reserved = ConfiguredReceivers {
        current: None,
        receivers: BTreeMap::from([(
            "adhoc:192.0.2.1".into(),
            ReceiverIdentity::ad_hoc("192.0.2.1"),
        )]),
        ..ConfiguredReceivers::default()
    };
    let error = ConfigRepository::save(&repository, &reserved).unwrap_err();
    assert!(error.to_string().contains("reserved"), "{error}");
    assert!(!scratch.file().exists());
}

#[test]
fn other_versions_and_inconsistent_files_are_rejected() {
    let receivers = "receivers:\n  den:\n    host: 192.0.2.1\n";
    assert_rejected_by_both(
        &format!("version: 3\n{receivers}"),
        "version 3 is not supported",
    );
    assert_rejected_by_both(
        &format!("version: 1\n{receivers}"),
        "version 1 is not supported",
    );
    assert_rejected_by_both(
        &format!("version: 2\ncurrent: gone\n{receivers}"),
        "current receiver gone is not configured",
    );
    assert_rejected_by_both(
        &format!("version: 2\n{receivers}sound_mode_favorites:\n  gone:\n    - STEREO\n"),
        "unknown receiver gone",
    );
    assert_rejected_by_both(
        &format!("version: 2\nreceiver:\n  host: 192.0.2.1\n{receivers}"),
        "both `version` and `receiver`",
    );
    assert_rejected_by_both("hosts: []\n", "neither `version` nor `receiver`");
    assert_rejected_by_both("receiver:\n  host: \"\"\n", "empty host");
}

#[test]
fn an_earlier_release_cannot_read_the_new_file() {
    let scratch = Scratch::new("old-release");
    let repository = YamlConfigRepository::new(scratch.file());
    ConfigRepository::save(&repository, &two_receivers()).unwrap();
    let stored = fs::read_to_string(scratch.file()).unwrap();
    // Version 3.0.0 parsed exactly this shape and refused anything else.
    assert!(serde_yaml::from_str::<LegacyConfigFile>(&stored).is_err());
}

#[test]
fn category_keyed_favorites_flatten_without_discarding_modes() {
    let scratch = Scratch::new("category-keyed");
    fs::write(
        scratch.file(),
        "receiver:\n  host: 192.0.2.10\nsound_mode_favorites:\n  default:\n    movie:\n      - DTS NEURAL:X\n      - DOLBY SURROUND\n    music:\n      - DTS NEURAL:X\n",
    )
    .unwrap();
    let repository = YamlConfigRepository::new(scratch.file());
    let config = ConfigRepository::load(&repository).unwrap();
    assert!(config.is_sound_mode_favorite("DTS NEURAL:X"));
    assert!(config.is_sound_mode_favorite("DOLBY SURROUND"));
    assert_eq!(config.sound_mode_favorites["default"].len(), 2);

    ConfigRepository::save(&repository, &config).unwrap();
    let stored = fs::read_to_string(scratch.file()).unwrap();
    assert!(stored.contains("DOLBY SURROUND") && stored.contains("DTS NEURAL:X"));
    assert!(!stored.contains("movie:"));
    assert_eq!(ConfigRepository::load(&repository).unwrap(), config);
}

#[cfg(not(target_os = "macos"))]
#[test]
fn default_repository_keeps_repository_relative_development_path() {
    assert_eq!(
        YamlConfigRepository::default().path(),
        Path::new("config/denon-avr-remote.yaml")
    );
}
