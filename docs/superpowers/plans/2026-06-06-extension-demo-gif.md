# DuckDB Extension Demo GIF Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce a short VHS-generated GIF showing the eider DuckDB extension used directly in SQL (load → inspect → analyze a Zarr array), embedded in the README's "Quick Start (Reading)" section alongside the existing CLI demo GIF.

**Architecture:** A committed VHS tape (`scripts/demo_extension.tape`) drives a plain `duckdb` CLI session over the local `climate_data.zarr` fixture and renders `docs/static/img/demo-extension.gif`. A small README correction fixes the (currently broken) extension-load SQL snippet the GIF sits beside.

**Tech Stack:** VHS (`charmbracelet/vhs` 0.11.0), DuckDB CLI v1.5.2, the eider loadable extension, bash.

---

## Conventions & verified facts

All commands run from the repo root `/Users/danielfisher/repos/zarrduck`. Branch: `docs/extension-demo-gif` (already created; do NOT commit to `main`). Commit messages use Conventional Commits, `--no-gpg-sign`, and end with the `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` trailer.

These facts were verified live against the current `main` (post-#114) and must be relied upon:

- **VHS** is installed (`vhs version 0.11.0`); **DuckDB CLI** is `v1.5.2`, matching the libduckdb the `duckdb` crate bundles (`=1.10502.0`). The CLI version MUST match or the extension won't load.
- The extension builds to `target/debug/eider.duckdb_extension` via:
  `cargo duckdb-ext build -o target/debug/eider.duckdb_extension -d v1.5.2 -- --no-default-features --features loadable-extension`
- **`LOAD` requires an absolute path** ("relative path not allowed in hardened program"). The plan copies the built extension to `/tmp/eider.duckdb_extension` so the visible `LOAD` is short, portable, and machine-independent.
- **`SET allow_unsigned_extensions = true` fails at runtime** ("Cannot change … while database is running"). You MUST launch `duckdb -unsigned`. (The current README snippet shows the broken `SET` form; Task 1 fixes it.)
- The **zarr path argument accepts a relative path** (resolved against the CLI's cwd = repo root), so the queries can use the short `climate_data.zarr/air_temperature`.
- The extension's table functions (post-#114) are **`read_geo`** and **`read_zarr_metadata`** (the old `read_zarr` no longer exists).
- Verified beat-2 output (`SELECT array_shape, chunk_shape, data_type FROM read_zarr_metadata('climate_data.zarr/air_temperature')`):
  `[938, 73, 144]` | `Some(ChunkShape([12, 73, 144]))` | `Float32`. (`crs` is omitted from the demo: it currently reads `UNKNOWN` because the fixture stores CRS at `geozarr.spatial_reference.crs` but the parser expects `geozarr.crs` — a separate known limitation; showing `UNKNOWN` would mislead.)
- Verified beat-3 output (`SELECT lat, AVG(value) AS mean_temp FROM read_geo('climate_data.zarr/air_temperature', lat_min := 30, lat_max := 50) GROUP BY lat ORDER BY lat`): rows `30.0 → 19.0971…`, `32.5 → 16.9257…`, `35.0 → 14.8220…`, … (lat in 2.5° steps).

## File structure

- **Create:** `scripts/demo_extension.tape` — the VHS script (one responsibility: drive + record the extension SQL demo).
- **Create (by VHS):** `docs/static/img/demo-extension.gif` — the rendered artifact (committed).
- **Modify:** `README.md` — fix the "Quick Start (Reading)" SQL snippet and embed the new GIF.

---

## Task 1: Fix the README "Quick Start (Reading)" SQL snippet

The snippet the GIF will sit beside is currently inaccurate: it uses `SET allow_unsigned_extensions = true;` (which errors at runtime) and `read_zarr(...)` (renamed to `read_geo`). Fix it so it reflects the working invocation.

**Files:**
- Modify: `README.md` (the fenced ```sql block under `## Quick Start (Reading)`)

- [ ] **Step 1: Inspect the current snippet**

Run: `sed -n '/## Quick Start (Reading)/,/## Eider CLI/p' README.md`
Expected: shows a ```sql block containing `SET allow_unsigned_extensions = true;`, `LOAD '/path/to/eider_extension.duckdb_extension';`, and `FROM read_zarr(`.

- [ ] **Step 2: Replace the snippet body**

Replace the existing ```sql fenced block under `## Quick Start (Reading)` with exactly:

````markdown
```sql
-- Launch DuckDB allowing unsigned extensions (the flag must be set at startup):
--   duckdb -unsigned

-- Load the extension (LOAD requires an absolute path)
LOAD '/path/to/eider.duckdb_extension';

-- Query a Zarr array directly, aggregating over a spatial bounding box
SELECT
    lat,
    AVG(value) AS mean_temp
FROM read_geo(
    's3://climate-data/temperature.zarr',
    lat_min := 45.0,
    lat_max := 55.0
)
GROUP BY lat;
```
````

- [ ] **Step 3: Verify the edit**

Run: `sed -n '/## Quick Start (Reading)/,/## Eider CLI/p' README.md`
Expected: the block now shows `duckdb -unsigned`, `LOAD '/path/to/eider.duckdb_extension';`, and `FROM read_geo(`; no `SET allow_unsigned_extensions` line and no `read_zarr(`.

- [ ] **Step 4: Commit**

```bash
git add README.md
git commit --no-gpg-sign -m "docs: correct extension load snippet in Quick Start (Reading)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Create the VHS tape

**Files:**
- Create: `scripts/demo_extension.tape`

- [ ] **Step 1: Write the tape**

Create `scripts/demo_extension.tape` with exactly this content:

```tape
Output docs/static/img/demo-extension.gif

Set FontSize 16
Set FontFamily "Geist Mono, Berkeley Mono, Fira Code, JetBrains Mono, Menlo, Apple Color Emoji"
Set Width 1200
Set Height 900
Set Padding 18
Set LineHeight 1.12
Set TypingSpeed 28ms
Set CursorBlink false
Set Framerate 30
Set Theme "Dracula"

Hide
Type "export PS1='❯ '"
Enter
Type "python3 scripts/generate_demo_data.py >/dev/null 2>&1; cargo build -p eider >/dev/null 2>&1; cargo duckdb-ext build -o target/debug/eider.duckdb_extension -d v1.5.2 -- --no-default-features --features loadable-extension >/dev/null 2>&1"
Enter
Sleep 20s
Type "cp target/debug/eider.duckdb_extension /tmp/eider.duckdb_extension"
Enter
Type "source scripts/header.sh"
Enter
Type "clear"
Enter
Show

# Beat 1: Load the extension into a plain DuckDB shell
Hide
Type "header '1. Load the eider extension into DuckDB'"
Enter
Show
Sleep 1s
Type "duckdb -unsigned"
Enter
Sleep 1500ms
Type "LOAD '/tmp/eider.duckdb_extension';"
Enter
Sleep 2s

# Beat 2: Inspect the Zarr array as a SQL table
Hide
Type "header '2. Inspect the array as a SQL table'"
Enter
Show
Sleep 1s
Type "SELECT array_shape, chunk_shape, data_type FROM read_zarr_metadata('climate_data.zarr/air_temperature');"
Enter
Sleep 3s

# Beat 3: Run analytical SQL directly over the Zarr array
Hide
Type "header '3. Run analytical SQL directly over the Zarr array'"
Enter
Show
Sleep 1s
Type "SELECT lat, AVG(value) AS mean_temp FROM read_geo('climate_data.zarr/air_temperature', lat_min := 30, lat_max := 50) GROUP BY lat ORDER BY lat LIMIT 8;"
Enter
Sleep 5s
Type ".exit"
Enter
Sleep 2s
```

Note on the `header` calls: `scripts/header.sh` emits a cyan `# <title>` banner; this matches `scripts/demo.tape`. The header banner is printed at the shell, then the DuckDB session continues — beats 2 and 3 issue `header` from a `Hide` block at the shell level the same way `demo.tape` does between steps. If, when rendering, the `header` call cannot run because the session is inside the DuckDB prompt (not the shell), see Task 3 Step 2's adjustment note.

- [ ] **Step 2: Sanity-check the tape parses (no render yet)**

Run: `head -20 scripts/demo_extension.tape`
Expected: the `Set` directives and `Output docs/static/img/demo-extension.gif` are present and well-formed.

- [ ] **Step 3: Commit the tape**

```bash
git add scripts/demo_extension.tape
git commit --no-gpg-sign -m "docs: add VHS tape for the DuckDB extension demo

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Render and verify the GIF

**Files:**
- Create (by VHS): `docs/static/img/demo-extension.gif`

- [ ] **Step 1: Pre-flight the three SQL beats manually**

Confirm the real output before recording (the recorded session must be correct):

```bash
cargo duckdb-ext build -o target/debug/eider.duckdb_extension -d v1.5.2 -- --no-default-features --features loadable-extension
cp target/debug/eider.duckdb_extension /tmp/eider.duckdb_extension
duckdb -unsigned -cmd "LOAD '/tmp/eider.duckdb_extension'" -c "SELECT array_shape, chunk_shape, data_type FROM read_zarr_metadata('climate_data.zarr/air_temperature'); SELECT lat, AVG(value) AS mean_temp FROM read_geo('climate_data.zarr/air_temperature', lat_min := 30, lat_max := 50) GROUP BY lat ORDER BY lat LIMIT 8;"
```
Expected: beat 2 shows a 1-row table (`[938, 73, 144]` | `Some(ChunkShape([12, 73, 144]))` | `Float32`); beat 3 shows an 8-row table of `lat`/`mean_temp` starting `30.0 | 19.0971…`. If column names differ, update `scripts/demo_extension.tape` to match before rendering.

- [ ] **Step 2: Render the GIF**

Run: `vhs scripts/demo_extension.tape`
Expected: completes without error; `docs/static/img/demo-extension.gif` is created.

Adjustment note: if the rendered GIF shows the `header` banners failing or the `LOAD`/SQL lines not executing inside DuckDB, the most likely cause is `header` being typed while inside the DuckDB prompt. Fix by moving the beat-2 and beat-3 `header` banners to BEFORE `duckdb -unsigned` is launched (print all three banners is not desired) — instead, prefix each query with a DuckDB comment line, e.g. change beat 2's typed line to `Type "-- 2. Inspect the array as a SQL table"` + `Enter` then the SELECT, and similarly for beat 3, and remove their `header`/`Hide`/`Show` blocks. Keep beat 1's `header` at the shell (before launching duckdb). Re-render after the change.

- [ ] **Step 3: Eyeball the GIF**

Open `docs/static/img/demo-extension.gif` (e.g. `open docs/static/img/demo-extension.gif`) and confirm:
- All three beats are legible and the SQL output tables render fully within the 1200×900 frame.
- Timing is comfortable (no output cut off before it can be read).
- The GIF loops cleanly.

If timing/legibility is off, adjust `Sleep`/`Height` in the tape and re-run `vhs scripts/demo_extension.tape`.

- [ ] **Step 4: Commit the GIF**

```bash
git add docs/static/img/demo-extension.gif
git commit --no-gpg-sign -m "docs: add rendered DuckDB extension demo GIF

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Embed the GIF in the README

**Files:**
- Modify: `README.md` (the `## Quick Start (Reading)` section)

- [ ] **Step 1: Add the embed under the Quick Start (Reading) heading**

Insert, immediately after the `## Quick Start (Reading)` line and its following intro sentence (before the ```sql block), the figure:

```markdown
![Querying Zarr directly in DuckDB with the eider extension](docs/static/img/demo-extension.gif)
```

- [ ] **Step 2: Verify**

Run: `grep -n "demo-extension.gif" README.md`
Expected: one match, inside the Quick Start (Reading) section.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit --no-gpg-sign -m "docs: embed DuckDB extension demo GIF in README

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Final verification

**Files:** none (verification only)

- [ ] **Step 1: Confirm artifacts and embeds exist**

```bash
ls -la docs/static/img/demo-extension.gif
grep -c "demo-extension.gif" README.md
grep -c "read_geo" README.md
grep -c "allow_unsigned_extensions" README.md
```
Expected: the GIF file exists (non-zero size); `demo-extension.gif` appears once in README; `read_geo` appears (snippet fixed); `allow_unsigned_extensions` no longer appears in the Quick Start (Reading) snippet (0, or only outside that section if referenced elsewhere — verify the Quick Start block specifically).

- [ ] **Step 2: Confirm the existing CLI GIF is untouched**

Run: `git status --short docs/static/img/`
Expected: only `demo-extension.gif` added; `demo-v2.gif` unchanged.

- [ ] **Step 3: Confirm working tree is clean and the branch is ready**

Run: `git status --short && git log --oneline -5`
Expected: no uncommitted changes from this work; commits for the README fix, tape, GIF, and embed are present.

---

## Self-review notes (author checklist, already applied)

- **Spec coverage:** tape file + settings (Task 2) ✓; hidden setup incl. data gen + extension build + `/tmp` copy (Task 2 setup) ✓; 3-beat storyboard load/inspect/analyze (Task 2) ✓; README "Quick Start (Reading)" embed (Task 4) ✓; verification by manual SQL + render + eyeball (Tasks 3, 5) ✓; deterministic local data ✓.
- **Deviations from spec, with rationale:** (a) `crs` is omitted from beat 2 — it reads `UNKNOWN` due to a known nested-CRS parse limitation and would mislead; the spec's beat 2 listed crs but flagged exact columns as verify-before-recording. (b) Beat 1 launches `duckdb -unsigned` rather than showing `SET allow_unsigned_extensions` — verified that `SET` errors at runtime; the spec flagged this for implementation-time verification. (c) The extension is copied to `/tmp/eider.duckdb_extension` so the visible `LOAD` is portable/short — `LOAD` requires an absolute path.
- **Placeholder scan:** no TBD/TODO; the Task 3 Step 2 "adjustment note" is a concrete fallback with exact edits, not a placeholder.
- **Added scope (justified):** Task 1 fixes the adjacent broken README snippet (`SET`→`-unsigned`, `read_zarr`→`read_geo`) — the GIF lands in that section and shipping a working snippet beside it is correct. This was an implicit non-goal-adjacent improvement; flagged here for the reviewer.
