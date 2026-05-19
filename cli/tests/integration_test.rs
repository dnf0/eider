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

#[test]
fn test_cli_completions_bash() {
    let mut cmd = Command::cargo_bin("zarrduck").unwrap();
    cmd.arg("completions")
        .arg("bash")
        .assert()
        .success()
        .stdout(predicate::str::contains("_zarrduck() {"));
}

#[test]
fn test_cli_search_invalid_api() {
    let mut cmd = Command::cargo_bin("zarrduck").unwrap();
    cmd.arg("search")
        .arg("--api")
        .arg("http://api.test.invalid")
        .arg("--collection")
        .arg("era5")
        .arg("--output=json")
        .assert()
        .failure()
        .stdout(predicate::str::contains(r#""status":"error""#));
}

#[test]
fn test_cli_resample_missing_input() {
    let mut cmd = Command::cargo_bin("zarrduck").unwrap();
    cmd.arg("resample")
        .arg("missing_input.duckdb")
        .arg("out.duckdb")
        .arg("--freq")
        .arg("month")
        .arg("--agg")
        .arg("avg")
        .arg("--output=json")
        .assert()
        .failure()
        .stdout(predicate::str::contains("Input database 'missing_input.duckdb' does not exist"));
}

#[test]
fn test_cli_ingest_missing_input() {
    let mut cmd = Command::cargo_bin("zarrduck").unwrap();
    cmd.arg("ingest")
        .arg("missing_input.nc")
        .arg("s3://bucket/out.zarr")
        .arg("--output=json")
        .assert()
        .failure()
        .stdout(predicate::str::contains("Input file 'missing_input.nc' does not exist"));
}