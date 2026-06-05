# eider CLI Test Suite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a hermetic, layered test suite for the `eider` CLI covering all 9 subcommands with unit, integration, snapshot, and interactive tests.

**Architecture:** A layered pyramid — co-located `#[cfg(test)]` unit tests for pure logic, one integration-test file per command in `cli/tests/` driving the compiled binary via `assert_cmd`, `insta` snapshots for output, and a `#[cfg(unix)]` `rexpect` tier for prompts/REPL. A shared `cli/tests/common/` harness provides an in-process `wiremock` STAC server, fixture locators, an extension finder, and a synthetic-db builder. Hermetic via local fixtures + mocked STAC; the DuckDB `spatial` extension is cached via `DUCKDB_EXTENSION_DIRECTORY`.

**Tech Stack:** Rust, `assert_cmd`, `predicates`, `insta`, `wiremock`, `serial_test`, `tempfile`, `rexpect`, `duckdb` (bundled =1.10502.0), `tokio`.

---

## Conventions for this plan

- **Working directory:** all `cargo`/`git` commands run from the repo root `/Users/danielfisher/repos/zarrduck` unless stated. Cargo commands target the CLI with `-p eider`.
- **Two test rhythms appear here.** Most of `eider`'s pure logic *already exists*, so its unit tests are **characterization tests**: write the test, run it, expect it to **PASS immediately** (it documents/locks current behavior). Where we *extract new* logic (hybrid refactor), classic TDD applies: test fails first, then implement. Each task states which rhythm it uses.
- **Extension prerequisite:** integration tests for `info`/`extract`/`export`/`ingest` require the built extension. Build it once before running those: `cargo duckdb-ext build` (produces `target/debug/eider.duckdb_extension`, already present in this workspace).
- **Commit style:** Conventional Commits, `--no-gpg-sign` (repo policy skips GPG). End commit messages with the `Co-Authored-By` trailer. Never commit to `main` — work stays on `test/cli-comprehensive-suite`.
- After each commit, repo policy asks to run `roborev status` / `roborev show HEAD`; do so and address criticals.

## Fixture data reference (climate_data.zarr, at repo root)

- Dimensions: `time[938]` (`<f8`), `lat[73]` (`<f4`, 90.0 → -90.0 step 2.5), `lon[144]` (`<f4`, 0.0 → 357.5 step 2.5).
- Array `air_temperature[time, lat, lon]` `<f4`, attrs include `_ARRAY_DIMENSIONS=[time,lat,lon]`, `geozarr.spatial_reference.crs=EPSG:4326`, transform translation `[0, 90, -180]` scale `[1, -2.5, 2.5]` (projects grid → lon -180..177.5, lat 90..-90).
- `read_zarr_metadata('<uri>')` returns columns `array_shape, chunk_shape, data_type, crs`.

---

## Phase 1 — Harness + fixtures

### Task 1: Add `wiremock` dev-dependency

**Files:**
- Modify: `cli/Cargo.toml` (dev-dependencies block)

- [ ] **Step 1: Add the dependency**

In `cli/Cargo.toml`, under `[dev-dependencies]`, add:

```toml
wiremock = "0.6"
```

- [ ] **Step 2: Verify it resolves**

Run: `cargo fetch -p eider`
Expected: completes without error; `wiremock` and transitive deps appear in `Cargo.lock`.

- [ ] **Step 3: Commit**

```bash
git add cli/Cargo.toml Cargo.lock
git commit --no-gpg-sign -m "test: add wiremock dev-dependency for mock STAC server

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Fixture files

**Files:**
- Create: `cli/tests/fixtures/polygon.geojson`
- Create: `cli/tests/fixtures/ingest_input.csv`

- [ ] **Step 1: Create the extraction polygon**

The polygon must cover *projected* grid points (lon -180..177.5, lat -90..90, both step 2.5). A box over lon[-6,6] × lat[-6,6] contains grid points at 0, ±2.5, ±5 in each axis. Create `cli/tests/fixtures/polygon.geojson`:

```json
{
  "type": "FeatureCollection",
  "features": [
    {
      "type": "Feature",
      "properties": { "name": "test_box" },
      "geometry": {
        "type": "Polygon",
        "coordinates": [
          [[-6.0, -6.0], [6.0, -6.0], [6.0, 6.0], [-6.0, 6.0], [-6.0, -6.0]]
        ]
      }
    }
  ]
}
```

- [ ] **Step 2: Create the ingest CSV**

Create `cli/tests/fixtures/ingest_input.csv` (a small regular lon/lat grid with a value column; `ST_Read` reads CSV via GDAL):

```csv
lon,lat,value
0.0,0.0,1.0
2.5,0.0,2.0
0.0,2.5,3.0
2.5,2.5,4.0
```

- [ ] **Step 3: Commit**

```bash
git add cli/tests/fixtures/polygon.geojson cli/tests/fixtures/ingest_input.csv
git commit --no-gpg-sign -m "test: add geojson polygon and csv ingest fixtures

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Shared harness `cli/tests/common/mod.rs`

Rhythm: classic — this is new code; we validate it with a smoke test.

**Files:**
- Create: `cli/tests/common/mod.rs`
- Create: `cli/tests/harness_smoke_test.rs`

- [ ] **Step 1: Write the harness**

Create `cli/tests/common/mod.rs`:

```rust
//! Shared test harness for eider CLI integration tests.
//!
//! Each integration test file does `mod common;` and `use common::*;`.
#![allow(dead_code)] // not every test file uses every helper

use std::path::PathBuf;

use assert_cmd::Command;
use duckdb::Connection;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Absolute path to the CLI crate root (`cli/`).
pub fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Absolute path to the workspace root (parent of `cli/`).
pub fn repo_root() -> PathBuf {
    manifest_dir().parent().unwrap().to_path_buf()
}

/// Path to a file under `cli/tests/fixtures/`.
pub fn fixture_path(name: &str) -> PathBuf {
    manifest_dir().join("tests").join("fixtures").join(name)
}

/// Path to the real Zarr fixture at the repo root.
pub fn climate_zarr() -> PathBuf {
    repo_root().join("climate_data.zarr")
}

/// Locate the built eider DuckDB extension, or fail loudly with guidance.
pub fn geozarr_ext_path() -> PathBuf {
    let candidates = [
        repo_root().join("target/debug/eider.duckdb_extension"),
        repo_root().join("target/release/eider.duckdb_extension"),
    ];
    candidates
        .iter()
        .find(|p| p.exists())
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "eider.duckdb_extension not found in target/{{debug,release}}. \
                 Build it first with: cargo duckdb-ext build"
            )
        })
}

/// True when the external `duckdb` CLI is on PATH (gates shell REPL tests).
pub fn duckdb_cli_available() -> bool {
    std::process::Command::new("duckdb")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A configured `assert_cmd::Command` for the `eider` binary, isolated in `dir`.
pub fn eider(dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("eider").unwrap();
    cmd.current_dir(dir.path());
    // Cache DuckDB-managed extensions (e.g. spatial) in a stable per-user dir so
    // the network fetch happens at most once across the whole suite.
    cmd.env(
        "DUCKDB_EXTENSION_DIRECTORY",
        repo_root().join(".duckdb_ext_cache"),
    );
    cmd
}

/// Build a deterministic `.duckdb` file containing an `extracted_data` table with
/// a numeric (unix-seconds) time column. Returns the path to the db file.
///
/// Rows (lat=10, lon=20):
///   2020-01-15 -> 2.0, 2020-06-15 -> 4.0, 2021-01-15 -> 10.0
/// so year-avg gives 2020 => 3.0, 2021 => 10.0.
pub fn make_extracted_db_numeric_time(dir: &TempDir) -> PathBuf {
    let db_path = dir.path().join("extracted.duckdb");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch(
        "CREATE TABLE extracted_data (
             time BIGINT,
             lat DOUBLE,
             lon DOUBLE,
             air_temperature DOUBLE
         );
         INSERT INTO extracted_data VALUES
             (1579046400, 10.0, 20.0, 2.0),   -- 2020-01-15
             (1592179200, 10.0, 20.0, 4.0),   -- 2020-06-15
             (1610668800, 10.0, 20.0, 10.0);  -- 2021-01-15",
    )
    .unwrap();
    db_path
}

/// Start an in-process mock STAC server. Mirrors scripts/mock_stac.py:
/// GET /collections lists one zarr-bearing collection; POST /search returns one
/// feature with a zarr asset. Returns the running server (keep it alive for the
/// duration of the test; its `.uri()` is the `--api` base URL).
pub async fn mock_stac() -> MockServer {
    let server = MockServer::start().await;

    let collections_body = serde_json::json!({
        "collections": [{
            "id": "cmip6-cesm2-historical",
            "title": "CMIP6 CESM2 Historical Surface Temperature",
            "description": "Near-surface air temperature, monthly means.",
            "assets": {
                "data": {
                    "href": "https://example.com/cmip6/tas.zarr",
                    "type": "application/vnd+zarr"
                }
            }
        }]
    });

    let search_body = serde_json::json!({
        "features": [{
            "assets": {
                "data": {
                    "href": "https://example.com/cmip6/tas.zarr",
                    "type": "application/vnd+zarr",
                    "title": "CMIP6 CESM2 Near-Surface Air Temperature",
                    "description": "Monthly mean near-surface air temperature (tas)."
                }
            }
        }]
    });

    Mock::given(method("GET"))
        .and(path("/collections"))
        .respond_with(ResponseTemplate::new(200).set_body_json(collections_body))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_body))
        .mount(&server)
        .await;

    server
}
```

- [ ] **Step 2: Write a smoke test for the harness**

Create `cli/tests/harness_smoke_test.rs`:

```rust
mod common;
use common::*;

#[test]
fn fixtures_exist() {
    assert!(climate_zarr().exists(), "climate_data.zarr fixture missing");
    assert!(fixture_path("polygon.geojson").exists());
    assert!(fixture_path("ingest_input.csv").exists());
}

#[test]
fn synthetic_db_has_expected_rows() {
    let dir = tempfile::tempdir().unwrap();
    let db = make_extracted_db_numeric_time(&dir);
    let conn = duckdb::Connection::open(&db).unwrap();
    let n: i64 = conn
        .query_row("SELECT count(*) FROM extracted_data", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 3);
}

#[tokio::test]
async fn mock_stac_serves_collections() {
    let server = mock_stac().await;
    let url = format!("{}/collections", server.uri());
    let body: serde_json::Value = reqwest::get(&url).await.unwrap().json().await.unwrap();
    assert_eq!(body["collections"][0]["id"], "cmip6-cesm2-historical");
}
```

- [ ] **Step 3: Run the smoke test**

Run: `cargo test -p eider --test harness_smoke_test`
Expected: 3 passed. (If `Command::cargo_bin` or fixtures resolve wrong, fix paths now.)

- [ ] **Step 4: Commit**

```bash
git add cli/tests/common/mod.rs cli/tests/harness_smoke_test.rs
git commit --no-gpg-sign -m "test: add shared CLI test harness and smoke test

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Add `.duckdb_ext_cache` to `.gitignore`

**Files:**
- Modify: `.gitignore`

- [ ] **Step 1: Append the ignore entry**

Add a line to `.gitignore`:

```
.duckdb_ext_cache/
```

- [ ] **Step 2: Commit**

```bash
git add .gitignore
git commit --no-gpg-sign -m "chore: ignore local DuckDB extension cache dir

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Phase 2 — Unit tests (co-located, characterization rhythm)

These add `#[cfg(test)] mod tests` blocks (or extend existing ones) in `cli/src/`. Run each module's tests with `cargo test -p eider <module>::`. All are expected to **PASS immediately** — they lock current behavior.

### Task 5: `duckdb_utils` pure-logic tests

**Files:**
- Modify: `cli/src/duckdb_utils.rs` (append a `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the tests**

Append to `cli/src/duckdb_utils.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use duckdb::Connection;

    #[test]
    fn format_pins_empty_is_blank() {
        assert_eq!(format_pins(&[]), "");
    }

    #[test]
    fn format_pins_joins_with_prefix() {
        let pins = vec!["time=0".to_string(), "lat=5".to_string()];
        assert_eq!(format_pins(&pins), ", pins := 'time=0,lat=5'");
    }

    #[test]
    fn format_pins_where_empty_is_blank() {
        assert_eq!(format_pins_where(&[]), "");
    }

    #[test]
    fn format_pins_where_builds_conditions() {
        let pins = vec!["time=0".to_string(), "lat=5".to_string()];
        assert_eq!(
            format_pins_where(&pins),
            " WHERE \"time\" = 0 AND \"lat\" = 5"
        );
    }

    #[test]
    fn format_pins_where_passes_through_malformed() {
        let pins = vec!["garbage".to_string()];
        assert_eq!(format_pins_where(&pins), " WHERE garbage");
    }

    fn mem_with_table(ddl: &str) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(ddl).unwrap();
        conn
    }

    #[test]
    fn detect_columns_finds_standard_names() {
        let conn = mem_with_table(
            "CREATE TABLE extracted_data (time BIGINT, lat DOUBLE, lon DOUBLE, air_temperature DOUBLE);",
        );
        let (t, la, lo, v, numeric) = detect_columns(&conn, "extracted_data").unwrap();
        assert_eq!((t.as_str(), la.as_str(), lo.as_str(), v.as_str()), ("time", "lat", "lon", "air_temperature"));
        assert!(numeric);
    }

    #[test]
    fn detect_columns_marks_timestamp_non_numeric() {
        let conn = mem_with_table(
            "CREATE TABLE t (time TIMESTAMP, lat DOUBLE, lon DOUBLE, val DOUBLE);",
        );
        let (_, _, _, _, numeric) = detect_columns(&conn, "t").unwrap();
        assert!(!numeric);
    }

    #[test]
    fn detect_columns_errors_without_time() {
        let conn = mem_with_table("CREATE TABLE t (lat DOUBLE, lon DOUBLE, val DOUBLE);");
        assert!(detect_columns(&conn, "t").is_err());
    }

    #[test]
    fn auto_calculate_chunks_defaults() {
        let conn = mem_with_table(
            "CREATE TABLE t (time BIGINT, lat DOUBLE, lon DOUBLE, val DOUBLE);",
        );
        let map = auto_calculate_chunks(&conn, "t").unwrap();
        assert_eq!(map.get("time").unwrap(), &serde_json::json!(10));
        assert_eq!(map.get("lat").unwrap(), &serde_json::json!(100));
        assert_eq!(map.get("lon").unwrap(), &serde_json::json!(100));
    }
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p eider duckdb_utils::tests`
Expected: all PASS.

- [ ] **Step 3: Commit**

```bash
git add cli/src/duckdb_utils.rs
git commit --no-gpg-sign -m "test: unit tests for duckdb_utils pins and column detection

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: `search` pure-logic tests

**Files:**
- Modify: `cli/src/commands/search.rs` (append a `#[cfg(test)] mod tests`)

Note: `build_stac_query`, `is_supported_asset`, `parse_search_results`, `extract_assets`, `SelectOption` are private in the module, so the test module (`mod tests` inside the same file) can access them.

- [ ] **Step 1: Write the tests**

Append to `cli/src/commands/search.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_query_minimal() {
        let q = build_stac_query("era5", None, None).unwrap();
        assert_eq!(q["collections"][0], "era5");
        assert_eq!(q["limit"], 10);
        assert!(q.get("bbox").is_none());
    }

    #[test]
    fn build_query_with_valid_bbox() {
        let b = "-10,-5,10,5".to_string();
        let q = build_stac_query("c", Some(&b), None).unwrap();
        assert_eq!(q["bbox"], json!([-10.0, -5.0, 10.0, 5.0]));
    }

    #[test]
    fn build_query_rejects_wrong_bbox_len() {
        let b = "1,2,3".to_string();
        assert!(build_stac_query("c", Some(&b), None).is_err());
    }

    #[test]
    fn build_query_rejects_non_numeric_bbox() {
        let b = "a,b,c,d".to_string();
        assert!(build_stac_query("c", Some(&b), None).is_err());
    }

    #[test]
    fn build_query_with_datetime() {
        let dt = "2020-01-01/2020-12-31".to_string();
        let q = build_stac_query("c", None, Some(&dt)).unwrap();
        assert_eq!(q["datetime"], "2020-01-01/2020-12-31");
    }

    #[test]
    fn supported_asset_detects_zarr_and_cog() {
        assert!(is_supported_asset(&json!({"type": "application/vnd+zarr", "href": ""})));
        assert!(is_supported_asset(&json!({"type": "", "href": "x/data.zarr/"})));
        assert!(is_supported_asset(&json!({"type": "image/tiff", "href": ""})));
        assert!(is_supported_asset(&json!({"type": "", "href": "a.tif"})));
    }

    #[test]
    fn supported_asset_rejects_other() {
        assert!(!is_supported_asset(&json!({"type": "application/json", "href": "a.json"})));
    }

    #[test]
    fn parse_results_from_features() {
        let resp = json!({
            "features": [{
                "assets": { "data": { "type": "application/vnd+zarr", "href": "s3://b/x.zarr" } }
            }]
        });
        let opts = parse_search_results(&resp);
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].id, "s3://b/x.zarr");
    }

    #[test]
    fn parse_results_from_collection_assets() {
        let resp = json!({
            "assets": { "data": { "type": "application/vnd+zarr", "href": "s3://b/y.zarr" } }
        });
        let opts = parse_search_results(&resp);
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].id, "s3://b/y.zarr");
    }

    #[test]
    fn parse_results_skips_unsupported() {
        let resp = json!({
            "assets": { "thumb": { "type": "image/png", "href": "a.png" } }
        });
        assert!(parse_search_results(&resp).is_empty());
    }
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p eider commands::search::tests`
Expected: all PASS.

- [ ] **Step 3: Commit**

```bash
git add cli/src/commands/search.rs
git commit --no-gpg-sign -m "test: unit tests for STAC query building and result parsing

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: `resample` validation logic — extract + test (hybrid refactor, TDD rhythm)

`get_freq_and_agg` mixes interactive prompting with validation. Extract the pure validation into a testable function, then test it. This is the one targeted refactor in `resample`.

**Files:**
- Modify: `cli/src/commands/resample.rs`

- [ ] **Step 1: Write the failing tests first**

Append to `cli/src/commands/resample.rs` a test module that calls a not-yet-existing `validate_agg`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_agg_accepts_known() {
        for a in ["sum", "avg", "min", "max", "count", "mean", "median", "mode", "stddev", "variance"] {
            assert!(validate_agg(a).is_ok(), "{a} should be valid");
        }
    }

    #[test]
    fn validate_agg_is_case_insensitive() {
        assert!(validate_agg("AVG").is_ok());
    }

    #[test]
    fn validate_agg_rejects_unknown() {
        assert!(validate_agg("hack; DROP TABLE").is_err());
    }

    #[test]
    fn build_resample_query_numeric_time_wraps_timestamp() {
        let q = build_resample_query("year", "avg", "time", "lat", "lon", "value", true);
        assert!(q.contains("to_timestamp(CAST(time AS BIGINT))"));
        assert!(q.contains("date_trunc('year'"));
        assert!(q.contains("avg(value)"));
    }

    #[test]
    fn build_resample_query_text_time_uses_column_directly() {
        let q = build_resample_query("month", "sum", "time", "lat", "lon", "value", false);
        assert!(q.contains("date_trunc('month', time)"));
        assert!(!q.contains("to_timestamp"));
    }
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p eider commands::resample::tests`
Expected: FAIL — `cannot find function validate_agg` / `build_resample_query`.

- [ ] **Step 3: Extract the functions and wire them in**

In `cli/src/commands/resample.rs`, add these functions above `run_resample`:

```rust
pub(crate) fn validate_agg(agg: &str) -> EyreResult<()> {
    let allowed_aggs = [
        "sum", "avg", "min", "max", "count", "mean", "median", "mode", "stddev", "variance",
    ];
    if !allowed_aggs.contains(&agg.to_lowercase().as_str()) {
        return Err(eyre!(
            "Invalid aggregation function: '{}'. Allowed: {:?}",
            agg,
            allowed_aggs
        ));
    }
    Ok(())
}

pub(crate) fn build_resample_query(
    freq: &str,
    agg: &str,
    time_col: &str,
    lat_col: &str,
    lon_col: &str,
    val_col: &str,
    time_is_numeric: bool,
) -> String {
    let time_expr = if time_is_numeric {
        format!("to_timestamp(CAST({} AS BIGINT))", time_col)
    } else {
        time_col.to_string()
    };
    format!(
        "CREATE TABLE resampled_data AS
         SELECT
             date_trunc('{}', {}) as {},
             {}, {},
             {}({}) as value
         FROM source_db.extracted_data
         GROUP BY 1, 2, 3",
        freq.replace('\'', "''"),
        time_expr,
        time_col,
        lat_col,
        lon_col,
        agg,
        val_col
    )
}
```

Then in `get_freq_and_agg`, replace the inline `allowed_aggs`/`contains` validation block with:

```rust
    validate_agg(&selected_agg)?;
    Ok((selected_freq, selected_agg))
```

And in `run_resample`, replace the inline `let query = format!( ... )` construction with:

```rust
    let query = build_resample_query(
        &selected_freq,
        &selected_agg,
        &time_col,
        &lat_col,
        &lon_col,
        &val_col,
        time_is_numeric,
    );
```

- [ ] **Step 4: Run tests + clippy**

Run: `cargo test -p eider commands::resample::tests`
Expected: all PASS.
Run: `cargo clippy -p eider --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add cli/src/commands/resample.rs
git commit --no-gpg-sign -m "refactor: extract validate_agg and build_resample_query with tests

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 8: `plot` value-column detection test

**Files:**
- Modify: `cli/src/plot.rs` (append/extend a `#[cfg(test)] mod tests`)

`detect_value_column` is private; test it in-file.

- [ ] **Step 1: Write the tests**

Append to `cli/src/plot.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use duckdb::Connection;

    fn mem(ddl: &str) -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(ddl).unwrap();
        c
    }

    #[test]
    fn detects_non_coordinate_value_column() {
        let c = mem("CREATE TABLE t (time BIGINT, lat DOUBLE, lon DOUBLE, air_temperature DOUBLE);");
        assert_eq!(detect_value_column(&c, "t").unwrap(), "air_temperature");
    }

    #[test]
    fn errors_when_only_coordinates() {
        let c = mem("CREATE TABLE t (time BIGINT, lat DOUBLE, lon DOUBLE);");
        assert!(detect_value_column(&c, "t").is_err());
    }
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p eider plot::tests`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add cli/src/plot.rs
git commit --no-gpg-sign -m "test: unit test for plot value-column auto-detection

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 9: `config` precedence test

**Files:**
- Modify: `cli/src/config.rs` (extend the existing `#[cfg(test)] mod tests`)

- [ ] **Step 1: Add a local-file + env precedence test**

Inside the existing `mod tests` in `cli/src/config.rs`, add:

```rust
    #[test]
    #[serial]
    fn test_local_toml_sets_output_format() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".eider.toml"),
            "output_format = \"json\"\n",
        )
        .unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let config = EiderConfig::load().unwrap();
        std::env::set_current_dir(prev).unwrap();

        assert_eq!(config.output_format.as_deref(), Some("json"));
    }
```

(Requires `tempfile`, already a dev-dependency.)

- [ ] **Step 2: Run**

Run: `cargo test -p eider config::tests`
Expected: PASS (runs serially with the existing env test).

- [ ] **Step 3: Commit**

```bash
git add cli/src/config.rs
git commit --no-gpg-sign -m "test: config loads output_format from local .eider.toml

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Phase 3 — Integration: local-only commands (no eider extension needed)

### Task 10: `resample` integration + correctness

**Files:**
- Create: `cli/tests/resample_test.rs`

- [ ] **Step 1: Write the tests**

Create `cli/tests/resample_test.rs`:

```rust
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

    // Verify the actual aggregated values.
    let conn = duckdb::Connection::open(&out).unwrap();
    let v2020: f64 = conn
        .query_row(
            "SELECT value FROM resampled_data WHERE year(time) = 2020",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!((v2020 - 3.0).abs() < 1e-9, "2020 avg should be 3.0, got {v2020}");
    let v2021: f64 = conn
        .query_row(
            "SELECT value FROM resampled_data WHERE year(time) = 2021",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!((v2021 - 10.0).abs() < 1e-9, "2021 avg should be 10.0, got {v2021}");
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
```

- [ ] **Step 2: Run**

Run: `cargo test -p eider --test resample_test`
Expected: 4 passed.

- [ ] **Step 3: Commit**

```bash
git add cli/tests/resample_test.rs
git commit --no-gpg-sign -m "test: integration tests for resample incl. aggregate correctness

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 11: `search` integration vs mock STAC

**Files:**
- Create: `cli/tests/search_test.rs`

- [ ] **Step 1: Write the tests**

Create `cli/tests/search_test.rs`:

```rust
mod common;
use common::*;
use predicates::prelude::*;

#[tokio::test]
async fn search_json_lists_zarr_uris() {
    let server = mock_stac().await;
    let dir = tempfile::tempdir().unwrap();
    eider(&dir)
        .arg("search")
        .args(["--api", &server.uri()])
        .args(["--collection", "cmip6-cesm2-historical"])
        .arg("--output=json")
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""status":"success""#))
        .stdout(predicate::str::contains("https://example.com/cmip6/tas.zarr"));
}

#[tokio::test]
async fn search_json_rejects_bad_bbox() {
    let server = mock_stac().await;
    let dir = tempfile::tempdir().unwrap();
    eider(&dir)
        .arg("search")
        .args(["--api", &server.uri()])
        .args(["--collection", "cmip6-cesm2-historical"])
        .args(["--bbox", "1,2,3"])
        .arg("--output=json")
        .assert()
        .failure()
        .stdout(predicate::str::contains(r#""status":"error""#));
}

#[test]
fn search_non_interactive_requires_api() {
    let dir = tempfile::tempdir().unwrap();
    eider(&dir)
        .arg("search")
        .args(["--collection", "x"])
        .arg("--output=json")
        .assert()
        .failure()
        .stdout(predicate::str::contains("--api is required"));
}
```

Note: `search`'s `get_selected_api` returns the `--api is required` error in non-interactive mode; since `assert_cmd` runs with a non-tty stdin, `OutputMode` is `Agent`/`AgentJson` and prompts are skipped.

- [ ] **Step 2: Run**

Run: `cargo test -p eider --test search_test`
Expected: 3 passed.

- [ ] **Step 3: Commit**

```bash
git add cli/tests/search_test.rs
git commit --no-gpg-sign -m "test: integration tests for search against in-process mock STAC

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 12: `completions` + global behavior integration

**Files:**
- Create: `cli/tests/cli_global_test.rs`

- [ ] **Step 1: Write the tests**

Create `cli/tests/cli_global_test.rs`:

```rust
mod common;
use common::*;
use predicates::prelude::*;

#[test]
fn help_describes_the_tool() {
    let dir = tempfile::tempdir().unwrap();
    eider(&dir)
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Agentic Spatial Data Engine"));
}

#[test]
fn completions_generate_for_all_shells() {
    for shell in ["bash", "zsh", "fish", "powershell"] {
        let dir = tempfile::tempdir().unwrap();
        eider(&dir)
            .arg("completions")
            .arg(shell)
            .assert()
            .success()
            .stdout(predicate::str::is_empty().not());
    }
}

#[test]
fn json_error_envelope_shape_on_missing_input() {
    // resample with a missing input db, json mode -> structured error envelope.
    let dir = tempfile::tempdir().unwrap();
    eider(&dir)
        .arg("resample")
        .arg("does_not_exist.duckdb")
        .arg("out.duckdb")
        .args(["--freq", "year", "--agg", "avg", "--output=json"])
        .assert()
        .failure()
        .stdout(predicate::str::contains(r#""status":"error""#))
        .stdout(predicate::str::contains(r#""message":"#));
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p eider --test cli_global_test`
Expected: 3 passed.

- [ ] **Step 3: Commit**

```bash
git add cli/tests/cli_global_test.rs
git commit --no-gpg-sign -m "test: integration tests for completions and global JSON error envelope

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Phase 4 — Integration: extension-backed commands

These require `target/debug/eider.duckdb_extension` (build with `cargo duckdb-ext build`) and, for extract/ingest, the DuckDB `spatial` extension (auto-fetched once into `.duckdb_ext_cache`). Run with network available the first time.

### Task 13: `info` integration against the local Zarr

**Files:**
- Create: `cli/tests/info_test.rs`

- [ ] **Step 1: Write the tests**

Create `cli/tests/info_test.rs`:

```rust
mod common;
use common::*;
use predicates::prelude::*;

// The fixture group has one array; pass the explicit array path.
fn air_temp_uri() -> String {
    climate_zarr().join("air_temperature").to_string_lossy().into_owned()
}

#[test]
fn info_json_reports_shape_and_crs() {
    geozarr_ext_path(); // fail loud if the extension isn't built
    let dir = tempfile::tempdir().unwrap();
    eider(&dir)
        .arg("info")
        .arg(air_temp_uri())
        .arg("--output=json")
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""array_shape""#))
        .stdout(predicate::str::contains("938"))
        .stdout(predicate::str::contains("EPSG:4326"));
}

#[test]
fn info_invalid_uri_json_errors() {
    let dir = tempfile::tempdir().unwrap();
    eider(&dir)
        .arg("info")
        .arg("s3://invalid-bucket-that-does-not-exist/data.zarr")
        .arg("--output=json")
        .assert()
        .failure()
        .stdout(predicate::str::contains(r#""status":"error""#));
}
```

Note on the URI: `run_info` calls `ui::prompt_zarr_uri`, which calls `geozarr_core::store::list_arrays`. Pointing directly at `.../air_temperature` resolves to a single array and passes through. If the test shows a "Group containing multiple datasets" error instead, adjust to the group root URI plus `--pin` per the actual `list_arrays` behavior — verify by running the binary manually first: `cargo run -p eider -- info <uri> --output=json`.

- [ ] **Step 2: Confirm the extension is built, then run**

Run: `cargo duckdb-ext build` (only if `target/debug/eider.duckdb_extension` is absent)
Run: `cargo test -p eider --test info_test`
Expected: 2 passed.

- [ ] **Step 3: Commit**

```bash
git add cli/tests/info_test.rs
git commit --no-gpg-sign -m "test: integration tests for info against local Zarr fixture

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 14: `extract` integration (spatial pushdown round-trip)

**Files:**
- Create: `cli/tests/extract_test.rs`

- [ ] **Step 1: Write the tests**

Create `cli/tests/extract_test.rs`:

```rust
mod common;
use common::*;
use predicates::prelude::*;

fn air_temp_uri() -> String {
    climate_zarr().join("air_temperature").to_string_lossy().into_owned()
}

#[test]
fn extract_writes_rows_into_output_db() {
    geozarr_ext_path();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("extracted.duckdb");

    eider(&dir)
        .arg("extract")
        .arg(air_temp_uri())
        .arg(fixture_path("polygon.geojson"))
        .args(["--out", out.to_str().unwrap()])
        .arg("--yes")
        .arg("--output=json")
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""status": "success""#));

    let conn = duckdb::Connection::open(&out).unwrap();
    let n: i64 = conn
        .query_row("SELECT count(*) FROM extracted_data", [], |r| r.get(0))
        .unwrap();
    assert!(n > 0, "extraction should produce rows for the polygon area");
}

#[test]
fn extract_refuses_overwrite_in_json_mode() {
    geozarr_ext_path();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("exists.duckdb");
    std::fs::write(&out, b"x").unwrap();
    eider(&dir)
        .arg("extract")
        .arg(air_temp_uri())
        .arg(fixture_path("polygon.geojson"))
        .args(["--out", out.to_str().unwrap()])
        .arg("--output=json")
        .assert()
        .failure()
        .stdout(predicate::str::contains("already exists"));
}
```

- [ ] **Step 2: Run (network needed on first run for spatial)**

Run: `cargo test -p eider --test extract_test`
Expected: 2 passed. (First run downloads `spatial` into `.duckdb_ext_cache`.)

- [ ] **Step 3: Commit**

```bash
git add cli/tests/extract_test.rs
git commit --no-gpg-sign -m "test: integration test for extract spatial round-trip

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 15: `ingest` + `export` round-trip

**Files:**
- Create: `cli/tests/ingest_export_test.rs`

- [ ] **Step 1: Write the tests**

Create `cli/tests/ingest_export_test.rs`:

```rust
mod common;
use common::*;
use predicates::prelude::*;

#[test]
fn ingest_csv_to_zarr_then_info_reads_it_back() {
    geozarr_ext_path();
    let dir = tempfile::tempdir().unwrap();
    let out_zarr = dir.path().join("ingested.zarr");

    eider(&dir)
        .arg("ingest")
        .arg(fixture_path("ingest_input.csv"))
        .arg(out_zarr.to_str().unwrap())
        .args(["--value-column", "value"])
        .arg("--output=json")
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""status": "success""#));

    assert!(out_zarr.join(".zgroup").exists() || out_zarr.join("value").exists(),
        "ingest should produce a zarr store");

    // Round-trip: info should read the produced store's value array.
    let value_uri = out_zarr.join("value");
    if value_uri.exists() {
        eider(&dir)
            .arg("info")
            .arg(value_uri.to_str().unwrap())
            .arg("--output=json")
            .assert()
            .success()
            .stdout(predicate::str::contains(r#""array_shape""#));
    }
}

#[test]
fn ingest_missing_input_errors() {
    let dir = tempfile::tempdir().unwrap();
    eider(&dir)
        .arg("ingest")
        .arg("missing_input.csv")
        .arg("s3://bucket/out.zarr")
        .arg("--output=json")
        .assert()
        .failure()
        .stdout(predicate::str::contains("does not exist"));
}

#[test]
fn ingest_rejects_invalid_chunks_json() {
    geozarr_ext_path();
    let dir = tempfile::tempdir().unwrap();
    eider(&dir)
        .arg("ingest")
        .arg(fixture_path("ingest_input.csv"))
        .arg(dir.path().join("o.zarr").to_str().unwrap())
        .args(["--chunks", "not-json"])
        .arg("--output=json")
        .assert()
        .failure()
        .stdout(predicate::str::contains("Failed to parse"));
}
```

Note: the exact internal layout of the produced Zarr (group vs. single array, the value array's path) depends on `export`/`MetadataBuilder`. Before finalizing assertions, run `cargo run -p eider -- ingest <csv> <out.zarr> --value-column value --output=json` once and inspect the produced directory; adjust the `value`/`.zgroup` path checks to match reality.

- [ ] **Step 2: Run**

Run: `cargo test -p eider --test ingest_export_test`
Expected: 3 passed (adjust path assertions per the note if the first run reveals a different layout).

- [ ] **Step 3: Commit**

```bash
git add cli/tests/ingest_export_test.rs
git commit --no-gpg-sign -m "test: integration tests for ingest CSV->Zarr and info round-trip

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Phase 5 — Snapshots

### Task 16: Help + command output snapshots

**Files:**
- Modify: `cli/tests/snapshot_test.rs`
- Create (by `insta`): `cli/tests/snapshots/*.snap`

- [ ] **Step 1: Add snapshot tests**

Append to `cli/tests/snapshot_test.rs` (keep the existing `test_cli_help_snapshot`):

```rust
mod common;

fn clean(s: &str) -> String {
    s.lines().map(|l| l.trim_end()).collect::<Vec<_>>().join("\n").replace("eider.exe", "eider")
}

#[test]
fn resample_help_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let assert = common::eider(&dir).args(["resample", "--help"]).assert().success();
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    insta::assert_snapshot!(clean(&out));
}

#[test]
fn extract_help_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let assert = common::eider(&dir).args(["extract", "--help"]).assert().success();
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    insta::assert_snapshot!(clean(&out));
}
```

Note: the existing `test_cli_help_snapshot` uses `Command::cargo_bin` directly; leave it as-is. The new tests use the harness `eider()` for consistent env.

- [ ] **Step 2: Generate and review snapshots**

Run: `cargo test -p eider --test snapshot_test` (creates `.snap.new` files; the new snapshots will "fail" pending review).
Run: `cargo insta review` and accept the snapshots after eyeballing them, OR `cargo insta accept` if confident.
(If `cargo insta` isn't installed: `cargo install cargo-insta`.)

- [ ] **Step 3: Re-run to confirm green**

Run: `cargo test -p eider --test snapshot_test`
Expected: all PASS.

- [ ] **Step 4: Commit**

```bash
git add cli/tests/snapshot_test.rs cli/tests/snapshots/
git commit --no-gpg-sign -m "test: snapshot resample and extract help output

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 17: `plot` ASCII snapshots

**Files:**
- Create: `cli/tests/plot_test.rs`
- Create (by `insta`): `cli/tests/snapshots/plot_test__*.snap`

- [ ] **Step 1: Write the plot snapshot tests**

Create `cli/tests/plot_test.rs`:

```rust
mod common;
use common::*;

fn run_plot(plot_type: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let db = make_extracted_db_numeric_time(&dir);
    let assert = eider(&dir)
        .arg("plot")
        .arg(&db)
        .args(["--plot-type", plot_type])
        .args(["--value", "air_temperature"])
        .assert()
        .success();
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

#[test]
fn plot_hist_snapshot() {
    insta::assert_snapshot!(run_plot("hist"));
}

#[test]
fn plot_line_snapshot() {
    insta::assert_snapshot!(run_plot("line"));
}

#[test]
fn plot_heatmap_snapshot() {
    insta::assert_snapshot!(run_plot("heatmap"));
}

#[test]
fn plot_bad_table_errors() {
    let dir = tempfile::tempdir().unwrap();
    let db = make_extracted_db_numeric_time(&dir);
    eider(&dir)
        .arg("plot")
        .arg(&db)
        .args(["--plot-type", "hist", "--table", "nonexistent_table"])
        .assert()
        .failure();
}
```

Note: `plot` does not load the eider extension (it queries an existing db), so these run without the extension. If a `--plot-type` requires more rows/columns than the synthetic db provides and errors, expand `make_extracted_db_numeric_time` (or add a richer builder) so each plot type has enough data; verify by running `cargo run -p eider -- plot <db> --plot-type heatmap --value air_temperature` first.

- [ ] **Step 2: Generate + review snapshots**

Run: `cargo test -p eider --test plot_test`
Run: `cargo insta review` (accept after inspecting the ASCII output).

- [ ] **Step 3: Confirm green**

Run: `cargo test -p eider --test plot_test`
Expected: 4 passed.

- [ ] **Step 4: Commit**

```bash
git add cli/tests/plot_test.rs cli/tests/snapshots/
git commit --no-gpg-sign -m "test: snapshot plot ASCII output for hist/line/heatmap

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Phase 6 — Interactive tier

### Task 18: Deterministic `search` prompt test + `shell` REPL smoke

**Files:**
- Modify: `cli/tests/interactive_test.rs`

- [ ] **Step 1: Replace the flaky network test with a mock-backed one and add a shell smoke test**

Replace the contents of `cli/tests/interactive_test.rs` with:

```rust
#![cfg(unix)]

mod common;
use common::*;
use rexpect::spawn;

#[tokio::test]
async fn search_interactive_reaches_dataset_prompt() {
    let server = mock_stac().await;
    let bin = env!("CARGO_BIN_EXE_eider");
    // Provide --api so we skip the provider prompt and hit the collection list
    // served deterministically by the mock server.
    let cmd = format!("{} search --api {}", bin, server.uri());
    let mut p = spawn(&cmd, Some(10_000)).unwrap();
    // The collection is zarr-bearing, so we should reach a selection prompt or
    // (if a single result) proceed; either way no network flakiness.
    let res = p.exp_regex("(?i)Select a STAC Collection|Found .* Data URIs|Select a dataset");
    assert!(res.is_ok(), "did not reach an interactive selection prompt");
}

#[test]
fn shell_launches_or_reports_missing_duckdb() {
    if !duckdb_cli_available() {
        eprintln!("skipping shell REPL test: `duckdb` CLI not on PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let db = make_extracted_db_numeric_time(&dir);
    let bin = env!("CARGO_BIN_EXE_eider");
    let cmd = format!("{} shell {}", bin, db.display());
    let mut p = spawn(&cmd, Some(10_000)).unwrap();
    p.exp_regex("(?i)Starting DuckDB shell").unwrap();
    // Run a query and quit the REPL.
    p.send_line("SELECT count(*) FROM extracted_data;").unwrap();
    p.exp_regex("3").unwrap();
    p.send_line(".quit").unwrap();
    p.exp_eof().unwrap();
}
```

Note: `cli/tests/interactive_test.rs` now references `mod common;`; that compiles fine since `common/mod.rs` is shared. The `search` test is `#[tokio::test]` to host the mock server.

- [ ] **Step 2: Run (unix only)**

Run: `cargo test -p eider --test interactive_test`
Expected: `search_interactive_reaches_dataset_prompt` passes; `shell_launches_or_reports_missing_duckdb` passes (or prints the skip notice if `duckdb` isn't installed).

- [ ] **Step 3: Commit**

```bash
git add cli/tests/interactive_test.rs
git commit --no-gpg-sign -m "test: deterministic search prompt test and shell REPL smoke

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Phase 7 — CI wiring

### Task 19: Cache spatial ext + install DuckDB CLI in CI

**Files:**
- Modify: `.github/workflows/pull_request.yaml`

- [ ] **Step 1: Add a DuckDB extension cache and CLI install to the `ci-checks` job**

In `.github/workflows/pull_request.yaml`, inside the `ci-checks` job `steps:`, after the "Cache Cargo build" step, add:

```yaml
      - name: Cache DuckDB extensions
        uses: actions/cache@v4
        with:
          path: .duckdb_ext_cache
          key: ${{ runner.os }}-duckdb-ext-cache-v1

      - name: Install DuckDB CLI (for shell REPL tests)
        shell: bash
        run: |
          if [ "$RUNNER_OS" = "Linux" ]; then
            curl -L https://github.com/duckdb/duckdb/releases/download/v1.1.3/duckdb_cli-linux-amd64.zip -o duckdb.zip
            unzip -o duckdb.zip -d "$HOME/.local/bin"
            echo "$HOME/.local/bin" >> "$GITHUB_PATH"
          elif [ "$RUNNER_OS" = "macOS" ]; then
            brew install duckdb || true
          fi
        continue-on-error: true
```

(The shell REPL test self-skips if `duckdb` is unavailable, so `continue-on-error` keeps CI green if an install hiccups; Windows is intentionally left without the CLI — the `#[cfg(unix)]` interactive tests don't run there anyway.)

- [ ] **Step 2: Validate the workflow YAML locally**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/pull_request.yaml')); print('yaml ok')"`
Expected: `yaml ok`.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/pull_request.yaml
git commit --no-gpg-sign -m "ci: cache DuckDB extensions and install DuckDB CLI for tests

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 20: Full-suite verification + lint gate

**Files:** none (verification only)

- [ ] **Step 1: Run the entire CLI test suite**

Run: `cargo test -p eider`
Expected: all tests pass (unit + integration + snapshot + interactive). On a machine without `duckdb` CLI, the shell test self-skips; on Windows the interactive file is cfg'd out.

- [ ] **Step 2: Lint clean (repo treats warnings as errors)**

Run: `cargo clippy -p eider --all-targets -- -D warnings`
Expected: no warnings.
Run: `cargo fmt --check`
Expected: no diffs (run `cargo fmt` and amend if needed).

- [ ] **Step 3: Coverage sanity (optional, mirrors CI)**

Run: `cargo llvm-cov -p eider --summary-only`
Expected: meaningful coverage of the previously-zero `commands/*` and `duckdb_utils`/`plot` modules.

- [ ] **Step 4: Final commit if fmt changed anything**

```bash
git add -A
git commit --no-gpg-sign -m "style: cargo fmt across new tests

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-review notes (author checklist, already applied)

- **Spec coverage:** every per-command matrix row maps to a task — info(13), extract(14), resample(7,10), search(6,11), ingest/export(15), plot(8,17), shell(18), completions/global(12), config(9), snapshots(16,17), interactive(18), harness/fixtures(1–4), CI(19). ✅
- **Hermeticity:** mock STAC in-process (Task 3), spatial cached via `DUCKDB_EXTENSION_DIRECTORY` (Task 3 env + Task 19 CI cache). ✅
- **Type/name consistency:** harness helpers (`eider`, `mock_stac`, `make_extracted_db_numeric_time`, `geozarr_ext_path`, `fixture_path`, `climate_zarr`, `duckdb_cli_available`) are used with identical names across all test files; `validate_agg`/`build_resample_query` signatures match between definition (Task 7) and tests. ✅
- **Verification-required notes:** three tasks (13 info URI resolution, 15 ingest output layout, 17 plot data sufficiency) include an explicit "run the binary manually first and adjust assertions" instruction, because those depend on extension/export internals not fully visible from the CLI source. These are deliberate verification steps, not placeholders.
