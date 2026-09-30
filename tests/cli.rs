use std::process::Command;
use assert_cmd::prelude::*;
use predicates::prelude::*;

#[test]
fn test_cli_version() {
    let mut cmd = Command::cargo_bin("frontlane-static").unwrap();
    cmd.arg("--version");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("frontlane-static"));
}

#[test]
fn test_cli_help() {
    let mut cmd = Command::cargo_bin("frontlane-static").unwrap();
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Usage: frontlane-static"))
        .stdout(predicate::str::contains("--headless-scroll"))
        .stdout(predicate::str::contains("--sitemap-timeout"))
        .stdout(predicate::str::contains("--serve"))
        .stdout(predicate::str::contains("--audit"))
        .stdout(predicate::str::contains("--form-webhook"))
        .stdout(predicate::str::contains("--generate-search"))
        .stdout(predicate::str::contains("--localize-fonts"))
        .stdout(predicate::str::contains("--max-posts"))
        .stdout(predicate::str::contains("--extract-data"))
        .stdout(predicate::str::contains("--port"))
        .stdout(predicate::str::contains("--spa"));
}

#[test]
fn test_cli_audit_subcommand() {
    let temp_dir = tempfile::tempdir().unwrap();
    let p = temp_dir.path();
    std::fs::write(p.join("index.html"), r#"<!DOCTYPE html><html><body><a href="about.html">About</a></body></html>"#).unwrap();
    std::fs::write(p.join("about.html"), r#"<!DOCTYPE html><html><body><a href="index.html">Home</a></body></html>"#).unwrap();

    let mut cmd = Command::cargo_bin("frontlane-static").unwrap();
    cmd.arg("audit")
       .arg(p.to_str().unwrap())
       .arg("--live-url")
       .arg("https://example.com");

    let assert = cmd.assert().success();
    let output = assert.get_output();
    let output_str = String::from_utf8_lossy(&output.stdout);
    assert!(output_str.contains("CLONE FIDELITY AUDIT REPORT:"));
    assert!(output_str.contains("Overall Fidelity Score:"));
}


