# E2E Docker Test Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create a Docker Compose E2E testing framework that builds the extension in Linux, generates a Zarr dataset, benchmarks queries, and reports performance to PRs via a pre-push hook.

**Architecture:** We use three Docker services: `builder` (compiles the Rust extension into a dedicated `target_docker` dir), `generator` (creates a multi-dimensional Zarr with missing values using Python), and `duckdb` (executes the query and times it). A wrapper script formats the results into Markdown, and a pre-push Git hook updates active PRs.

**Tech Stack:** Docker Compose, Rust, Python (`zarr`, `numpy`), Bash, GitHub CLI.

---

### Task 1: Python Zarr Generator

**Files:**
- Create: `e2e/generate_zarr.py`

- [ ] **Step 1: Write the failing test**
No failing test for script creation.

- [ ] **Step 2: Write minimal implementation**
Create `e2e/generate_zarr.py` to build a 2D Zarr array `[1000, 1000]` with chunks `[100, 100]` and a fill value of `-9999.0`.

```python
import zarr
import numpy as np
import os
import shutil

def main():
    store_path = os.environ.get('ZARR_STORE_PATH', os.path.join(os.path.dirname(__file__), 'data', 'test.zarr'))
    if os.path.exists(store_path):
        shutil.rmtree(store_path)

    # Initialize a Zarr root group
    root = zarr.group(store=store_path)

    # Create a 2D array of floats with a fill value
    z = root.create_dataset(
        'data',
        shape=(1000, 1000),
        chunks=(100, 100),
        dtype='f8',
        fill_value=-9999.0
    )

    # Fill with some data, leaving some as fill_value
    data = np.ones((1000, 1000), dtype='f8') * 42.0
    # Add some fill values
    data[0:100, 0:100] = -9999.0
    z[:] = data

    # Create coordinate arrays
    lat = root.create_dataset('lat', shape=(1000,), chunks=(100,), dtype='f8')
    lat[:] = np.linspace(0, 99.9, 1000)

    lon = root.create_dataset('lon', shape=(1000,), chunks=(100,), dtype='f8')
    lon[:] = np.linspace(0, 99.9, 1000)

    print(f"Generated Zarr array at {store_path}")

if __name__ == '__main__':
    main()
```

- [ ] **Step 3: Commit**
```bash
mkdir -p e2e/data
git add e2e/generate_zarr.py
git commit -m "test: create python zarr generator script"
```

### Task 2: Docker Compose Setup

**Files:**
- Create: `e2e/docker-compose.yml`

- [ ] **Step 1: Write the minimal implementation**
Create `e2e/docker-compose.yml` to orchestrate the build, generation, and querying. We build the Rust extension inside a standard Rust container to ensure Linux ABI compatibility, outputting to a separate target directory to avoid clashing with the host's macOS artifacts.

```yaml
version: '3.8'

services:
  builder:
    image: rust:1.80
    volumes:
      - ../:/app
    environment:
      - CARGO_TARGET_DIR=/app/target_docker
    command: >
      sh -c "apt-get update && apt-get install -y clang cmake
      && cd /app
      && cargo build --release"

  generator:
    image: python:3.12-slim
    volumes:
      - ./data:/data
      - ./generate_zarr.py:/app/generate_zarr.py
    command: sh -c "pip install zarr numpy && python /app/generate_zarr.py"

  duckdb:
    image: ubuntu:22.04
    depends_on:
      - builder
      - generator
    volumes:
      - ./data:/data
      - ../:/app
    command: >
      sh -c "apt-get update && apt-get install -y curl unzip time
      && curl -L -o duckdb.zip https://github.com/duckdb/duckdb/releases/download/v1.1.1/duckdb_cli_linux_amd64.zip
      && unzip duckdb.zip
      && /usr/bin/time -o /app/e2e_benchmark.txt -v ./duckdb -unsigned -c \"LOAD '/app/target_docker/release/libduckdb_geozarr.so'; SELECT SUM(value) FROM read_zarr('/data/test.zarr');\""
```

- [ ] **Step 2: Commit**
```bash
git add e2e/docker-compose.yml
git commit -m "test: add docker-compose configuration for e2e tests"
```

### Task 3: Wrapper Script

**Files:**
- Create: `scripts/run_e2e.sh`

- [ ] **Step 1: Write the minimal implementation**
Create `scripts/run_e2e.sh` to run compose, clean up the output, and generate the markdown report.

```bash
#!/bin/bash
set -e

echo "Running E2E Docker Tests..."
cd e2e
docker-compose up --build --abort-on-container-exit
cd ..

echo "Formatting benchmark results..."
# Extract the relevant timing info from the output
if [ -f e2e_benchmark.txt ]; then
    TIME_STR=$(grep "Elapsed (wall clock) time" e2e_benchmark.txt | awk -F': ' '{print $2}')
    MEM_STR=$(grep "Maximum resident set size" e2e_benchmark.txt | awk -F': ' '{print $2}')

    cat << EOF > e2e_benchmark.md
## 🚀 E2E Performance Benchmark

**Dataset:** 1M elements (1000x1000) Zarr array with coordinates and missing data.
**Query:** \`SELECT SUM(value) FROM read_zarr('/data/test.zarr');\`

- **Wall Clock Time:** $TIME_STR
- **Max Memory (RSS):** $MEM_STR kb

*Generated by automated E2E testing locally.*
EOF
    rm e2e_benchmark.txt
    echo "Benchmark saved to e2e_benchmark.md"
else
    echo "Benchmark failed to generate."
    exit 1
fi
```

- [ ] **Step 2: Make it executable**
```bash
chmod +x scripts/run_e2e.sh
```

- [ ] **Step 3: Commit**
```bash
git add scripts/run_e2e.sh
git commit -m "test: add e2e test execution wrapper script"
```

### Task 4: Git Pre-Push Hook

**Files:**
- Create: `.githooks/pre-push`
- Create: `scripts/setup_hooks.sh`

- [ ] **Step 1: Write the minimal implementation**
Create `.githooks/pre-push` (no `.sh` extension required by Git):

```bash
#!/bin/bash
# Pre-push hook to report benchmark results to active PR

BENCHMARK_FILE="e2e_benchmark.md"

if [ -f "$BENCHMARK_FILE" ]; then
    # Check if we are on a branch with an open PR
    BRANCH_NAME=$(git branch --show-current)
    PR_EXISTS=$(gh pr view "$BRANCH_NAME" --json number -q '.number' 2>/dev/null || echo "false")

    if [ "$PR_EXISTS" != "false" ]; then
        echo "Posting benchmark to PR #$PR_EXISTS..."
        gh pr comment "$BRANCH_NAME" --body-file "$BENCHMARK_FILE"
        echo "Successfully posted benchmark."
    else
        echo "No open PR found for branch $BRANCH_NAME. Skipping benchmark report."
    fi
fi

exit 0
```

Make it executable:
```bash
mkdir -p .githooks
chmod +x .githooks/pre-push
```

Create `scripts/setup_hooks.sh` to configure the repo:
```bash
#!/bin/bash
git config core.hooksPath .githooks
echo "Git hooks configured successfully."
```
Make it executable:
```bash
chmod +x scripts/setup_hooks.sh
```

- [ ] **Step 2: Add to gitignore**
Append `e2e_benchmark.md` and `target_docker/` to `.gitignore`.

```bash
echo "e2e_benchmark.md" >> .gitignore
echo "target_docker/" >> .gitignore
```

- [ ] **Step 3: Commit**
```bash
git add .githooks/pre-push scripts/setup_hooks.sh .gitignore
git commit -m "chore: setup pre-push git hook for reporting benchmark results"
```
