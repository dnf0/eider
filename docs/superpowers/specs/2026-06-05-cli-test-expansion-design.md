# Design: Comprehensive `eider` CLI Test Suite

- **Date:** 2026-06-05
- **Status:** Approved (design); implementation pending
- **Scope:** The `eider` CLI crate (`cli/`). Does not cover the DuckDB extension (`extension/`) or `geozarr_core/` internals beyond what the CLI exercises.

## Goal

Expand `eider` CLI testing across four dimensions the user explicitly wants, all of them:

1. **Regression safety net** — lock in current correct behavior across all 9 subcommands (happy and error paths).
2. **Behavior verification** — prove commands produce correct results on real data, not just exit code 0.
3. **Coverage** — fill the many zero-test modules; lift the existing `llvm-cov` numbers.
4. **Release confidence** — exercise full end-to-end user journeys.

Success = every subcommand has happy-path, error-path, and (where it has logic) correctness tests; the suite is hermetic and green on Linux/Windows/macOS CI; presentation output is locked by snapshots.

## Decisions (from brainstorming)

| Decision | Choice |
|---|---|
| Refactoring tolerance | **Hybrid** — black-box integration for every command, plus targeted extraction of hard-to-reach logic where needed. |
| External dependencies | **Fully hermetic** — local fixtures + in-process mock STAC; no live STAC/S3. |
| Interactive/TTY coverage | **Full** — rexpect prompt + shell REPL tests, plus plot ASCII snapshots. |
| Overall structure | **Layered pyramid** — unit (co-located) + integration (per-command) + snapshot + interactive. |
| DuckDB `spatial` extension | **Cache via `DUCKDB_EXTENSION_DIRECTORY`** — downloads at most once, then offline. |

## Current state (baseline)

CLI surface (`cli/src/main.rs`): `info`, `extract`, `shell`, `export`, `completions`, `search`, `resample`, `plot`, `ingest`, plus global `--output table|json`.

Existing tests:
- `cli/tests/integration_test.rs` — 7 `assert_cmd` tests, almost all error paths.
- `cli/tests/interactive_test.rs` — 1 network-dependent `rexpect` test (flaky by its own admission).
- `cli/tests/snapshot_test.rs` — 1 `insta` snapshot of `--help`.
- Unit tests: `config.rs` (1), `ui.rs` (5). **Zero** tests in `extract`, `info`, `ingest`, `resample`, `search`, `export/*`, `shell`, `duckdb_utils`, `plot.rs` (754 lines).

Dev-deps already present: `assert_cmd`, `predicates`, `insta`, `serial_test`, `tempfile`, `rexpect` (unix). To add: `wiremock`.

## Infrastructure realities

1. **eider extension dependency.** `info`/`extract`/`export`/`ingest` call `read_zarr_metadata()`/`read_zarr()`/`plan_read_zarr()` from the compiled `eider.duckdb_extension`, located by `duckdb_utils::load_geozarr_extension` (searches `./target/debug/`). Tests for these require `cargo duckdb-ext build` first. CI already does this before `cargo test`.
2. **DuckDB `spatial` extension.** `extract` and `ingest` run `INSTALL spatial; LOAD spatial;`, which fetches from DuckDB's network repo on first use. Mitigation: pin `DUCKDB_EXTENSION_DIRECTORY` to a CI-cached dir so it downloads at most once, then runs offline.
3. **`shell` external CLI.** `shell` shells out to a `duckdb` binary on `PATH`. REPL tests are gated on its presence; CI adds an install step.
4. **`search` is already well-factored.** `build_stac_query`, `is_supported_asset`, `extract_assets`, `parse_search_results`, `output_json_results` are pure and unit-testable with no refactor. `--output=json` mode performs no network URI resolution, so it is fully hermetic against a mock STAC.

## Architecture

### Test taxonomy

| Layer | Location | Covers |
|---|---|---|
| Unit | co-located `#[cfg(test)] mod tests` (per AGENTS.md: co-locate Rust unit tests) | pure logic: STAC parse, query/SQL building, pin/bbox/chunk parsing, column detection, output formatting |
| Integration | `cli/tests/<command>_test.rs`, one per command | each command end-to-end via `assert_cmd` against fixtures + mock STAC |
| Snapshot | `insta`, within integration tests | exact table / JSON / plot-ASCII / help output |
| Interactive | `cli/tests/interactive_test.rs` (`#[cfg(unix)]`, `rexpect`) | `inquire` prompt flows + `shell` REPL |

### Shared harness — `cli/tests/common/mod.rs`

- `mock_stac()` — in-process `wiremock` server serving canned `/collections` + `/search` payloads ported from `scripts/mock_stac.py`. Cross-platform, no Python, deterministic.
- `fixture_path(name)` — resolves `cli/tests/fixtures/…` and repo-root `climate_data.zarr` via `CARGO_MANIFEST_DIR`.
- `geozarr_ext_path()` — locates the built `eider.duckdb_extension`; **fails loud** with a "run `cargo duckdb-ext build` first" message if absent.
- `make_extracted_db(dir)` — builds a fixture `.duckdb` with a synthetic `extracted_data(time, lat, lon, value)` table directly via a duckdb `Connection` (no extension needed). Feeds `resample`/`plot`/`shell` tests deterministically.
- `eider()` — `assert_cmd::Command` preconfigured with a `tempfile::TempDir` working dir and `DUCKDB_EXTENSION_DIRECTORY` set to the cached dir.

### Fixtures — `cli/tests/fixtures/`

- `climate_data.zarr` (repo root, existing) — drives `info`/`extract`/`export`.
- `polygon.geojson` — tiny polygon over the fixture's lat/lon range for `extract`.
- `ingest_input.csv` — small lon/lat/value CSV for `ingest` (deterministic; avoids the 29 MB `air.mon.mean.nc`).
- Synthetic `extracted_data` db — generated at runtime, not committed.

## Per-command test matrix

- **`info`** — unit: `format_pins`, URI escaping. Integration: local zarr → table + JSON snapshot, `--pin`, invalid URI (exists), missing-metadata error.
- **`extract`** — unit: `format_pins_where`, `print_extraction_plan` time/volume math, overwrite-protection logic. Integration: zarr+polygon → output-db row-count assertions, `--out`/`default_out`, `--yes` overwrite, non-interactive overwrite-abort, stdin `-` URI, JSON envelope.
- **`resample`** — unit: `get_freq_and_agg` validation (allowed/invalid agg, json-mode missing-flag errors), `date_trunc` query builder, numeric-time `to_timestamp` branch. Integration: synthetic db → resampled row counts + **aggregate correctness** (monthly avg equals hand-computed), overwrite protection, missing input (exists).
- **`search`** — unit: `build_stac_query` (bbox parse, 4-coord validation, datetime), `is_supported_asset` (zarr/cog/neither), `parse_search_results` (features vs collection assets, description truncation). Integration vs mock STAC: `--output=json` URI list, bad bbox, non-interactive `--api` required, API error status (exists).
- **`ingest`** — unit: `auto_calculate_chunks`, value-column fallback, `--chunks` JSON merge + invalid-JSON error. Integration: CSV → zarr round-trip verified by reading back with `info`, missing input (exists).
- **`export`** — unit: schema/metadata builders in `export/`. Integration: query → zarr, then `info` round-trip; invalid query error.
- **`plot`** — integration: synthetic db → **snapshot** each `--plot-type` (hist/heatmap/line), auto value-column detection, `--group-by`, `--pin`, bad-table error.
- **`shell`** — interactive: launch, run a `SELECT`, `.quit`; assert version-gate notice path. Gated on `duckdb` CLI.
- **`completions`** — bash (exists) + zsh/fish/powershell smoke.
- **Global** — `--output` precedence over config; JSON error envelope (`{"status":"error","message":…}`) shape across commands; `--help`/subcommand help snapshots; config loading (extend `config.rs`).

## Snapshot strategy

`insta` snapshots in `cli/tests/snapshots/`, normalizing volatile content (versions, absolute paths, timestamps, trailing whitespace, `.exe`) via redactions/filters. Covers help, per-command table & JSON output, and all plot renderings.

## Interactive tier

`#[cfg(unix)]` + `rexpect`. Replace the network-dependent search test with one driven against the mock STAC (deterministic), plus a `shell` REPL smoke test (gated on `duckdb` CLI). One thin test per prompt path — smoke, not exhaustive.

## CI integration

`pull_request.yaml` already builds the extension then runs `cargo test --no-default-features` on all three OSes. Additions:
- Cached `DUCKDB_EXTENSION_DIRECTORY` (warms `spatial` once).
- DuckDB-CLI install step (for `shell` REPL tests).
- `wiremock` dev-dependency.
- Existing `llvm-cov` job reports the coverage lift.

## Phasing

Each phase ends green and is independently mergeable:

1. **Harness + fixtures** — `common/mod.rs`, fixture files, `wiremock` dep, extension/`spatial` wiring.
2. **Unit tests** — all pure logic across modules (cheapest coverage, no infra).
3. **Integration: local-only commands** — `resample`, `plot`, `search --output=json`, `completions`, config/global.
4. **Integration: extension-backed** — `info`, `extract`, `export`, `ingest` + correctness assertions.
5. **Snapshots** — across commands.
6. **Interactive tier** — search prompts, shell REPL.

## Non-goals

- Testing the DuckDB extension's internal Zarr decoding (covered by `extension/`'s own tests).
- Live network/cloud tests against real STAC APIs or S3 (explicitly excluded as flaky).
- Performance/benchmark assertions (separate concern).
- Property-based/fuzz testing (YAGNI for this pass).
