use std::process::Command;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_denon-avr-remote"))
}

#[test]
fn help_lists_phase_one_commands() {
    let output = cli().arg("help").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("discover"));
    assert!(stdout.contains("status"));
}

#[test]
fn invalid_status_selection_is_nonzero_and_actionable() {
    let output = cli()
        .args(["status", "--receiver", "nope"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--receiver requires a number"));
}

#[test]
fn development_volume_command_remains_available() {
    let output = cli().args(["volume", "5"]).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "MV805\n");
}
