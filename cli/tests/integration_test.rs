use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn test_cli_help() {
    let mut cmd = Command::cargo_bin("zarrduck").unwrap();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Agentic Spatial Data Engine"));
}

#[test]
fn test_cli_info_invalid_uri_table() {
    let mut cmd = Command::cargo_bin("zarrduck").unwrap();
    cmd.arg("info")
        .arg("s3://invalid-bucket-that-does-not-exist/data.zarr")
        .arg("--output=table")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Failed to load extension at").or(predicate::str::contains("Failed to read metadata for")));
}

#[test]
fn test_cli_info_invalid_uri_json() {
    let mut cmd = Command::cargo_bin("zarrduck").unwrap();
    cmd.arg("info")
        .arg("s3://invalid-bucket-that-does-not-exist/data.zarr")
        .arg("--output=json")
        .assert()
        .failure()
        .stdout(predicate::str::contains(r#""message":"#))
        .stdout(predicate::str::contains(r#""status":"error""#));
}