# Zonal-Stats Kernel Head-to-Head — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** A reproducible Python harness that benchmarks the zonal-stats kernel (warm, local COG) for eider/DuckDB vs exactextract vs rasterstats across both regimes, with a correctness gate that runs before timing, and an honest results section in the benchmarks docs.

**Architecture:** One orchestrator script `scripts/bench_zonal_headtohead.py` with four pieces — data generation, three contender runners (each `(case, convention, metric) → {poly_id: value}`), a correctness gate (align + assert agreement per matching convention), and a timing loop (median of reps). The **controller runs the full-scale harness and captures real numbers**; the docs section is written from captured output, never estimated.

**Tech Stack:** Python 3.12, duckdb==1.5.2 (ABI-matched to the eider extension) + the eider loadable extension + spatial, exactextract==0.3.0, rasterstats==0.21.0, rasterio==1.5.0, geopandas==1.1.3, shapely, pyarrow, numpy.

---

## Conventions & prerequisites

Repo root `/Users/danielfisher/repos/zarrduck`, branch `bench/zonal-headtohead`. Conventional Commits; `--no-gpg-sign`; trailer `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`. Never `git add -A`.

**Venv:** `python3 -m venv /tmp/bench_venv && source /tmp/bench_venv/bin/activate && pip install -r scripts/bench_requirements.txt`. The eider extension is at `target/debug/eider.duckdb_extension` (build with `cargo duckdb-ext build -o target/debug/eider.duckdb_extension -d v1.5.2 -- --no-default-features --features loadable-extension` if missing). Run the harness with `GEOZARR_ALLOW_PATH=<data dir>` set.

**Confirmed facts (feasibility spike):**
- Python `duckdb` is v1.5.2; `conn.execute("LOAD '<abs>/eider.duckdb_extension'"); conn.execute("INSTALL spatial; LOAD spatial;")` works (`read_geo` registered). Use `duckdb.connect(config={'allow_unsigned_extensions': True})`.
- `from exactextract import exact_extract` (0.3.0); `from rasterstats import zonal_stats` (0.21.0).
- eider has first-class local COG support (reads `.tif`).

**CRITICAL — verify the real `read_geo` output for a COG before writing the join.** Run `DESCRIBE SELECT * FROM read_geo('<file>.tif')` and inspect the actual column names (coords may be `lon/lat` or `x/y`, value column name may differ). Build the zonal SQL against the REAL columns. Do not assume.

**CRS:** generate the raster + polygons in a **projected metric CRS with square cells** (e.g. EPSG:3857 or a custom equal-area grid) so area weights are meaningful and consistent across tools. All polygons and the raster share one CRS.

---

## Task 1: requirements + data generator

**Files:** Create `scripts/bench_requirements.txt`, `scripts/bench_zonal_headtohead.py` (data-gen portion + `__main__` stub).

- [ ] **Step 1: `scripts/bench_requirements.txt`**
```
# Zonal head-to-head benchmark deps. Setup:
#   python3 -m venv /tmp/bench_venv && source /tmp/bench_venv/bin/activate
#   pip install -r scripts/bench_requirements.txt
duckdb==1.5.2
exactextract==0.3.0
rasterstats==0.21.0
rasterio==1.5.0
geopandas==1.1.3
shapely
pyarrow
numpy
```

- [ ] **Step 2: Data generator (write the failing test first)**

In `scripts/bench_zonal_headtohead.py`, implement `generate_data(out_dir, regime, n_polys, seed=42)`:
- **regime "fine"** (R2): a `H×W` (default 2000×2000) float32 array = smooth gradient + mild noise: `base = np.add.outer(np.linspace(0,50,H), np.linspace(0,50,W)); field = base + rng.normal(0, 2, (H,W))`. Write a tiled COG GeoTIFF via rasterio with a projected CRS (`EPSG:3857`) and a square-pixel `Affine` (e.g. 30 m pixels): `transform = Affine(30, 0, x0, 0, -30, y0)`. Footprints = **diamonds** radius ≈ 5 px centered at random in-bounds points: `Polygon([(cx-r,cy),(cx,cy+r),(cx+r,cy),(cx,cy-r)])` in CRS coords. `GeoDataFrame({'poly_id': range(n)}, geometry=...)`, write `polys.parquet` (GeoParquet) and `polys.geojson`.
- **regime "coarse"** (R1): `200×200` array; polygons are **sub-cell** squares (side ≈ 0.3 px) at random centers.
- Write the raster as `grid.tif`. Return paths `{raster, parquet, geojson, transform, crs, shape}`.

Test `tests_bench/test_generate.py` (or an inline `if __name__` self-check; a pytest file is cleaner):
```python
def test_generate_fine(tmp_path):
    from scripts.bench_zonal_headtohead import generate_data
    d = generate_data(tmp_path, "fine", 100, seed=1)
    import rasterio, geopandas as gpd
    with rasterio.open(d["raster"]) as r:
        assert r.count == 1 and r.crs.to_epsg() == 3857 and r.width == 2000
    g = gpd.read_parquet(d["parquet"])
    assert len(g) == 100 and "poly_id" in g.columns
```

- [ ] **Step 3: Run** `python -m pytest tests_bench/test_generate.py -q` (venv active) → PASS. (Use a SMALL raster override for the test, e.g. `generate_data(..., shape=(2000,2000))` default but the test can pass `n_polys=100`; keep raster gen fast — 2000² float32 is 16 MB, fine.)

- [ ] **Step 4: Commit** `git add scripts/bench_requirements.txt scripts/bench_zonal_headtohead.py tests_bench/` then commit `feat(bench): synthetic COG + polygon generator for zonal head-to-head`.

---

## Task 2: contender runners

**Files:** Modify `scripts/bench_zonal_headtohead.py`.

Each runner returns a dict `{poly_id: float}` for a given `(raster, polys, convention, metric)`. `convention ∈ {centroid, all_touched, area_weighted}`, `metric ∈ {max, mean, count}`.

- [ ] **Step 1: rasterstats runner**
```python
from rasterstats import zonal_stats
def run_rasterstats(raster, geojson_path, convention, metric, poly_ids):
    if convention == "area_weighted":
        raise NotImplementedError("rasterstats has no exact area weighting")
    all_touched = (convention == "all_touched")  # False == centroid (GDAL center-rasterize)
    res = zonal_stats(geojson_path, raster, stats=[metric], all_touched=all_touched, nodata=None)
    return {pid: (r[metric] if r[metric] is not None else float("nan")) for pid, r in zip(poly_ids, res)}
```

- [ ] **Step 2: exactextract runner** (verify the 0.3.0 API by a one-off call first)
```python
from exactextract import exact_extract
def run_exactextract(raster, gdf, convention, metric):
    # exactextract weights by coverage fraction. 'mean' => area-weighted mean;
    # 'max'/'min' consider all intersecting cells (coverage>0) => all_touched MAX.
    op = {"max": "max", "mean": "mean", "count": "count"}[metric]
    df = exact_extract(raster, gdf, [op], output="pandas", include_cols=["poly_id"])
    col = [c for c in df.columns if c != "poly_id"][0]
    return dict(zip(df["poly_id"], df[col]))
# Mapping: exactextract is used for all_touched (max) and area_weighted (mean). Not centroid.
```
> Confirm against 0.3.0: column naming (e.g. `band_<op>` vs `<op>`), whether it accepts a GeoDataFrame directly, and that `count`/`mean` semantics are coverage-weighted. Adjust to the real API; keep the mapping intent.

- [ ] **Step 3: eider runner** (discover columns first!)
```python
import duckdb, os
def eider_conn():
    c = duckdb.connect(config={"allow_unsigned_extensions": True})
    c.execute(f"LOAD '{os.path.abspath('target/debug/eider.duckdb_extension')}'")
    c.execute("INSTALL spatial; LOAD spatial;")
    return c
```
Discover the read_geo columns for the COG (`DESCRIBE SELECT * FROM read_geo('grid.tif')`) → name them `XC, YC, VC` (coord-x, coord-y, value). Build per-convention SQL, pushing the polygons' bbox into read_geo and deriving the cell step from the read (as in `mcp zonal_stats`):
```sql
SET VARIABLE bb = (SELECT ST_Extent_Agg(geom) FROM ST_Read('polys.parquet'));
WITH field AS (
  SELECT {XC} AS x, {YC} AS y, {VC} AS v FROM read_geo('grid.tif',
    lon_min:=ST_XMin(getvariable('bb')), lat_min:=ST_YMin(getvariable('bb')),
    lon_max:=ST_XMax(getvariable('bb')), lat_max:=ST_YMax(getvariable('bb')))),
step AS (SELECT (max(x)-min(x))/nullif(count(DISTINCT x)-1,0) dx,
                (max(y)-min(y))/nullif(count(DISTINCT y)-1,0) dy FROM field)
SELECT v.poly_id, {AGG} AS metric
FROM ST_Read('polys.parquet') v JOIN field z, step s
ON {PREDICATE}
GROUP BY v.poly_id;
```
- centroid: `PREDICATE = ST_Contains(v.geom, ST_Point(z.x, z.y))`, `AGG = max(z.v)|avg(z.v)|count(*)`
- all_touched: `PREDICATE = ST_Intersects(v.geom, ST_MakeEnvelope(z.x-s.dx/2, z.y-s.dy/2, z.x+s.dx/2, z.y+s.dy/2))`
- area_weighted (mean only): weighted `sum(z.v*ST_Area(ST_Intersection(v.geom, cell_box)))/sum(ST_Area(ST_Intersection(v.geom, cell_box)))` with the same `ST_Intersects` join.
- Also implement `run_eider_indexjoin(...)` for Regime 1: arithmetic cell-index equi-join (`round((x-x0)/dx)` ↔ cell index), point-model, labeled.

Return `{poly_id: metric}` from the fetched rows.

- [ ] **Step 4: Smoke test each runner** on a tiny case (regime "coarse", 50 polys) in `tests_bench/test_runners.py`: each returns a dict keyed by all poly_ids with finite values for `max`. Run → PASS. Commit `feat(bench): eider/exactextract/rasterstats runners`.

---

## Task 3: correctness gate

**Files:** Modify `scripts/bench_zonal_headtohead.py`, add `tests_bench/test_correctness.py`.

- [ ] **Step 1: Implement `assert_agree`**
```python
import math
def assert_agree(a: dict, b: dict, name_a, name_b, abs_tol):
    ids = set(a) & set(b)
    assert ids, f"{name_a}/{name_b}: no overlapping poly_ids"
    diffs = [(i, a[i], b[i]) for i in ids
             if not (math.isnan(a[i]) and math.isnan(b[i]))
             and abs((a[i] or 0) - (b[i] or 0)) > abs_tol]
    maxd = max((abs((a[i] or 0)-(b[i] or 0)) for i in ids
                if not (math.isnan(a[i]) and math.isnan(b[i]))), default=0.0)
    return {"agree": not diffs, "max_abs_diff": maxd, "n_compared": len(ids),
            "n_mismatch": len(diffs), "examples": diffs[:5]}
```

- [ ] **Step 2: Correctness test** (regime "fine", small n e.g. 500): assert
  - `centroid/max`: eider ≈ rasterstats(all_touched=False) within tol (MAX exact → tol ~1e-4).
  - `all_touched/max`: eider ≈ rasterstats(all_touched=True) ≈ exactextract(max) within tol.
  - `all_touched/mean`: eider ≈ rasterstats(all_touched=True) within a looser tol (float accumulation).
  - `area_weighted/mean`: eider ≈ exactextract(mean). Pick tol from a first run; if the gate exposes a genuine convention mismatch, **that is the finding** — investigate (cell-box edges, half-open intervals, coverage at boundary) and reconcile or document precisely.
  Run → must PASS (or produce a documented, understood discrepancy). Commit `test(bench): cross-tool correctness gate`.

> The whole benchmark's credibility rests here. Do not relax a tolerance to force agreement without understanding *why* it differs. Centroid/all-touched MAX should match near-exactly; area-weighted MEAN should match exactextract closely. If rasterstats all_touched MEAN differs from eider, explain (rasterstats counts a touched cell fully; eider all_touched does too → should match; area weighting is the only fractional one).

---

## Task 4: timing harness + results emit

**Files:** Modify `scripts/bench_zonal_headtohead.py`.

- [ ] **Step 1: Timing loop** — `time_call(fn, reps=3, warmup=1)` using `time.perf_counter`, return median seconds. Each contender's read of the warm local COG is included; exclude data-gen and the one-time `LOAD extension`/first-GDAL-open (do the warmup outside timing).

- [ ] **Step 2: Driver** — `run_matrix(cases, counts)` over regime × convention × metric × n × contender (only valid combos: skip rasterstats area_weighted; eider index-join only R1). For each: run correctness gate (once per convention/case at a fixed n), then time. Respect a `--budget-seconds` per call; if exceeded, mark `skipped (budget)`. Emit:
  - stdout table (human readable),
  - `--json <path>` machine-readable results (so the controller/doc step consumes real numbers),
  - environment block: machine, `platform`, and all lib versions (`duckdb.__version__`, `exactextract.__version__`, etc.).

- [ ] **Step 3: CLI** — `argparse`: `--out-dir`, `--counts 10000 100000 1000000`, `--reps`, `--budget-seconds`, `--json`, `--regime {fine,coarse,both}`, `--quick` (tiny sizes for CI/self-test). Running `--quick` end-to-end must complete and print a table + pass the gate. Commit `feat(bench): timing harness + results matrix emit`.

---

## Task 5: Controller runs full-scale (NOT a subagent)

- [ ] The controller runs `python scripts/bench_zonal_headtohead.py --counts 10000 100000 1000000 --reps 3 --json bench_results.json` (with venv + `GEOZARR_ALLOW_PATH` + extension built), capturing **real** numbers for both regimes, all valid convention/metric/contender cells. Confirm the correctness gate passed. Keep `bench_results.json` for the doc step (do not commit the JSON unless small/useful; the doc carries the numbers).

---

## Task 6: Docs section

**Files:** Modify `docs/docs/engineering/benchmarks.*` (check the actual file extension/structure first).

- [ ] **Step 1:** Add a section "Head-to-head: eider vs the raster-zonal stack" written **from the captured `bench_results.json`** — a table per regime (regime × convention × metric × N) with seconds per contender and the winner; a correctness-agreement statement (max abs diff per convention); capability gaps (rasterstats: no area weighting); the Regime-1 eider index-join row labeled "point-model, valid only sub-cell"; **honest verdict including where eider loses**; the read-pruning advantage flagged as the separate (non-benchmarked-here) axis, cross-linking `spatial_pruning.mdx`; reproduction instructions (venv + command); single-machine/synthetic caveat + versions.
- [ ] **Step 2:** `cd docs && npm run build 2>&1 | tail -5` → `[SUCCESS]`, no broken links. Commit `docs: head-to-head zonal benchmark vs exactextract/rasterstats`.

---

## Task 7: Verification

- [ ] **Step 1:** `python scripts/bench_zonal_headtohead.py --quick` runs clean end-to-end (gate passes, table prints).
- [ ] **Step 2:** `cd docs && npm run build` → `[SUCCESS]`.
- [ ] **Step 3: Scope** — `git diff --name-status main..HEAD` shows only: `scripts/bench_zonal_headtohead.py`, `scripts/bench_requirements.txt`, `tests_bench/**`, `docs/docs/engineering/benchmarks.*`, and the spec/plan. No production Rust/extension changes.

---

## Self-review notes

- **Correctness-before-timing** is enforced (Task 3 gate runs before Task 4 timing) and the controller — not a subagent — captures the headline numbers (Task 5), so the doc can't report invented results.
- **Honest mapping** of conventions across tools is explicit (centroid↔rasterstats default; all_touched↔rasterstats all_touched + exactextract max; area_weighted↔exactextract mean; rasterstats area-weight = documented gap).
- **No silent caps:** budget-skips are labeled; every run cell is reported.
- **read_geo column discovery** is called out as a required verification (not assumed), as is the exactextract 0.3.0 API.
- **CRS** is projected/square-pixel so area weighting is meaningful and cross-tool-consistent.
