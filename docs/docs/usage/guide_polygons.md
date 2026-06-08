---
sidebar_position: 2
---

import Tabs from '@theme/Tabs';
import TabItem from '@theme/TabItem';

# Extracting with polygons

`eider extract` pulls the grid cells that fall inside a vector boundary. The
boundary can hold **several polygons at once** — a cell is kept if it lies in
**any** of them (union) — and each polygon's properties travel through to the
output, so you can aggregate **per polygon**.

This guide uses the sample dataset (see [Installation](./installation.md)):

```bash
python3 scripts/generate_demo_data.py   # writes ./climate_data.zarr
```

## 1. Define multiple polygons

`scripts/demo_polygons.geojson` is a `FeatureCollection` with two named,
non-rectangular regions:

```json
{
  "type": "FeatureCollection",
  "features": [
    { "type": "Feature", "properties": { "name": "west" },
      "geometry": { "type": "Polygon",
        "coordinates": [[[-130,30],[-100,30],[-115,55],[-130,30]]] } },
    { "type": "Feature", "properties": { "name": "east" },
      "geometry": { "type": "Polygon",
        "coordinates": [[[60,10],[110,10],[85,40],[60,10]]] } }
  ]
}
```

The `name` property identifies each polygon in the results.

## 2. Extract the cells inside the polygons

You can perform the extraction using the CLI or natively in DuckDB using the Eider extension.

<Tabs>
<TabItem value="cli" label="CLI" default>

```bash
eider extract climate_data.zarr/air_temperature scripts/demo_polygons.geojson \
  --out analysis.duckdb --yes
```

```text
Target Area: 79 chunks
Data Volume: 38.01 MB
- SUCCESS: Extraction complete!
Run `eider shell analysis.duckdb` to explore the extracted data.
```

</TabItem>
<TabItem value="sql" label="DuckDB SQL">

```sql
D INSTALL spatial; LOAD spatial;
D INSTALL eider; LOAD eider;
D CREATE TABLE extracted_data AS
  SELECT z.*, v.* EXCLUDE (geom)
  FROM read_geo('climate_data.zarr/air_temperature') z,
       ST_Read('scripts/demo_polygons.geojson') v
  WHERE ST_Contains(v.geom, ST_Point(z.lon, z.lat));
```

</TabItem>
</Tabs>

This writes an `extracted_data` table containing only the cells inside `west`
or `east`, with a `name` column tagging which polygon each cell came from. See
[`eider extract`](./cli_extract.md).

## 3. Aggregate per polygon

Group by `name` to get one row per polygon — here, the maximum value in each:

```sql
SELECT name, MAX(value) AS max_temp
FROM extracted_data
GROUP BY name
ORDER BY name;
```

Run it in the SQL shell (`eider shell analysis.duckdb`):

```text
┌─────────┬───────────┐
│  name   │ max_temp  │
│ varchar │   float   │
├─────────┼───────────┤
│ east    │ 39.158062 │
│ west    │ 33.116116 │
└─────────┴───────────┘
```

One row per polygon, computed directly in DuckDB. See [`eider shell`](./cli_shell.md).

## 4. See the mask

Render the extracted cells as a heatmap — the populated cells trace the two
polygon shapes; everything outside is absent:

```bash
eider plot analysis.duckdb --plot-type heatmap
```

```text
Heatmap of value (Spatial):

   52.50 ┤   ██
         │   ██
         │   ██
         │ ██████
         │ ██████
         │ ██████
         │ ██████                                                ██
         │ ██████                                              ████
         │ ████████                                            ████
         │                                                     ██████
         │                                                   ████████
         │                                                   ████████
         │                                                   ████████
         │                                                   ██████████
         │                                                 ████████████
         │                                                 ████████████
   12.50 ┤                                                 ████████████
          └────────────────────────────────────────────────────────────
           -127.50                                               107.50
```

The two clusters are the `west` and `east` triangles; the rest of the globe is
masked out. See [`eider plot`](./cli_plot.md).

## Performance

The polygon-to-aggregate path is what Eider is built for: it reads only the
chunks the polygons touch, streams the cells straight into DuckDB, and lets
DuckDB do the grouping. On the sample dataset (a global
938 × 73 × 144 array), extracting both polygons materialized **151,956 cells in
~0.6 s**, and the per-polygon `MAX` aggregation over them ran in **~10 ms**.

(Measured on a laptop with a debug build over the local sample; a release build
and warm OS cache are faster. Treat these as order-of-magnitude — the point is
that spatial subsetting plus aggregation stays interactive.)

## Next steps

- [End-to-end analysis workflow](./guide_workflow.md)
- [SQL Reference](./sql_reference.md) — query `extracted_data` directly.
