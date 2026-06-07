# Docs Workstream D (Guides) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add three task-oriented guides — end-to-end analysis workflow, working with cloud data, and converting & exporting to Zarr — and remove the fictional `COPY … (FORMAT ZARR)` from the docs.

**Architecture:** Two new Markdown guides plus a rewrite of the stale `exporting.md`, all under `docs/docs/usage/`, wired into the "Guides" sidebar category. Docs-only. Runnable commands are verified live against the built CLI + extension over the local sample.

**Tech Stack:** Docusaurus (Markdown + TS sidebar), the `eider` CLI + extension, DuckDB.

---

## Conventions & prerequisites

Work from repo root `/Users/danielfisher/repos/zarrduck`. Conventional Commits, `--no-gpg-sign`, end commit messages with `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.

**DEPENDENCY ON WORKSTREAM C:** the guides cross-link to the CLI Reference pages (`cli_info.md`, `cli_extract.md`, `cli_resample.md`, `cli_plot.md`, `cli_shell.md`, `cli_ingest.md`, `cli_export.md`), which only exist once C is merged. Before building/verifying:

- [ ] **Task 0: Ensure the branch sits on top of Workstream C**

Run: `git log --oneline main..HEAD | head` and `ls docs/docs/usage/cli_info.md`.
If `cli_info.md` does NOT exist (C not yet on this branch's base), rebase onto the updated main:
```bash
git fetch origin main
git rebase origin/main
ls docs/docs/usage/cli_info.md   # must now exist
```
Expected: the `cli_*.md` reference pages are present (so Docusaurus links resolve). Do not proceed to the build gate (Task 6) until they are.

Verified facts (this session): `eider info|extract|resample|plot|shell|ingest|export` all work over the local sample; `eider export` requires 0-based integer coordinate columns; `read_geo`/`read_zarr_metadata` are the SQL functions; the extension registers NO `COPY … FORMAT ZARR` (the old `exporting.md` example is fictional). Build the CLI + extension for verification:
```bash
cargo build -p eider && export PATH="$PWD/target/debug:$PATH"
cargo duckdb-ext build -o target/debug/eider.duckdb_extension -d v1.5.2 -- --no-default-features --features loadable-extension
python3 scripts/generate_demo_data.py   # if climate_data.zarr missing
```
Docs build: `cd docs && (test -d node_modules || npm ci) && npm run build` (`onBrokenLinks: 'throw'`).

## File structure

- Create: `docs/docs/usage/guide_workflow.md`, `docs/docs/usage/guide_cloud.md`.
- Modify: `docs/docs/usage/exporting.md` (rewrite — remove the fictional COPY).
- Modify: `docs/sidebars.ts` — "Guides" category lists the three.

---

## Task 1: Verify the runnable guide commands

**Files:** none (confirms the workflow + ingest/export commands before documenting them).

- [ ] **Step 1: Run the end-to-end workflow**

```bash
export PATH="$PWD/target/debug:$PATH"
rm -f /tmp/d_analysis.duckdb /tmp/d_monthly.duckdb
eider info climate_data.zarr/air_temperature
eider extract climate_data.zarr/air_temperature scripts/demo_region.geojson --out /tmp/d_analysis.duckdb --yes
eider resample /tmp/d_analysis.duckdb /tmp/d_monthly.duckdb --freq month --agg avg
eider plot /tmp/d_analysis.duckdb --plot-type heatmap
```
Expected: `info` prints metadata; `extract` reports success and writes the db; `resample` writes `resampled_data`; `plot` renders an ASCII heatmap. If any command's flags/behavior differ from the workflow guide draft (Task 2), correct the guide.

- [ ] **Step 2: Verify the export path (0-based coordinate contract)**

```bash
duckdb /tmp/d_src.duckdb -c "CREATE TABLE src(t BIGINT, y BIGINT, x BIGINT, value DOUBLE); INSERT INTO src VALUES (0,0,0,1.0),(0,0,1,2.0),(0,1,0,3.0),(0,1,1,4.0);"
rm -rf /tmp/d_out.zarr
eider export --db /tmp/d_src.duckdb --query "SELECT * FROM src" --dest /tmp/d_out.zarr --value-column value
```
Expected: "Export successful!". Confirms the `eider export` syntax used in `exporting.md` (Task 4). (Coordinate columns must be 0-based integer indices.)

No commit (verification only).

---

## Task 2: End-to-end workflow guide

**Files:** Create `docs/docs/usage/guide_workflow.md`.

- [ ] **Step 1: Write the guide**

Create `docs/docs/usage/guide_workflow.md`:

```markdown
---
sidebar_position: 1
---

# End-to-end analysis workflow

This guide chains the `eider` CLI from a raw Zarr array to a finished
visualization, using the sample dataset from the repo. See
[Installation](./installation.md) to set up the CLI and extension, then
generate the sample:

```bash
python3 scripts/generate_demo_data.py   # writes ./climate_data.zarr
```

## 1. Inspect the dataset

Start by checking the array's shape, chunking, type, and CRS:

```bash
eider info climate_data.zarr/air_temperature
```

See [`eider info`](./cli_info.md) for details.

## 2. Extract a region

Materialize the cells intersecting a vector boundary into a local DuckDB file —
only the chunks the polygon touches are fetched:

```bash
eider extract climate_data.zarr/air_temperature scripts/demo_region.geojson \
  --out analysis.duckdb --yes
```

This writes an `extracted_data` table. See [`eider extract`](./cli_extract.md).

## 3. Resample over time

Aggregate the time series to monthly averages:

```bash
eider resample analysis.duckdb monthly.duckdb --freq month --agg avg
```

This writes a `resampled_data` table. See [`eider resample`](./cli_resample.md).

## 4. Visualize and explore

Render an ASCII heatmap in the terminal:

```bash
eider plot analysis.duckdb --plot-type heatmap
```

Or drop into a SQL shell for ad-hoc queries over the extracted data:

```bash
eider shell analysis.duckdb
```

See [`eider plot`](./cli_plot.md) and [`eider shell`](./cli_shell.md).

## Next steps

- Query arrays directly in SQL — see the [SQL Reference](./sql_reference.md).
- Write results back to Zarr — see [Converting & exporting to Zarr](./exporting.md).
```

- [ ] **Step 2: Reconcile with Task 1 Step 1 and commit**

Confirm each command matches what you ran in Task 1 Step 1 (flags, behavior). Then:
```bash
git add docs/docs/usage/guide_workflow.md
git commit --no-gpg-sign -m "docs: add end-to-end workflow guide

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Cloud data guide

**Files:** Create `docs/docs/usage/guide_cloud.md`.

- [ ] **Step 1: Write the guide**

Create `docs/docs/usage/guide_cloud.md`:

```markdown
---
sidebar_position: 2
---

# Working with cloud data

Eider reads Zarr arrays directly from cloud storage via
[Apache OpenDAL](https://opendal.apache.org/) — no download step. The same URIs
work from both the SQL extension and the CLI.

> The examples below hit remote endpoints and require valid credentials /
> network access; they are not runnable from the repo as-is.

## Configure access

Set standard environment variables before querying:

```bash
# S3
export AWS_ACCESS_KEY_ID=…
export AWS_SECRET_ACCESS_KEY=…
export AWS_REGION=us-east-1

# Allow local filesystem reads (when mixing local + remote)
export GEOZARR_ALLOW_PATH=/
```

## Query remote data from SQL

```sql
LOAD '/absolute/path/to/eider.duckdb_extension';

SELECT lat, AVG(value) AS mean_temp
FROM read_geo('s3://my-bucket/data.zarr', lat_min := 45.0, lat_max := 55.0)
GROUP BY lat;
```

`http(s)://` URIs work the same way:

```sql
SELECT * FROM read_zarr_metadata('https://example.com/data.zarr');
```

## Query remote data from the CLI

```bash
eider info s3://my-bucket/data.zarr
eider extract https://example.com/data.zarr ./region.geojson --out analysis.duckdb --yes
```

## Next steps

- [SQL Reference](./sql_reference.md) — `read_geo` parameters and pushdown.
- [CLI Reference](./cli_tui.md) — all commands accept remote URIs.
```

- [ ] **Step 2: Commit**

```bash
git add docs/docs/usage/guide_cloud.md
git commit --no-gpg-sign -m "docs: add cloud data guide

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Converting & exporting to Zarr (rewrite `exporting.md`)

**Files:** Modify `docs/docs/usage/exporting.md`.

- [ ] **Step 1: Rewrite the page (removing the fictional COPY)**

Overwrite `docs/docs/usage/exporting.md`:

```markdown
---
sidebar_position: 3
---

# Converting & exporting to Zarr

Eider writes Zarr two ways, both via the CLI:

- **`eider ingest`** — convert a legacy file (NetCDF, GeoTIFF, CSV with geometry) to a GeoZarr array.
- **`eider export`** — write the result of a DuckDB query to a Zarr array.

## Convert a legacy file

```bash
eider ingest input.geojson out.zarr --value-column value
```

Override automatic chunking with `--chunks`, e.g. `--chunks '{"time": 30}'`.
See [`eider ingest`](./cli_ingest.md).

## Export a query result

`eider export` runs a SQL query and writes the `--value-column` as the array
values; every other column is treated as a **0-based integer coordinate index**.

```bash
eider export \
  --db analysis.duckdb \
  --query "SELECT t, y, x, value FROM gridded" \
  --dest out.zarr \
  --value-column value
```

The coordinate columns (`t`, `y`, `x` above) must be 0-based integer dimension
indices; the array shape is inferred from their distinct counts. See
[`eider export`](./cli_export.md).

## Next steps

- [End-to-end analysis workflow](./guide_workflow.md)
- [Working with cloud data](./guide_cloud.md) — `--dest s3://…` writes to the cloud.
```

- [ ] **Step 2: Verify there is no fictional COPY and the export example is real**

Run: `grep -nE "FORMAT ZARR|COPY \(" docs/docs/usage/exporting.md || echo "clean"`
Expected: `clean`. Confirm the `eider export` invocation matches what you ran in Task 1 Step 2.

- [ ] **Step 3: Commit**

```bash
git add docs/docs/usage/exporting.md
git commit --no-gpg-sign -m "docs: rewrite exporting guide with real ingest/export (drop fictional COPY)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Wire the Guides sidebar

**Files:** Modify `docs/sidebars.ts`.

- [ ] **Step 1: Update the Guides category**

In `docs/sidebars.ts`, replace the `Guides` category's `items` with:

```typescript
      items: [
        'usage/guide_workflow',
        'usage/guide_cloud',
        'usage/exporting',
      ],
```

(Leave the other four categories unchanged.)

- [ ] **Step 2: Commit**

```bash
git add docs/sidebars.ts
git commit --no-gpg-sign -m "docs: wire guides into the sidebar

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: Build verification

**Files:** none (verification only).

- [ ] **Step 1: Confirm C's pages are present (link targets)**

Run: `ls docs/docs/usage/cli_info.md docs/docs/usage/cli_extract.md docs/docs/usage/cli_resample.md docs/docs/usage/cli_plot.md docs/docs/usage/cli_shell.md docs/docs/usage/cli_ingest.md docs/docs/usage/cli_export.md docs/docs/usage/cli_tui.md`
Expected: all exist (if not, complete Task 0's rebase first).

- [ ] **Step 2: Build the docs (gates broken links)**

Run: `cd docs && (test -d node_modules || npm ci) && npm run build 2>&1 | tail -6`
Expected: `[SUCCESS]`, no "Broken link" errors. The guides link to `installation.md`, `sql_reference.md`, `cli_*` pages, and each other — all of which exist. No reference to Engineering pages as links.

- [ ] **Step 3: No fictional/stale content**

Run: `grep -rnE "FORMAT ZARR|COPY \(|read_zarr\(" docs/docs/usage/guide_*.md docs/docs/usage/exporting.md || echo clean`
Expected: `clean`.

- [ ] **Step 4: Confirm scope**

Run: `git diff --name-status main..HEAD`
Expected: `A docs/docs/usage/guide_workflow.md`, `A docs/docs/usage/guide_cloud.md`, `M docs/docs/usage/exporting.md`, `M docs/sidebars.ts`, plus the spec/plan docs — and nothing else (no Rust, no edits to Getting Started / SQL Reference / CLI Reference / engineering pages). (After rebasing onto C, C's commits are in the branch history but not in this diff against the post-C main once merged; verify the *working* changes are only the four files above.)

---

## Self-review notes (author checklist, already applied)

- **Spec coverage:** three guides — workflow (Task 2), cloud (Task 3), converting & exporting (Task 4) ✓; no separate landing ✓; `exporting.md` rewritten in place, fictional `COPY FORMAT ZARR` removed (Task 4 + Task 6 Step 3) ✓; sidebar wiring (Task 5) ✓; C dependency handled via rebase (Task 0, Task 6 Step 1) ✓; runnable commands verified live (Task 1) ✓; cloud examples labeled credential-requiring (Task 3) ✓; build/no-broken-links gate (Task 6) ✓.
- **Placeholders:** none; all guide bodies are complete; the only verification-driven step is reconciling the workflow/export commands against Task 1's live runs.
- **Link safety:** guides link only to existing pages (Getting Started, SQL Reference [B], CLI Reference [C after rebase]); Engineering is not linked.
- **Non-goals honored:** no engineering content (E), no product/code changes, no new reference content, no file moves beyond rewriting `exporting.md`.
