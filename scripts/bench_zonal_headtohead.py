"""Zonal-stats kernel head-to-head benchmark harness.

Task 1 implements the synthetic data generator: a tiled COG GeoTIFF plus
matching polygon sets (GeoParquet + GeoJSON) in a projected metric CRS with
square pixels, so that area weights are meaningful and consistent across the
contender tools (eider/DuckDB, exactextract, rasterstats).

Later tasks add the contender runners, correctness gate, and timing loop.
"""

from __future__ import annotations

import argparse
import contextlib
import json
import math
import platform
import signal
import statistics
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

import geopandas as gpd
import numpy as np
import rasterio
from rasterio.transform import Affine
from shapely.geometry import Polygon

# --- Grid geometry constants (projected metric CRS, square pixels) ---------
RASTER_CRS = "EPSG:3857"
PIXEL_SIZE_M = 30.0  # square pixel side length in CRS (metre) units
GRID_ORIGIN_X = 0.0  # x of the upper-left corner of the raster
GRID_ORIGIN_Y = 0.0  # y of the upper-left corner of the raster

# Default raster shapes (rows, cols) per regime.
FINE_SHAPE = (2000, 2000)
COARSE_SHAPE = (200, 200)

# Gradient field range used to build the smooth base surface.
GRADIENT_MAX = 50.0
GRADIENT_NOISE_STD = 2.0

# Diamond footprint radius for the "fine" regime, in pixels.
FINE_DIAMOND_RADIUS_PX = 5.0
# Sub-cell square side for the "coarse" regime, in pixels.
COARSE_SQUARE_SIDE_PX = 0.3


@dataclass(frozen=True)
class Regime:
    """Configuration for a synthetic data-generation regime."""

    name: str
    shape: tuple[int, int]


REGIMES: dict[str, Regime] = {
    "fine": Regime(name="fine", shape=FINE_SHAPE),
    "coarse": Regime(name="coarse", shape=COARSE_SHAPE),
}


def _build_field(shape: tuple[int, int], rng: np.random.Generator) -> np.ndarray:
    """Smooth diagonal gradient plus mild Gaussian noise, as float32."""
    height, width = shape
    base = np.add.outer(
        np.linspace(0.0, GRADIENT_MAX, height),
        np.linspace(0.0, GRADIENT_MAX, width),
    )
    field = base + rng.normal(0.0, GRADIENT_NOISE_STD, size=shape)
    return field.astype(np.float32)


def _transform() -> Affine:
    """Square-pixel north-up affine transform for the projected grid."""
    return Affine(PIXEL_SIZE_M, 0.0, GRID_ORIGIN_X, 0.0, -PIXEL_SIZE_M, GRID_ORIGIN_Y)


def _write_cog(path: Path, field: np.ndarray, transform: Affine) -> None:
    """Write a single-band float32 tiled GeoTIFF that rasterio can reopen as a COG."""
    height, width = field.shape
    profile = {
        "driver": "GTiff",
        "dtype": "float32",
        "count": 1,
        "height": height,
        "width": width,
        "crs": RASTER_CRS,
        "transform": transform,
        "tiled": True,
        "blockxsize": 256,
        "blockysize": 256,
        "compress": "deflate",
    }
    with rasterio.open(path, "w", **profile) as dst:
        dst.write(field, 1)


def _diamond(cx: float, cy: float, radius: float) -> Polygon:
    """Diamond (rotated square) footprint centred at (cx, cy)."""
    return Polygon(
        [
            (cx - radius, cy),
            (cx, cy + radius),
            (cx + radius, cy),
            (cx, cy - radius),
        ]
    )


def _square(cx: float, cy: float, half: float) -> Polygon:
    """Axis-aligned square footprint centred at (cx, cy) with the given half-side."""
    return Polygon(
        [
            (cx - half, cy - half),
            (cx - half, cy + half),
            (cx + half, cy + half),
            (cx + half, cy - half),
        ]
    )


def _random_centers(
    n: int,
    transform: Affine,
    shape: tuple[int, int],
    margin: float,
    rng: np.random.Generator,
) -> tuple[np.ndarray, np.ndarray]:
    """Random (x, y) centres well inside the raster extent, avoiding edges by `margin`."""
    height, width = shape
    x_min = transform.c
    x_max = transform.c + width * transform.a
    y_max = transform.f
    y_min = transform.f + height * transform.e  # transform.e is negative (north-up)

    cx = rng.uniform(x_min + margin, x_max - margin, size=n)
    cy = rng.uniform(y_min + margin, y_max - margin, size=n)
    return cx, cy


def generate_data(
    out_dir,
    regime: str,
    n_polys: int,
    seed: int = 42,
    shape: tuple[int, int] | None = None,
) -> dict:
    """Generate a synthetic tiled COG and a matching polygon set.

    Parameters
    ----------
    out_dir:
        Directory to write ``grid.tif``, ``polys.parquet`` and ``polys.geojson`` into.
    regime:
        ``"fine"`` (R2) -> large smooth grid + diamond footprints (~5 px radius);
        ``"coarse"`` (R1) -> small grid + sub-cell square footprints.
    n_polys:
        Number of polygons to generate.
    seed:
        Seed for the numpy RNG (determinism).
    shape:
        Optional ``(height, width)`` override for the raster. Defaults to the
        regime's standard shape.

    Returns
    -------
    dict with keys ``raster``, ``parquet``, ``geojson``, ``transform``, ``crs``,
    ``shape``.
    """
    if regime not in REGIMES:
        raise ValueError(f"unknown regime {regime!r}; expected one of {sorted(REGIMES)}")

    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    grid_shape = shape if shape is not None else REGIMES[regime].shape
    rng = np.random.default_rng(seed)

    field = _build_field(grid_shape, rng)
    transform = _transform()

    raster_path = out_dir / "grid.tif"
    _write_cog(raster_path, field, transform)

    if regime == "fine":
        radius = FINE_DIAMOND_RADIUS_PX * PIXEL_SIZE_M
        cx, cy = _random_centers(n_polys, transform, grid_shape, margin=radius * 2, rng=rng)
        geometries = [_diamond(x, y, radius) for x, y in zip(cx, cy)]
    else:  # coarse
        half = (COARSE_SQUARE_SIDE_PX * PIXEL_SIZE_M) / 2.0
        cx, cy = _random_centers(
            n_polys, transform, grid_shape, margin=PIXEL_SIZE_M * 2, rng=rng
        )
        geometries = [_square(x, y, half) for x, y in zip(cx, cy)]

    gdf = gpd.GeoDataFrame(
        {"poly_id": range(n_polys)},
        geometry=geometries,
        crs=RASTER_CRS,
    )

    parquet_path = out_dir / "polys.parquet"
    geojson_path = out_dir / "polys.geojson"
    gdf.to_parquet(parquet_path)
    gdf.to_file(geojson_path, driver="GeoJSON")

    return {
        "raster": str(raster_path),
        "parquet": str(parquet_path),
        "geojson": str(geojson_path),
        "transform": transform,
        "crs": RASTER_CRS,
        "shape": grid_shape,
    }


# ---------------------------------------------------------------------------
# Contender runners (Task 2)
#
# Each runner returns ``{poly_id: float}`` for a ``(raster, polys, convention,
# metric)`` request. Conventions: ``centroid``, ``all_touched``,
# ``area_weighted``. Metrics: ``max``, ``mean``, ``count``.
#
# Convention <-> tool mapping (kept honest, per the plan):
#   * centroid      -> rasterstats(all_touched=False) ; eider ST_Contains(centre)
#   * all_touched   -> rasterstats(all_touched=True), exactextract max,
#                      eider ST_Intersects(cell envelope)
#   * area_weighted -> exactextract mean (coverage-weighted),
#                      eider sum(v*area)/sum(area) over intersecting cells.
#                      rasterstats has no exact area weighting -> NotImplemented.
# ---------------------------------------------------------------------------

# read_geo column names for a local COG, discovered via
# ``DESCRIBE SELECT * FROM read_geo('grid.tif')`` (verified for EPSG:3857):
#   y (DOUBLE), x (DOUBLE), value (FLOAT)
# Coordinates are CELL CENTRES in the raster's own CRS (EPSG:3857 metres) --
# read_geo does NOT reproject to lon/lat. The polygons are generated in the
# same CRS, so the cell coords and polygon geometry are directly comparable.
READ_GEO_X = "x"
READ_GEO_Y = "y"
READ_GEO_VALUE = "value"

# Path to the loadable eider extension (relative to the repo root).
EIDER_EXTENSION_PATH = (
    Path(__file__).resolve().parents[1] / "target" / "debug" / "eider.duckdb_extension"
)

# Conventions and metrics as small enums-of-strings to avoid magic literals.
CONVENTION_CENTROID = "centroid"
CONVENTION_ALL_TOUCHED = "all_touched"
CONVENTION_AREA_WEIGHTED = "area_weighted"

METRIC_MAX = "max"
METRIC_MEAN = "mean"
METRIC_COUNT = "count"


def run_rasterstats(
    raster: str,
    geojson_path: str,
    convention: str,
    metric: str,
    poly_ids,
) -> dict:
    """rasterstats zonal_stats runner.

    ``all_touched=True`` for the all_touched convention, ``False`` (GDAL
    cell-centre rasterize) for centroid. rasterstats has no exact area
    weighting, so ``area_weighted`` raises ``NotImplementedError``.
    """
    from rasterstats import zonal_stats

    if convention == CONVENTION_AREA_WEIGHTED:
        raise NotImplementedError("rasterstats has no exact area weighting")

    all_touched = convention == CONVENTION_ALL_TOUCHED
    res = zonal_stats(
        geojson_path, raster, stats=[metric], all_touched=all_touched, nodata=None
    )
    return {
        pid: (r[metric] if r[metric] is not None else float("nan"))
        for pid, r in zip(poly_ids, res)
    }


def run_exactextract(raster: str, gdf, convention: str, metric: str) -> dict:
    """exactextract runner (coverage-weighted).

    exactextract weights every intersecting cell by its coverage fraction:
      * ``mean``  -> coverage-weighted mean  (used for area_weighted)
      * ``max``   -> max over all cells with coverage > 0 (used for all_touched)
      * ``count`` -> sum of coverage fractions

    Used for all_touched(max) and area_weighted(mean); NOT centroid (exactextract
    has no cell-centre-only mode). Output columns in 0.3.0 are named exactly by
    the op (``mean``/``max``/``count``), no ``band_`` prefix; it accepts a raster
    path plus a GeoDataFrame directly. Raster and gdf must share a CRS -- both
    are EPSG:3857 here.
    """
    from exactextract import exact_extract

    op = {METRIC_MAX: "max", METRIC_MEAN: "mean", METRIC_COUNT: "count"}[metric]
    df = exact_extract(raster, gdf, [op], output="pandas", include_cols=["poly_id"])
    value_col = next(c for c in df.columns if c != "poly_id")
    return {
        int(pid): float(val) for pid, val in zip(df["poly_id"], df[value_col])
    }


def eider_conn():
    """Open a DuckDB connection with the eider extension + spatial loaded."""
    import duckdb

    conn = duckdb.connect(config={"allow_unsigned_extensions": True})
    conn.execute(f"LOAD '{EIDER_EXTENSION_PATH}'")
    conn.execute("INSTALL spatial; LOAD spatial;")
    return conn


def _poly_bbox(conn, polys_parquet: str) -> tuple[float, float, float, float]:
    """Bounding box (xmin, ymin, xmax, ymax) of all polygons, in the polys' CRS."""
    row = conn.execute(
        f"""
        SELECT ST_XMin(ext), ST_YMin(ext), ST_XMax(ext), ST_YMax(ext)
        FROM (SELECT ST_Extent_Agg(geometry) AS ext
              FROM read_parquet('{polys_parquet}'))
        """
    ).fetchone()
    return tuple(float(v) for v in row)  # type: ignore[return-value]


# Number of cells of padding applied to the polygon bbox when slicing the field.
# read_geo's lat/lon bbox pushdown only fires for EPSG:4326 COGs (dim names
# lat/lon); for EPSG:3857 the dims are y/x with no matching pushdown param, so we
# prune with a SQL WHERE on x/y instead. A generous pad keeps every touched cell.
_BBOX_PAD_CELLS = 2.0
# Approximate cell size used only to pad the bbox slice; the exact dx/dy used in
# the geometry predicates is derived from the read itself.
_APPROX_CELL_M = PIXEL_SIZE_M


# Name of the reusable TEMP table holding the pruned, centre-corrected field
# with PRECOMPUTED geometry columns (pt = cell centre point, box = cell
# envelope). Materialising these as real columns (rather than inline
# ST_Point(...)/ST_MakeEnvelope(...) in the JOIN ON predicate) is what lets
# DuckDB-spatial's spatial-join optimiser build an RTree and engage a proper
# spatial join, instead of degrading to an O(n_polys x n_cells) nested loop
# (the latter both hangs AND OOMs at 100k+ polygons).
_FIELD_TABLE = "bench_field"

# Which (raster, parquet) case the ``bench_field`` TEMP table currently holds,
# keyed by ``id(conn)``. A DuckDB connection object rejects arbitrary
# attributes, so we track the materialised case here rather than on the conn.
# NOTE: id() can be recycled after a connection is closed and GC'd, so the cache
# is only ever trusted in combination with a live check that the table actually
# exists in THIS connection (see ``_field_table_exists``) -- the existence check
# is the source of truth, the dict is a fast-path hint.
_FIELD_CACHE: dict[int, tuple[str, str]] = {}


def _field_table_exists(conn) -> bool:
    """True iff the ``bench_field`` TEMP table exists in this connection."""
    row = conn.execute(
        "SELECT count(*) FROM duckdb_tables() WHERE table_name = ?",
        [_FIELD_TABLE],
    ).fetchone()
    return bool(row and row[0])


def _materialize_field(conn, raster: str, polys_parquet: str) -> None:
    """Build the reusable ``bench_field`` TEMP table for ``(raster, parquet)``.

    The table is built ONCE per ``(raster, polys_parquet)`` per connection and
    reused across every convention/metric/rep (a big win: the 4M-cell read +
    geometry construction is not repeated per query). It is created with
    ``CREATE OR REPLACE`` so repeated calls never accumulate multiple 4M-row
    tables in memory.

    Columns: ``x, y, v`` (cell CENTRE coords + value) plus the precomputed
    geometry columns ``pt`` (``ST_Point`` of the centre) and ``box``
    (``ST_MakeEnvelope`` of the cell). Bare-column predicates against ``pt`` /
    ``box`` let the spatial-join optimiser engage.

    The WHERE on x/y prunes the read to the polygons' neighbourhood (read_geo's
    lat/lon bbox pushdown does not apply to EPSG:3857 COGs, whose dims are y/x).

    Half-pixel correction: ``read_geo`` reports each cell's UPPER-LEFT CORNER,
    not its centre (verified empirically: a 30 m grid with origin x0=0 yields
    read_geo x = 0, 30, 60 ... whereas the true cell centres are 15, 45, 75 ...
    = x + dx/2; and read_geo y = -1470, -1440 ... whereas the true centres are
    -1485, -1455 ... = y - dy/2, since y is the top edge and the cell extends
    downward). All downstream predicates (the ST_Point centroid test, the
    cell-box envelope, the index-join snap) assume CENTRES, so we shift the raw
    corner coords to centres here. Without this shift the cell boxes are offset
    by half a pixel and MAX/centroid selection picks the wrong cell.

    The materialised case is recorded in ``_FIELD_CACHE`` keyed by ``id(conn)``
    (guarded by ``_field_table_exists``) so we skip the rebuild when the same
    case is timed across multiple conventions on the same connection.
    """
    key = (raster, polys_parquet)
    if _FIELD_CACHE.get(id(conn)) == key and _field_table_exists(conn):
        return  # already materialised for this exact case in THIS connection

    xmin, ymin, xmax, ymax = _poly_bbox(conn, polys_parquet)
    pad = _BBOX_PAD_CELLS * _APPROX_CELL_M
    conn.execute(
        f"""
        CREATE OR REPLACE TEMP TABLE {_FIELD_TABLE} AS
        WITH raw AS (
            SELECT {READ_GEO_X} AS x, {READ_GEO_Y} AS y, {READ_GEO_VALUE} AS v
            FROM read_geo('{raster}')
            WHERE {READ_GEO_X} BETWEEN {xmin - pad} AND {xmax + pad}
              AND {READ_GEO_Y} BETWEEN {ymin - pad} AND {ymax + pad}
              -- Drop nodata cells so count/area-weighted means match the
              -- coverage-based tools (exactextract/rasterstats ignore nodata).
              AND {READ_GEO_VALUE} IS NOT NULL
        ),
        step AS (
            SELECT (max(x) - min(x)) / nullif(count(DISTINCT x) - 1, 0) AS dx,
                   (max(y) - min(y)) / nullif(count(DISTINCT y) - 1, 0) AS dy
            FROM raw
        )
        -- Shift read_geo corner coords to true cell centres (see docstring),
        -- then materialise the centre point and the cell envelope as COLUMNS.
        SELECT
            raw.x + s.dx / 2 AS x,
            raw.y - s.dy / 2 AS y,
            raw.v AS v,
            ST_Point(raw.x + s.dx / 2, raw.y - s.dy / 2) AS pt,
            ST_MakeEnvelope(raw.x, raw.y - s.dy, raw.x + s.dx, raw.y) AS box
        FROM raw CROSS JOIN step s
        """
    )
    _FIELD_CACHE[id(conn)] = key


def _metric_agg(metric: str) -> str:
    return {
        METRIC_MAX: "max(z.v)",
        METRIC_MEAN: "avg(z.v)",
        METRIC_COUNT: "count(*)",
    }[metric]


def run_eider(
    conn, raster: str, polys_parquet: str, convention: str, metric: str
) -> dict:
    """eider/DuckDB + spatial runner: per-convention spatial-join zonal stats.

    Cell coords from read_geo are cell centres in the raster CRS (EPSG:3857
    metres); polygons are read from GeoParquet in the same CRS, so the join is
    CRS-consistent without any reprojection. dx/dy are derived from the read.
    """
    _materialize_field(conn, raster, polys_parquet)

    if convention == CONVENTION_CENTROID:
        # Bare ST_Contains(polygon, precomputed point column) -> spatial join.
        sql = f"""
            SELECT v.poly_id, {_metric_agg(metric)} AS metric
            FROM read_parquet('{polys_parquet}') v
            JOIN {_FIELD_TABLE} z
              ON ST_Contains(v.geometry, z.pt)
            GROUP BY v.poly_id
        """
    elif convention == CONVENTION_ALL_TOUCHED:
        # Bare ST_Intersects(polygon, precomputed box column) -> spatial join.
        sql = f"""
            SELECT v.poly_id, {_metric_agg(metric)} AS metric
            FROM read_parquet('{polys_parquet}') v
            JOIN {_FIELD_TABLE} z
              ON ST_Intersects(v.geometry, z.box)
            GROUP BY v.poly_id
        """
    elif convention == CONVENTION_AREA_WEIGHTED:
        if metric != METRIC_MEAN:
            raise NotImplementedError(
                "area_weighted is only defined for the mean metric"
            )
        # Bare ST_Intersects join to prune to touched cells, then weight each by
        # the cell-box/polygon ST_Intersection area (the box column is reused).
        sql = f"""
            SELECT v.poly_id,
                   sum(z.v * ST_Area(ST_Intersection(v.geometry, z.box)))
                   / nullif(sum(ST_Area(ST_Intersection(v.geometry, z.box))), 0)
                     AS metric
            FROM read_parquet('{polys_parquet}') v
            JOIN {_FIELD_TABLE} z
              ON ST_Intersects(v.geometry, z.box)
            GROUP BY v.poly_id
        """
    else:
        raise ValueError(f"unknown convention {convention!r}")

    rows = conn.execute(sql).fetchall()
    return {int(pid): float(val) for pid, val in rows if val is not None}


def run_eider_indexjoin(
    conn, raster: str, polys_parquet: str, convention: str, metric: str
) -> dict:
    """eider point-model runner via an arithmetic cell-index equi-join.

    For Regime 1 (sub-cell polygons) we model each polygon by its centroid and
    snap it to the nearest cell index using the read's origin/step, then equi-join
    on integer (ix, iy). This avoids the geometry predicate entirely. Valid only
    for the sub-cell / point-model case; labelled as such in the results.
    """
    _materialize_field(conn, raster, polys_parquet)

    # Derive step/origin from the centre-corrected, materialised field table.
    sql = f"""
        WITH step AS (
            SELECT (max(x) - min(x)) / nullif(count(DISTINCT x) - 1, 0) AS dx,
                   (max(y) - min(y)) / nullif(count(DISTINCT y) - 1, 0) AS dy
            FROM {_FIELD_TABLE}
        ),
        origin AS (SELECT min(x) AS x0, min(y) AS y0 FROM {_FIELD_TABLE}),
        cells AS (
            SELECT round((z.x - o.x0) / s.dx) AS ix,
                   round((z.y - o.y0) / s.dy) AS iy,
                   z.v AS v
            FROM {_FIELD_TABLE} z CROSS JOIN step s CROSS JOIN origin o
        ),
        centroids AS (
            SELECT v.poly_id,
                   round((ST_X(ST_Centroid(v.geometry)) - o.x0) / s.dx) AS ix,
                   round((ST_Y(ST_Centroid(v.geometry)) - o.y0) / s.dy) AS iy
            FROM read_parquet('{polys_parquet}') v
            CROSS JOIN step s CROSS JOIN origin o
        )
        SELECT c.poly_id, {_metric_agg_index(metric)} AS metric
        FROM centroids c
        JOIN cells z ON c.ix = z.ix AND c.iy = z.iy
        GROUP BY c.poly_id
    """
    rows = conn.execute(sql).fetchall()
    return {int(pid): float(val) for pid, val in rows if val is not None}


def _metric_agg_index(metric: str) -> str:
    return {
        METRIC_MAX: "max(z.v)",
        METRIC_MEAN: "avg(z.v)",
        METRIC_COUNT: "count(*)",
    }[metric]


# ---------------------------------------------------------------------------
# Correctness gate (Task 3)
# ---------------------------------------------------------------------------
def assert_agree(
    a: dict,
    b: dict,
    name_a: str,
    name_b: str,
    abs_tol: float,
) -> dict:
    """Compare two ``{poly_id: float}`` results on their overlapping poly_ids.

    Comparison is NaN-aware:
      * both values NaN  -> agreement (both tools declined the same polygon);
      * exactly one NaN  -> mismatch (the tools disagree about coverage);
      * both finite       -> mismatch iff ``abs(a - b) > abs_tol``.

    Parameters
    ----------
    a, b:
        Result dicts keyed by ``poly_id``. Only ids present in BOTH are compared.
    name_a, name_b:
        Labels used in the no-overlap assertion message.
    abs_tol:
        Absolute tolerance for finite comparisons.

    Returns
    -------
    dict with keys ``agree`` (bool), ``max_abs_diff`` (float over comparable
    finite pairs), ``n_compared`` (int), ``n_mismatch`` (int) and ``examples``
    (up to five ``(poly_id, a_value, b_value)`` mismatch tuples).
    """
    ids = set(a) & set(b)
    assert ids, f"{name_a}/{name_b}: no overlapping poly_ids"

    mismatches: list[tuple] = []
    max_abs_diff = 0.0
    for i in sorted(ids):
        av, bv = a[i], b[i]
        a_nan, b_nan = math.isnan(av), math.isnan(bv)
        if a_nan and b_nan:
            continue  # both declined -> agreement
        if a_nan or b_nan:
            mismatches.append((i, av, bv))  # exactly one NaN -> disagreement
            continue
        diff = abs(av - bv)
        if diff > max_abs_diff:
            max_abs_diff = diff
        if diff > abs_tol:
            mismatches.append((i, av, bv))

    return {
        "agree": not mismatches,
        "max_abs_diff": max_abs_diff,
        "n_compared": len(ids),
        "n_mismatch": len(mismatches),
        "examples": mismatches[:5],
    }


# ---------------------------------------------------------------------------
# Timing harness (Task 4)
# ---------------------------------------------------------------------------

# Contender labels (stable, machine-readable column keys in the JSON/table).
CONTENDER_EIDER = "eider"
CONTENDER_EIDER_INDEXJOIN = "eider_indexjoin"
CONTENDER_EXACTEXTRACT = "exactextract"
CONTENDER_RASTERSTATS = "rasterstats"

# Marker used in the stdout table / JSON when a cell is not applicable.
NA_MARKER = "n/a"

# Status string for a cell deliberately NOT timed because the contender is
# known to be impractically slow at that scale (rasterstats above the cap).
STATUS_IMPRACTICAL = "impractical"
# Note recorded on impractical rasterstats cells (carried into the JSON).
RASTERSTATS_IMPRACTICAL_NOTE = "~1000x slower (measured 1030s at 100k); not timed"

# Default cap above which rasterstats is recorded as impractical, not timed.
DEFAULT_RASTERSTATS_MAX_N = 20_000


class _BudgetTimeout(TimeoutError):
    """Raised by the SIGALRM handler when a single call exceeds the budget."""


@contextlib.contextmanager
def _alarm_guard(seconds: float | None):
    """Hard per-call timeout via ``SIGALRM``, as a safety net for slow paths.

    A pure-Python contender (rasterstats) that blows past ``seconds`` is
    interrupted with a ``_BudgetTimeout``; fast C calls (exactextract) that
    never return to the interpreter won't trip it. The alarm is always reset in
    the ``finally`` so it can never leak into a later call. ``signal.alarm``
    only works on the main thread of a real OS process (true here); if it is
    unavailable or ``seconds`` is falsy, the guard is a no-op.
    """
    if not seconds or not hasattr(signal, "SIGALRM"):
        yield
        return

    def _handler(signum, frame):  # noqa: ARG001 - signal handler signature
        raise _BudgetTimeout(f"call exceeded hard budget of {seconds:.0f}s")

    previous = signal.signal(signal.SIGALRM, _handler)
    # signal.alarm takes whole seconds; round up so we never fire early.
    signal.alarm(max(1, math.ceil(seconds)))
    try:
        yield
    finally:
        signal.alarm(0)
        signal.signal(signal.SIGALRM, previous)


@dataclass(frozen=True)
class TimingResult:
    """Outcome of timing a single contender call (median over reps)."""

    seconds: float | None  # None == not timed (skipped / NA)
    status: str  # "ok" | "skipped (budget)" | "error"
    reps: int = 0
    detail: str = ""


def time_call(
    fn: Callable[[], object],
    reps: int = 3,
    warmup: int = 1,
    budget_seconds: float | None = None,
) -> TimingResult:
    """Time ``fn`` and return the median wall-clock seconds over ``reps``.

    Warmup runs happen OUTSIDE the timed region, so one-time costs (a DuckDB
    ``LOAD extension``, the first GDAL open of a file, lazy imports) are excluded
    from the reported median. Each contender's read of the warm local COG is
    therefore measured against an already-warm process and OS page cache, which
    is the regime we want to compare.

    Budget guard: if any single timed rep exceeds ``budget_seconds`` the call is
    abandoned immediately (no further reps) and reported as ``skipped (budget)``
    so a pathologically slow cell can never hang the whole matrix.
    """
    try:
        for _ in range(max(0, warmup)):
            with _alarm_guard(budget_seconds):
                fn()
    except _BudgetTimeout as exc:
        return TimingResult(None, "skipped (budget)", 0, str(exc))
    except Exception as exc:  # noqa: BLE001 - record, don't crash the matrix
        return TimingResult(None, "error", 0, f"{type(exc).__name__}: {exc}")

    samples: list[float] = []
    for _ in range(max(1, reps)):
        start = time.perf_counter()
        try:
            with _alarm_guard(budget_seconds):
                fn()
        except _BudgetTimeout as exc:
            return TimingResult(None, "skipped (budget)", len(samples), str(exc))
        except Exception as exc:  # noqa: BLE001
            return TimingResult(None, "error", len(samples), f"{type(exc).__name__}: {exc}")
        elapsed = time.perf_counter() - start
        samples.append(elapsed)
        if budget_seconds is not None and elapsed > budget_seconds:
            return TimingResult(
                None,
                "skipped (budget)",
                len(samples),
                f"rep took {elapsed:.2f}s > budget {budget_seconds:.0f}s",
            )

    return TimingResult(statistics.median(samples), "ok", len(samples))


# --- Valid (convention, metric) -> applicable contenders -------------------
#
# Honest pairings (see Task 3 docstring for the convention<->tool mapping):
#   * centroid/max      : eider, rasterstats         (exactextract has no centroid)
#   * all_touched/max   : eider, rasterstats, exactextract
#   * all_touched/mean  : eider, rasterstats, exactextract (optional, all average
#                          the same touched-cell set / coverage)
#   * area_weighted/mean: eider, exactextract        (rasterstats: no area weight)
#
# ``run_eider_indexjoin`` is added ONLY for the coarse regime (R1, sub-cell
# point model) inside ``run_matrix`` -- it is not a general contender.
VALID_PAIRINGS: list[tuple[str, str, tuple[str, ...]]] = [
    (CONVENTION_CENTROID, METRIC_MAX, (CONTENDER_EIDER, CONTENDER_RASTERSTATS)),
    (
        CONVENTION_ALL_TOUCHED,
        METRIC_MAX,
        (CONTENDER_EIDER, CONTENDER_RASTERSTATS, CONTENDER_EXACTEXTRACT),
    ),
    (
        CONVENTION_ALL_TOUCHED,
        METRIC_MEAN,
        (CONTENDER_EIDER, CONTENDER_RASTERSTATS, CONTENDER_EXACTEXTRACT),
    ),
    (
        CONVENTION_AREA_WEIGHTED,
        METRIC_MEAN,
        (CONTENDER_EIDER, CONTENDER_EXACTEXTRACT),
    ),
]

# Conventions to put through the correctness gate (once per regime, at the
# reference n) before timing. Each entry is (convention, metric, contender_a,
# contender_b, tol) where contender_a is always eider (the system under test).
GATE_PAIRINGS: list[tuple[str, str, str, str, float]] = [
    (CONVENTION_CENTROID, METRIC_MAX, CONTENDER_EIDER, CONTENDER_RASTERSTATS, 1e-3),
    (CONVENTION_ALL_TOUCHED, METRIC_MAX, CONTENDER_EIDER, CONTENDER_RASTERSTATS, 1e-3),
    (CONVENTION_ALL_TOUCHED, METRIC_MAX, CONTENDER_EIDER, CONTENDER_EXACTEXTRACT, 1e-3),
    (CONVENTION_ALL_TOUCHED, METRIC_MEAN, CONTENDER_EIDER, CONTENDER_RASTERSTATS, 1e-2),
    (
        CONVENTION_AREA_WEIGHTED,
        METRIC_MEAN,
        CONTENDER_EIDER,
        CONTENDER_EXACTEXTRACT,
        1e-1,
    ),
]


@dataclass
class CaseData:
    """One generated (regime, n) case, reused across conventions/contenders/reps.

    Generating raster + polygons is expensive, so we do it once per (regime, n)
    and keep the polygon GeoDataFrame in memory (exactextract wants a gdf;
    rasterstats reads the geojson path; eider reads the parquet path).
    """

    regime: str
    n_polys: int
    raster: str
    parquet: str
    geojson: str
    gdf: gpd.GeoDataFrame
    poly_ids: list[int]


def _make_contender_call(
    contender: str,
    conn,
    case: CaseData,
    convention: str,
    metric: str,
) -> Callable[[], object]:
    """Build a zero-arg callable that runs ``contender`` fresh (no cached result).

    The data (raster/gdf/paths) is captured by reference and reused across reps;
    only the *compute* is re-executed each call, against the warm OS cache.
    """
    if contender == CONTENDER_EIDER:
        return lambda: run_eider(conn, case.raster, case.parquet, convention, metric)
    if contender == CONTENDER_EIDER_INDEXJOIN:
        return lambda: run_eider_indexjoin(
            conn, case.raster, case.parquet, convention, metric
        )
    if contender == CONTENDER_EXACTEXTRACT:
        return lambda: run_exactextract(case.raster, case.gdf, convention, metric)
    if contender == CONTENDER_RASTERSTATS:
        return lambda: run_rasterstats(
            case.raster, case.geojson, convention, metric, case.poly_ids
        )
    raise ValueError(f"unknown contender {contender!r}")


def _run_one_contender(contender: str, conn, case: CaseData, convention, metric):
    """Run a contender once and return its ``{poly_id: float}`` result."""
    return _make_contender_call(contender, conn, case, convention, metric)()


def _run_gate(
    conn, case: CaseData, findings: list[dict]
) -> dict[tuple[str, str], dict]:
    """Run the correctness gate for ``case`` once; record any disagreement.

    Returns ``{(convention, metric): agreement_report}`` keyed by the eider-side
    pairing, so the timing table can annotate each row with the observed
    ``max_abs_diff``. A disagreement beyond tolerance is recorded as a FINDING
    (it does NOT crash the run -- credibility issues must be visible, not fatal).
    """
    agreements: dict[tuple[str, str], dict] = {}
    for convention, metric, ca, cb, tol in GATE_PAIRINGS:
        try:
            res_a = _run_one_contender(ca, conn, case, convention, metric)
            res_b = _run_one_contender(cb, conn, case, convention, metric)
            rep = assert_agree(res_a, res_b, ca, cb, tol)
        except Exception as exc:  # noqa: BLE001
            findings.append(
                {
                    "regime": case.regime,
                    "convention": convention,
                    "metric": metric,
                    "pair": f"{ca} vs {cb}",
                    "kind": "gate_error",
                    "detail": f"{type(exc).__name__}: {exc}",
                }
            )
            continue

        key = (convention, metric)
        prev = agreements.get(key)
        # Keep the worst (largest) max_abs_diff seen for the row label.
        if prev is None or rep["max_abs_diff"] > prev["max_abs_diff"]:
            agreements[key] = rep

        if not rep["agree"]:
            findings.append(
                {
                    "regime": case.regime,
                    "convention": convention,
                    "metric": metric,
                    "pair": f"{ca} vs {cb}",
                    "kind": "disagreement",
                    "tol": tol,
                    "max_abs_diff": rep["max_abs_diff"],
                    "n_mismatch": rep["n_mismatch"],
                    "examples": rep["examples"],
                }
            )
    return agreements


def _versions() -> dict[str, str]:
    """Collect contender library versions for the environment block."""
    out: dict[str, str] = {}
    for mod_name, key in (
        ("duckdb", "duckdb"),
        ("exactextract", "exactextract"),
        ("rasterstats", "rasterstats"),
        ("rasterio", "rasterio"),
        ("geopandas", "geopandas"),
        ("numpy", "numpy"),
    ):
        try:
            mod = __import__(mod_name)
            out[key] = getattr(mod, "__version__", "unknown")
        except Exception:  # noqa: BLE001
            out[key] = "unavailable"
    return out


def _env_block() -> dict:
    """Platform/machine/version environment block for the JSON + stdout."""
    return {
        "platform": platform.platform(),
        "machine": platform.machine(),
        "processor": platform.processor() or "unknown",
        "python": platform.python_version(),
        "versions": _versions(),
    }


@dataclass
class MatrixConfig:
    """Configuration for one ``run_matrix`` invocation."""

    out_dir: Path
    regime: str  # "fine" | "coarse" | "both"
    counts: list[int]
    reps: int
    budget_seconds: float
    shape_override: dict[str, tuple[int, int]] = field(default_factory=dict)
    # n at which the correctness gate runs (the smallest count, by default).
    gate_n: int | None = None
    # rasterstats is ~1000x slower than the C/SQL contenders; above this cap we
    # record it as "impractical" instead of running it (would take hours).
    rasterstats_max_n: int = DEFAULT_RASTERSTATS_MAX_N


def _regimes_for(regime: str) -> list[str]:
    if regime == "both":
        return ["fine", "coarse"]
    return [regime]


def run_matrix(cfg: MatrixConfig, conn) -> dict:
    """Drive the full benchmark matrix and return a results document.

    For each regime we generate each (regime, n) case ONCE (raster + polygons),
    reuse it across every convention/metric/contender/rep, run the correctness
    gate once at the reference n, then time every valid contender call. The
    eider connection is created by the caller and reused throughout.
    """
    findings: list[dict] = []
    rows: list[dict] = []

    for regime in _regimes_for(cfg.regime):
        shape = cfg.shape_override.get(regime)
        gate_n = cfg.gate_n if cfg.gate_n is not None else min(cfg.counts)

        # Generate every (regime, n) case once; cache for gate + timing reuse.
        cases: dict[int, CaseData] = {}
        for n in cfg.counts:
            data = generate_data(cfg.out_dir / f"{regime}_n{n}", regime, n, shape=shape)
            gdf = gpd.read_parquet(data["parquet"])
            cases[n] = CaseData(
                regime=regime,
                n_polys=n,
                raster=data["raster"],
                parquet=data["parquet"],
                geojson=data["geojson"],
                gdf=gdf,
                poly_ids=list(gdf["poly_id"]),
            )

        # Correctness gate: once per regime at the reference n, BEFORE timing.
        gate_case = cases.get(gate_n) or cases[min(cases)]
        agreements = _run_gate(conn, gate_case, findings)

        # Index-join contender is valid only for the coarse (sub-cell) regime.
        include_indexjoin = regime == "coarse"

        for n in cfg.counts:
            case = cases[n]
            for convention, metric, contenders in VALID_PAIRINGS:
                contenders = tuple(contenders)
                if include_indexjoin and convention == CONVENTION_CENTROID:
                    # The point-model index join is a centroid-style selection.
                    contenders = contenders + (CONTENDER_EIDER_INDEXJOIN,)

                agree = agreements.get((convention, metric))
                row = {
                    "regime": regime,
                    "convention": convention,
                    "metric": metric,
                    "n": n,
                    "agree": (
                        None
                        if agree is None
                        else {
                            "ok": agree["agree"],
                            "max_abs_diff": agree["max_abs_diff"],
                        }
                    ),
                    "timings": {},
                }
                for contender in (
                    CONTENDER_EIDER,
                    CONTENDER_EIDER_INDEXJOIN,
                    CONTENDER_EXACTEXTRACT,
                    CONTENDER_RASTERSTATS,
                ):
                    if contender not in contenders:
                        row["timings"][contender] = {
                            "seconds": None,
                            "status": NA_MARKER,
                        }
                        continue
                    # Honest cap: do not run rasterstats above the cap; record
                    # it as impractical (it is ~1000x slower than the others).
                    if (
                        contender == CONTENDER_RASTERSTATS
                        and n > cfg.rasterstats_max_n
                    ):
                        row["timings"][contender] = {
                            "seconds": None,
                            "status": STATUS_IMPRACTICAL,
                            "reps": 0,
                            "detail": RASTERSTATS_IMPRACTICAL_NOTE,
                        }
                        continue
                    call = _make_contender_call(
                        contender, conn, case, convention, metric
                    )
                    res = time_call(
                        call,
                        reps=cfg.reps,
                        warmup=1,
                        budget_seconds=cfg.budget_seconds,
                    )
                    row["timings"][contender] = {
                        "seconds": res.seconds,
                        "status": res.status,
                        "reps": res.reps,
                        "detail": res.detail,
                    }
                rows.append(row)

    return {
        "environment": _env_block(),
        "config": {
            "regime": cfg.regime,
            "counts": cfg.counts,
            "reps": cfg.reps,
            "budget_seconds": cfg.budget_seconds,
            "gate_n": cfg.gate_n if cfg.gate_n is not None else min(cfg.counts),
        },
        "rows": rows,
        "findings": findings,
    }


# --- Human-readable rendering ----------------------------------------------

_TABLE_CONTENDERS = (
    CONTENDER_EIDER,
    CONTENDER_EIDER_INDEXJOIN,
    CONTENDER_EXACTEXTRACT,
    CONTENDER_RASTERSTATS,
)
_TABLE_HEADERS = ("eider", "eider_ij", "exactextr", "rasterst")


def _fmt_cell(timing: dict) -> str:
    status = timing.get("status")
    if status == "ok" and timing.get("seconds") is not None:
        return f"{timing['seconds']:.4f}"
    if status == NA_MARKER:
        return NA_MARKER
    if status == "skipped (budget)":
        return "SKIP(bdg)"
    if status == STATUS_IMPRACTICAL:
        return "IMPRACT"
    if status == "error":
        return "ERROR"
    return str(status)


def _winner(timings: dict) -> str:
    """Fastest contender with an ``ok`` timing, or a dash if none timed."""
    best: tuple[float, str] | None = None
    for contender, t in timings.items():
        if t.get("status") == "ok" and t.get("seconds") is not None:
            if best is None or t["seconds"] < best[0]:
                best = (t["seconds"], contender)
    return best[1] if best else "-"


def render_table(doc: dict) -> str:
    """Build the human-readable stdout report (env block + matrix table)."""
    lines: list[str] = []
    env = doc["environment"]
    cfg = doc["config"]

    lines.append("=" * 92)
    lines.append("Zonal-stats head-to-head benchmark (warm local COG)")
    lines.append("=" * 92)
    lines.append("Environment:")
    lines.append(f"  platform : {env['platform']}")
    lines.append(f"  machine  : {env['machine']}  processor: {env['processor']}")
    lines.append(f"  python   : {env['python']}")
    lines.append("  versions :")
    for k, v in env["versions"].items():
        lines.append(f"      {k:<13} {v}")
    lines.append(
        f"Config: regime={cfg['regime']} counts={cfg['counts']} reps={cfg['reps']} "
        f"budget={cfg['budget_seconds']}s gate_n={cfg['gate_n']}"
    )
    lines.append("")

    # Table header.
    head = (
        f"{'regime':<7} {'convention':<13} {'metric':<5} {'n':>9} "
        f"{'eider':>10} {'eider_ij':>10} {'exactextr':>10} {'rasterst':>10} "
        f"{'winner':>11} {'agree(maxΔ)':>14}"
    )
    lines.append(head)
    lines.append("-" * len(head))

    for row in doc["rows"]:
        cells = [_fmt_cell(row["timings"][c]) for c in _TABLE_CONTENDERS]
        winner = _winner(row["timings"])
        agree = row["agree"]
        if agree is None:
            agree_str = NA_MARKER
        else:
            mark = "ok" if agree["ok"] else "FAIL"
            agree_str = f"{mark} {agree['max_abs_diff']:.2e}"
        lines.append(
            f"{row['regime']:<7} {row['convention']:<13} {row['metric']:<5} "
            f"{row['n']:>9} "
            f"{cells[0]:>10} {cells[1]:>10} {cells[2]:>10} {cells[3]:>10} "
            f"{winner:>11} {agree_str:>14}"
        )

    lines.append("")
    findings = doc["findings"]
    if findings:
        lines.append(f"FINDINGS ({len(findings)}):")
        for f in findings:
            if f["kind"] == "disagreement":
                lines.append(
                    f"  [DISAGREE] {f['regime']}/{f['convention']}/{f['metric']} "
                    f"{f['pair']}: max_abs_diff={f['max_abs_diff']:.3e} "
                    f"> tol={f['tol']:.1e} ({f['n_mismatch']} mismatches)"
                )
            else:
                lines.append(
                    f"  [GATE-ERR] {f['regime']}/{f['convention']}/{f['metric']} "
                    f"{f['pair']}: {f['detail']}"
                )
    else:
        lines.append("FINDINGS: none — correctness gate passed for every pairing.")

    lines.append(
        "\nLegend: seconds = median wall-clock over reps (warmup excluded). "
        f"{NA_MARKER}=pairing not applicable, SKIP(bdg)=exceeded budget, "
        "IMPRACT=not timed (rasterstats ~1000x slower above the cap), "
        "ERROR=runner raised. eider_ij = point-model index join (coarse/R1 only)."
    )
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

# Tiny self-test sizing for --quick (fast end-to-end smoke run).
QUICK_COUNTS = [200]
QUICK_REPS = 1
QUICK_SHAPE = (120, 120)
DEFAULT_COUNTS = [10_000, 100_000, 1_000_000]
DEFAULT_BUDGET_SECONDS = 120.0


def _parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    p = argparse.ArgumentParser(
        prog="bench_zonal_headtohead",
        description=(
            "Head-to-head zonal-stats benchmark: eider/DuckDB vs exactextract "
            "vs rasterstats on a warm local COG, with a pre-timing correctness gate."
        ),
    )
    p.add_argument(
        "--out-dir",
        default=None,
        help="Directory for generated rasters/polygons (default: a temp dir).",
    )
    p.add_argument(
        "--counts",
        type=int,
        nargs="+",
        default=DEFAULT_COUNTS,
        help="Polygon counts to benchmark (e.g. 10000 100000 1000000).",
    )
    p.add_argument("--reps", type=int, default=3, help="Timed reps per cell (median).")
    p.add_argument(
        "--budget-seconds",
        type=float,
        default=DEFAULT_BUDGET_SECONDS,
        help="Per-call budget; a rep exceeding this marks the cell skipped.",
    )
    p.add_argument(
        "--json", dest="json_path", default=None, help="Write machine-readable results here."
    )
    p.add_argument(
        "--regime", choices=["fine", "coarse", "both"], default="both", help="Regime(s)."
    )
    p.add_argument(
        "--rasterstats-max-n",
        type=int,
        default=DEFAULT_RASTERSTATS_MAX_N,
        help=(
            "Do not run rasterstats for n above this cap; record it as "
            f"'{STATUS_IMPRACTICAL}' instead (default: {DEFAULT_RASTERSTATS_MAX_N})."
        ),
    )
    p.add_argument(
        "--quick",
        action="store_true",
        help="Tiny end-to-end self-test (small grid, n=200, 1 rep).",
    )
    return p.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    import os
    import tempfile

    args = _parse_args(argv)

    if args.quick:
        counts = QUICK_COUNTS
        reps = QUICK_REPS
        shape_override = {"fine": QUICK_SHAPE, "coarse": QUICK_SHAPE}
    else:
        counts = args.counts
        reps = args.reps
        shape_override = {}

    if args.out_dir is not None:
        out_dir = Path(args.out_dir)
        out_dir.mkdir(parents=True, exist_ok=True)
        tmp_ctx = None
    else:
        tmp_ctx = tempfile.TemporaryDirectory(prefix="bench_zonal_")
        out_dir = Path(tmp_ctx.name)

    # eider reads must be permitted under the data dir.
    os.environ["GEOZARR_ALLOW_PATH"] = str(out_dir)

    cfg = MatrixConfig(
        out_dir=out_dir,
        regime=args.regime,
        counts=counts,
        reps=reps,
        budget_seconds=args.budget_seconds,
        shape_override=shape_override,
        rasterstats_max_n=args.rasterstats_max_n,
    )

    conn = eider_conn()
    try:
        doc = run_matrix(cfg, conn)
    finally:
        conn.close()
        if tmp_ctx is not None:
            tmp_ctx.cleanup()

    print(render_table(doc))

    if args.json_path:
        with open(args.json_path, "w", encoding="utf-8") as fh:
            json.dump(doc, fh, indent=2, default=str)
        print(f"\nWrote machine-readable results to {args.json_path}")

    # Non-zero exit if the gate exposed a real disagreement (not just NA cells).
    disagreements = [f for f in doc["findings"] if f["kind"] == "disagreement"]
    return 1 if disagreements else 0


if __name__ == "__main__":
    sys.exit(main())
