mod common;

use assert_cmd::Command;
use insta::assert_snapshot;

#[test]
fn test_cli_help_snapshot() {
    let mut cmd = Command::cargo_bin("eider").unwrap();
    let assert = cmd.arg("--help").assert().success();
    let output = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    // We strip out versions, trailing spaces, and .exe extension (on Windows) as they can make snapshots brittle
    let cleaned_output = output
        .lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        .replace("eider.exe", "eider");

    assert_snapshot!(cleaned_output);
}

fn clean(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        .replace("eider.exe", "eider")
}

#[test]
fn resample_help_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let assert = common::eider(&dir)
        .args(["resample", "--help"])
        .assert()
        .success();
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    insta::assert_snapshot!(clean(&out));
}

#[test]
fn extract_help_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let assert = common::eider(&dir)
        .args(["extract", "--help"])
        .assert()
        .success();
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    insta::assert_snapshot!(clean(&out));
}
