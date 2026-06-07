---
sidebar_position: 3
---

# SQL Quickstart

Query a Zarr array as a SQL table in about five minutes. See
[Installation](./installation.md) to get the extension first.

This quickstart uses the sample dataset from the repo. From a clone:

```bash
python3 scripts/generate_demo_data.py   # writes ./climate_data.zarr
```

The same queries work against remote `s3://` / `https://` Zarr — just swap the path.

## 1. Launch DuckDB and load Eider

```bash
duckdb -unsigned
```

```sql
LOAD '/absolute/path/to/eider.duckdb_extension';
```

## 2. Inspect the array

```sql
SELECT array_shape, chunk_shape, data_type
FROM read_zarr_metadata('climate_data.zarr/air_temperature');
```

```
┌────────────────┬─────────────────────────────────┬───────────┐
│  array_shape   │           chunk_shape           │ data_type │
├────────────────┼─────────────────────────────────┼───────────┤
│ [938, 73, 144] │ Some(ChunkShape([12, 73, 144])) │ Float32   │
└────────────────┴─────────────────────────────────┴───────────┘
```

## 3. Query with a spatial bounding box

`read_geo` streams only the chunks that intersect your bounds:

```sql
SELECT lat, AVG(value) AS mean_temp
FROM read_geo('climate_data.zarr/air_temperature', lat_min := 30, lat_max := 50)
GROUP BY lat
ORDER BY lat
LIMIT 5;
```

```
┌────────┬────────────────────┐
│  lat   │     mean_temp      │
├────────┼────────────────────┤
│   30.0 │  19.09712200024641 │
│   32.5 │ 16.925729751506257 │
│   35.0 │ 14.822018816108104 │
│   37.5 │ 13.730744419699588 │
│   40.0 │ 12.425807312232353 │
└────────┴────────────────────┘
```

## Next steps

- **SQL Reference** — all table functions and parameters.
- **Guides** — exporting results, cloud access.
