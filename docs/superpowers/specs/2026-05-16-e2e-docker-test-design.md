# E2E Docker Compose Test Design

**Date:** 2026-05-16
**Status:** Approved

## 1. Purpose & Context
We need a heavier, end-to-end performance test that validates the `duckdb_geozarr` extension under realistic conditions. This test will use Docker Compose to orchestrate Zarr array generation via Python and query execution via DuckDB, isolating the runtime environment. The performance results will be automatically appended to any open Pull Request via a Git hook.

## 2. Architecture: Multi-Service Compose
The testing infrastructure will live in the `e2e/` directory and consist of:

### 2.1 `docker-compose.yml`
Two services connected by a shared local volume (`/data`):
1. **`generator` service:** A Python-based container. It will run a script (`e2e/generate_zarr.py`) using `zarr` and `numpy` to create a large, multi-dimensional Zarr array with edge cases (like fill values and varying chunk sizes).
2. **`duckdb` service:** Based on a standard Linux environment. It mounts the shared `/data` volume and the compiled `.duckdb_extension` file from the host. It will execute a test query using the DuckDB CLI.

### 2.2 Execution Wrapper (`scripts/run_e2e.sh`)
A shell script that manages the orchestration:
1. Compiles the Rust extension for the target architecture (`cargo build --release`).
2. Runs `docker-compose up generator` to seed the shared volume.
3. Runs `docker-compose run duckdb` to execute queries and captures the execution time.
4. Formats the timing output into a Markdown file: `e2e_benchmark.md` on the host.

## 3. Pre-push Git Hook & PR Integration
To fulfill the requirement of reporting the metric to the PR if the test has been run recently:

### 3.1 Hook Implementation
We will add a script to `.git/hooks/pre-push` (or integrate it into an existing hook pipeline).

### 3.2 Reporting Logic
1. The hook will check if `e2e_benchmark.md` exists.
2. It will compare the modification time of `e2e_benchmark.md` against a marker file or the last push timestamp to determine if it has been run since the last push.
3. If it is newer, the hook uses the GitHub CLI (`gh pr view`) to check for an open PR associated with the current branch.
4. If a PR is found, the hook uses `gh pr comment` to post the markdown benchmark results.

## 4. Testing
The E2E test itself acts as the test for this infrastructure. Success is defined as the `run_e2e.sh` script completing successfully, the `e2e_benchmark.md` being generated, and the pre-push hook successfully posting the comment to a test PR.
