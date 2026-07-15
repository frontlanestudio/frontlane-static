use std::process::Command;
use assert_cmd::prelude::*;
use predicates::prelude::*;

#[test]
fn test_cli_dry_run() {
    let mut cmd = Command::cargo_bin("frontlane-static").unwrap();
    cmd.arg("--dry-run").arg("https://example.com");
    let output = cmd.assert().success().get_output();
    let output_str = String::from_utf8_lossy(&output.stdout);
    assert!(output_str.contains("Starting Site Backup:"));
}

#[test]
fn test_cli_help() {
    let mut cmd = Command::cargo_bin("frontlane-static").unwrap();
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Usage: frontlane-static"));
}
