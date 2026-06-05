mod common;
use common::*;
use predicates::prelude::*;

#[test]
fn resample_year_avg_is_correct() {
    let dir = tempfile::tempdir().unwrap();
    let input = make_extracted_db_numeric_time(&dir);
    let out = dir.path().join("resampled.duckdb");

    eider(&dir)
        .arg("resample")
        .arg(&input)
        .arg(&out)
        .args(["--freq", "year", "--agg", "avg", "--output=json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""status": "success""#));

    let conn = duckdb::Connection::open(&out).unwrap();
    let v2020: f64 = conn
        .query_row(
            "SELECT value FROM resampled_data WHERE year(time) = 2020",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        (v2020 - 3.0).abs() < 1e-9,
        "2020 avg should be 3.0, got {v2020}"
    );
    let v2021: f64 = conn
        .query_row(
            "SELECT value FROM resampled_data WHERE year(time) = 2021",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        (v2021 - 10.0).abs() < 1e-9,
        "2021 avg should be 10.0, got {v2021}"
    );
}

#[test]
fn resample_rejects_invalid_agg() {
    let dir = tempfile::tempdir().unwrap();
    let input = make_extracted_db_numeric_time(&dir);
    let out = dir.path().join("out.duckdb");
    eider(&dir)
        .arg("resample")
        .arg(&input)
        .arg(&out)
        .args(["--freq", "year", "--agg", "bogus", "--output=json"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("Invalid aggregation function"));
}

#[test]
fn resample_json_requires_freq() {
    let dir = tempfile::tempdir().unwrap();
    let input = make_extracted_db_numeric_time(&dir);
    let out = dir.path().join("out.duckdb");
    eider(&dir)
        .arg("resample")
        .arg(&input)
        .arg(&out)
        .args(["--agg", "avg", "--output=json"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("--freq is required"));
}

#[test]
fn resample_refuses_to_overwrite_in_json_mode() {
    let dir = tempfile::tempdir().unwrap();
    let input = make_extracted_db_numeric_time(&dir);
    let out = dir.path().join("exists.duckdb");
    std::fs::write(&out, b"not empty").unwrap();
    eider(&dir)
        .arg("resample")
        .arg(&input)
        .arg(&out)
        .args(["--freq", "year", "--agg", "avg", "--output=json"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("already exists"));
}
