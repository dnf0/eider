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
