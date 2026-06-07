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
