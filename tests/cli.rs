use std::process::Command;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_denon-avr-remote"))
}

#[test]
fn help_lists_get_resources_and_selectors() {
    let output = cli().arg("help").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for resource in [
        "receivers",
        "status",
        "power",
        "input",
        "volume",
        "mute",
        "surround",
    ] {
        assert!(stdout.contains(resource), "missing {resource}");
    }
    assert!(stdout.contains("--host HOST"));
    assert!(stdout.contains("--receiver N"));
    assert!(!stdout.contains("discover            "));
    assert!(!stdout.contains("avr <command>"));
    assert!(!stdout.contains("heos <command>"));
}

#[test]
fn each_receiver_backed_resource_accepts_new_grammar() {
    for resource in ["status", "power", "input", "volume", "mute", "surround"] {
        let output = cli()
            .args(["get", resource, "--receiver", "nope"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{resource}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("--receiver requires"));
    }
}

#[test]
fn receivers_resource_accepts_new_grammar_without_running_network() {
    let output = cli()
        .args(["get", "receivers", "--host", "127.0.0.1"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("get receivers does not accept"));
}

#[test]
fn selectors_are_validated() {
    let output = cli()
        .args(["get", "status", "--receiver", "0"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("positive number"));

    let output = cli()
        .args(["get", "status", "--host", "127.0.0.1", "--receiver", "1"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be combined"));
}

#[test]
fn old_top_level_commands_are_rejected() {
    for command in ["discover", "status", "avr", "heos", "volume"] {
        let output = cli().arg(command).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{command}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("unknown command"));
    }
}
