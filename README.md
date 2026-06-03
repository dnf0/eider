# Zarrduck

A high-performance, cloud-native [DuckDB](https://duckdb.org/) extension for reading and writing N-dimensional [Zarr](https://zarr.readthedocs.io/) and GeoZarr arrays directly as flat relational tables.

![Zarrduck End-to-End Demo](docs/src/demo.gif)

[![Semantic Release](https://github.com/dnf0/zarrduck/actions/workflows/release.yaml/badge.svg)](https://github.com/dnf0/zarrduck/actions/workflows/release.yaml)

📖 **[Read the Full Documentation here!](https://dnf0.github.io/zarrduck/)**

## Performance

All benchmarks use the [NCEP CDAS reanalysis surface air temperature dataset](https://downloads.psl.noaa.gov/Datasets/ncep.reanalysis.derived/surface/air.mon.mean.nc) converted to Zarr (938×73×144, chunk=12×73×144, 79 chunks). Timed query: extract all grid cells within the California bounding box (−125°–−115°, 30°–45°), returning 32,830 rows. In-process DuckDB Python API; median of 20 runs.

### Head-to-head: zarrduck vs the Python ecosystem

| Tool | CA bbox | Full scan | Spatial mean | Top-10 |
|---|---|---|---|---|
| xarray (1 thread) | 34.6 ms | 34.7 ms | 33.5 ms | 34.4 ms |
| zarr-python | 13.5 ms | 13.7 ms | 13.3 ms | — |
| zarr-python + zarrs pipeline¹ | 4.1 ms | **3.9 ms** | 4.1 ms | 4.4 ms |
| **zarrduck (1 thread)** | **3.0 ms** | 7.0 ms | **3.7 ms** | **3.5 ms** |

> ¹ [zarrs](https://github.com/LDeakin/zarrs) is the same Rust codec library zarrduck uses internally, exposed via Python bindings (`zarr.config.set({"codec_pipeline.path": "zarrs.ZarrsCodecPipeline"})`). It is the current fastest Python-accessible Zarr decoder.

zarrduck matches or beats the fastest available Python Zarr library on all filtered queries. The full-scan case is the one exception — zarrduck pays extra cost generating coordinate columns for 9.8 M rows; zarrs Python bindings return a raw NumPy array with no coordinate overhead. For the filtered queries that matter in practice, zarrduck wins because its chunk-level spatial pruning skips non-intersecting chunks before reading them, while zarrs Python reads every chunk in full.

### Scaling: threads × throughput

Each chunk is dispatched to a separate DuckDB worker, so throughput scales near-linearly with CPU cores. CA bbox extraction (32,830 rows, 79 chunks):

| Build | Threads | Time | Speedup |
|---|---|---|---|
| debug | 1 | 75 ms | 1× (baseline) |
| release (original) | 1 | 37 ms | 2× |
| release (SIMD optimized) | 1 | **3.0 ms** | **25×** |

### Remote data: CMIP6 on Google Cloud Storage

[CMIP6 CESM2 historical surface temperature](https://storage.googleapis.com/cmip6/CMIP6/CMIP/NCAR/CESM2/historical/r1i1p1f1/Amon/tas/gn/v20190308/) (~1° resolution, 1,980 monthly steps, 1850–2014). Query: California bbox.

| Tool | Download | Time | Note |
|---|---|---|---|
| zarrduck | 506 MB | ~48 s | Whole-globe spatial chunk (192×288) |
| xarray + shapely | ~42 MB | ~8 s | Server-side lat slice before download |
| zarrduck (spatially chunked²) | 38 MB | ~2.2 s | Only intersecting chunks fetched |
| xarray (spatially chunked²) | ~2 MB | ~0.9 s | Server-side bbox slice |

> ² Zarr re-chunked locally to 73×144 (2.5° grid). Chunk granularity is the dominant factor — zarrduck's spatial pruning skips non-intersecting chunks entirely, but cannot partially read within a chunk. For datasets with single global spatial chunks (common in raw CMIP6), xarray's server-side slicing downloads less data; zarrduck's advantage grows when chunk size aligns with the query region.

### Post-extraction analytics: zarrduck's sweet spot

Once data is extracted into DuckDB (32,830 rows, California subset), subsequent SQL queries are near-instant and compose freely with other DuckDB tables — no IPC or Python overhead:

| Query | zarrduck | pandas (equiv.) |
|---|---|---|
| Spatial mean (GROUP BY lat, lon) | 0.7 ms | ~0.5 ms |
| Top-N hottest months | 0.7 ms | ~0.4 ms |
| Decadal trend | 0.7 ms | ~3.6 ms |
| Monthly anomaly | 0.8 ms | ~0.3 ms |

Overall scan rate: **~174 M rows/s** on extracted data. The extraction cost is paid once; every subsequent query is free in DuckDB's vectorized engine.

## Why Zarrduck?

Geospatial and climate data are frequently stored in Zarr format because it enables efficient, chunked, and compressed storage of multi-dimensional arrays (like Time × Latitude × Longitude). However, querying this data traditionally required loading it into Python (via `xarray` or `zarr-python`) before performing analytics, introducing massive IPC (Inter-Process Communication) and memory overhead.

This project bridges the gap with two tools:
1. **The DuckDB Extension:** Natively streams remote Zarr chunks directly into DuckDB's vectorized execution engine for lightning-fast reads.
2. **`geozarr-cli`:** A companion CLI tool that executes SQL against your data and asynchronously uploads the results back to cloud storage as an N-dimensional Zarr array.

### Key Features
- **Zero-Copy Streaming**: Chunks are loaded, decompressed, and decoded natively inside DuckDB's engine.
- **Lock-Free Parallel Scanning**: Achieves maximum S3 throughput by utilizing DuckDB's multi-threaded worker pool to fetch and decode multiple chunks simultaneously.
- **Cloud Native**: Powered by Apache OpenDAL, natively supporting reading and writing from local filesystems, `s3://`, `http://`, and `https://` with standard AWS credentials.
- **Spatial Pruning**: Filter data at the chunk-level using bounding boxes (`lat_min`, `lon_max`), preventing out-of-bounds S3 requests before they are ever made.
- **Universal Types**: Supports all common Zarr primitives (`f32`, `f64`, `i8`, `i16`, `i32`, `i64`, `u8`, `u16`, `u32`, `u64`, `bool`, and `String`).
- **Missing Data Awareness**: Missing data tokens (Zarr `fill_value`s) are mapped perfectly to true SQL `NULL`s via DuckDB's `ValidityMask`.
- **GeoZarr Spec Alignment**: Natively parses GeoZarr `spatial` affine transforms to project grid coordinates into geographic coordinates (e.g., `lon`, `lat`) on-the-fly, and exposes global properties like `crs` via `read_zarr_metadata()`.

## Quick Start (Reading)

Download the `.duckdb_extension` binary for your platform from the [Releases page](https://github.com/dnf0/zarrduck/releases), or build it from source.

```sql
-- Allow unsigned extensions
SET allow_unsigned_extensions = true;

-- Load the extension
LOAD '/path/to/zarrduck_extension.duckdb_extension';

-- Query a remote Zarr array, aggregating over a specific spatial bounding box
SELECT
    time,
    AVG(value) as mean_temp
FROM read_zarr(
    's3://climate-data/temperature.zarr',
    lat_min := 45.0,
    lat_max := 55.0
)
GROUP BY time;
```

## Zarrduck CLI (Agentic Data Engine)

The companion `zarrduck` CLI allows you to perform complex spatial operations and STAC discoveries directly from the terminal. It features a powerful, multi-level interactive Terminal User Interface (TUI) for human users, while remaining fully LLM-agent friendly via the `--output=json` flag.

```bash
# 1. Multi-Level STAC Discovery (Interactive TUI)
# Run without arguments to launch the guided interactive explorer.
# Navigate Providers -> Collections -> Dataset URIs -> Zarr Channels
# Supports smart multi-word filtering and STAC descriptions!
zarrduck search --bbox -122.27,37.77,-122.22,37.81

# 2. Vector-Raster Extraction
# Downloads only intersecting chunks and joins spatial pixels with vector polygons
zarrduck extract climate_data.zarr/air_temperature ./my_region.geojson --out analysis.duckdb

# 3. Temporal Analytics
# Resample massive time-series data to coarser frequencies (e.g., monthly averages)
zarrduck resample analysis.duckdb monthly.duckdb --freq month --agg avg

# 4. Interactive SQL Shell
# Drop into a spatial-enabled REPL to query your extracted data
zarrduck shell monthly.duckdb
```

## Development

The project is structured as a Cargo workspace:
- `geozarr_core/`: The deep, pure Rust domain model handling Zarr metadata, coordinate projection, bounds validation, and types. Free from sink-specific (e.g., DuckDB) or UI dependencies.
- `extension/`: The core DuckDB loadable extension, acting as a thin C-API adapter over `geozarr_core`.
- `cli/`: The companion `zarrduck` extraction and analysis tool, acting as a command router.

To build both:
```bash
git clone https://github.com/dnf0/zarrduck.git
cd zarrduck
cargo build --release
```

## Documentation

Full documentation on installation, advanced spatial pruning, and architecture details can be found at [dnf0.github.io/zarrduck/](https://dnf0.github.io/zarrduck/).
