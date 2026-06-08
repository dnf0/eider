# Design: zonal-stats kernel head-to-head (eider vs exactextract vs rasterstats)

- **Date:** 2026-06-08
- **Status:** Approved (design); implementation pending
- **Scope:** A reproducible benchmark harness comparing the **zonal-statistics kernel** on a warm, local Cloud-Optimized GeoTIFF for three contenders — eider/DuckDB, exactextract, rasterstats — across both asset/cell regimes, with a **correctness cross-check that gates timing**. Results + honest commentary written into the existing `engineering/benchmarks` docs page.

## Why

The `engineering/zonal_stats.mdx` + `scripts/bench_polygons.sql` work measured eider/DuckDB's zonal patterns *internally* (which convention, which join shape) on synthetic data. It did **not** measure eider against the established raster-zonal stack. The open, honest question: for many small polygons over a grid, does eider's read-prune-then-DuckDB-spatial-join approach actually beat purpose-built tools on the **compute kernel** — exactextract (C++, exact coverage-fraction area weighting) and rasterstats (the Python baseline)? This benchmark answers it with numbers, on the axis where eider is *least* certain to win (the kernel itself, warm + local), so the result is credible rather than self-flattering.

The complementary story — eider's cloud-native **partial read** advantage (pruned chunk reads from remote Zarr/COG/STAC) — is explicitly a **non-goal here** (separate axis); this benchmark isolates the kernel by giving every tool the same warm local raster.

## Confirmed environment (feasibility spike, 2026-06-08)

- Python `duckdb` package is **v1.5.2**, exactly matching the eider extension's libduckdb ABI → eider is drivable from the same Python process (`LOAD '<abs>/eider.duckdb_extension'; INSTALL spatial; LOAD spatial;`). Verified `read_geo` + spatial load.
- `exactextract==0.3.0`, `rasterstats==0.21.0`, `rasterio==1.5.0`, `geopandas==1.1.3`, `shapely`, `pyarrow`, `numpy` install from wheels and import (GDAL 3.11.1 present). No source builds needed.
- eider CLI + extension built at `target/debug/`. duckdb CLI v1.5.2 on PATH.

## Data (synthetic, seeded, generated in-repo)

A generator produces deterministic inputs (numpy seeded; fixed transform/CRS EPSG:4326 or a projected metric CRS for honest area weights — see note):

- **Fine COG** (Regime 2 — asset spans many cells): ~2000×2000 float32 GeoTIFF, COG profile (tiled, overviews optional). Value field = a smooth spatial **gradient + mild noise** (not pure white noise) so convention differences in MEAN are realistic, not understated (the `bench_polygons.sql` caveat). Footprints = **diamonds** (rotated squares, ~5-cell radius ≈ ~100 cells), so bbox ≠ footprint and conventions genuinely differ.
- **Coarse COG** (Regime 1 — asset ≪ cell): ~200×200 GeoTIFF; polygons are **sub-cell** (each lands in ~1 cell).
- **Polygon counts:** scaled `10_000 / 100_000 / 1_000_000`, capped per-contender by a wall-clock budget (area-weighted at 1M may be skipped if it exceeds budget — *logged as skipped, not silently dropped*). Written as **GeoParquet** (eider reads via `ST_Read`) and reused by geopandas/rasterstats/exactextract in-memory.

**CRS note (correctness):** area-weighting is only meaningful in an equal-area / projected metric space. The synthetic grid uses a **projected CRS with square cells** (e.g. a local metric grid), and all polygons share it, so `ST_Area`/exactextract coverage and the cell geometry are consistent. This avoids the lat/lon-degree area distortion that would make "area-weighted" disagree for non-substantive reasons.

## Contenders & like-for-like conventions

All three read the **same local COG**; we compare matching definitions, and **verify numbers agree before reporting timing**:

| Convention | eider | rasterstats | exactextract | Notes |
|---|---|---|---|---|
| **centroid** (cell-center-in-polygon) | `ST_Contains(poly, ST_Point(lon,lat))` | `all_touched=False` (GDAL center-rasterize) | — (coverage-based; skip) | rasterstats default ≡ eider centroid |
| **all-touched** (cell box intersects) | `ST_Intersects(poly, cell_box)` | `all_touched=True` | coverage>0 (`min_coverage_frac≈0`) | MAX + MEAN |
| **area-weighted MEAN** | `ST_Intersection` area weights | — (no native area weighting) | coverage-fraction weighted mean (native) | rasterstats = **capability gap**, not a loss |

Metrics: `max`, `mean`, `count` where defined. Per matching convention, align per-polygon outputs and assert `max |Δ|` within tolerance (separate abs tol for MAX vs MEAN; document tol). Disagreement is a **reported finding**, not hidden.

**Regime 1 reporting:** eider is shown **two ways** — (a) the spatial zonal join (apples-to-apples with the polygon tools), and (b) the **arithmetic index equi-join** point-model shortcut, *clearly labeled "valid only when asset ≪ cell."*

## Measurement protocol

- Warm cache; per contender × case: 1 warmup, then median of N≥3 reps via `time.perf_counter` around the **compute call only** (each tool's intrinsic read of the warm local COG is included — that's the kernel as a user invokes it; one-time process/extension/GDAL init is excluded).
- Record machine + all tool/lib versions in the output. State the single-machine, synthetic-data caveat prominently.

## Components

1. **`scripts/bench_zonal_headtohead.py`** — one orchestrator: generate (or reuse cached) data → run each contender per case → correctness gate → timing → emit a results table (stdout + a machine-readable JSON/markdown fragment).
2. **`scripts/bench_requirements.txt`** — pinned versions (`duckdb==1.5.2`, `exactextract==0.3.0`, `rasterstats==0.21.0`, `rasterio==1.5.0`, `geopandas==1.1.3`, `shapely`, `pyarrow`, `numpy`) + a one-line venv setup comment.
3. **Docs:** a new "Head-to-head vs the raster-zonal stack" section in `docs/docs/engineering/benchmarks.*` — the results matrix (regime × convention × N), correctness-agreement confirmation, **winners called honestly including where eider loses**, capability gaps, the eider read-pruning story flagged as the separate axis (cross-link `spatial_pruning.mdx`), reproduction instructions, environment caveat. Docs build stays green.

## Honesty guardrails (non-negotiable)

- Timing is reported **only after** the correctness gate passes for that convention; mismatches are surfaced.
- The controller (not a subagent) runs the final full-scale harness and captures the real numbers; the doc is written from those captured numbers, not estimated.
- If a contender can't run a case (install, OOM, budget), the cell says **skipped + why** — never silently omitted.
- No cherry-picking: every regime × convention × N cell that ran is reported.

## Non-goals

- Remote/cloud partial-read (chunk-pruning) comparison — eider's *other* advantage, separate benchmark.
- xarray/flox and raw GDAL/numpy contenders (excluded per decision).
- Zarr-format comparison (exactextract/rasterstats can't read Zarr natively; the common-COG choice sidesteps this — Zarr partial-read is the non-goal above).
- Multi-band / temporal stacking; only single-band 2D here.
