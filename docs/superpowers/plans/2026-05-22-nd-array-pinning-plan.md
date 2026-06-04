# N-Dimensional Array Pinning (4D+ Support) Plan

## Objective
Enable Eider to query and export high-dimensional GeoZarr datasets (4D+) such as climate ensembles with `[member, time, altitude, lat, lon]` dimensions. This requires giving users the ability to "pin" or "slice" extra dimensions so the resulting output can be projected into a flat relational table.

## Current Architecture Friction
Eider assumes arrays are primarily 3D (`time`, `lat`, `lon`). While the `GridIterator` and chunk reading math handles N-dimensions generically, the `QueryPlanner` and DuckDB adapter expect to yield all dimensions as columns. If a 5D array is queried, yielding a full Cartesian product of all dimensions would result in an exponential explosion of rows and likely crash the query.

## Proposed Solution: Dimension Pinning
Introduce a "pinning" mechanism where users can specify a fixed index or coordinate value for specific dimensions. The `QueryPlanner` will lock the bounding box for that dimension to a single slice (`min_idx = max_idx = pinned_idx`). The `GridIterator` will then only iterate over the unpinned dimensions.

## Implementation Steps

### 1. Extend the Domain Model
- Update `geozarr_core::dataset::GeoZarrDataset` or create a new `QueryConstraints` struct to accept a `pinned_dimensions: HashMap<String, u64>` map.

### 2. Update `QueryPlanner`
- Modify `compute_bounds` in `geozarr_core/src/query_planner.rs`.
- If a dimension exists in `pinned_dimensions`, its `bounds_min` and `bounds_max` should both be set to the pinned index. This effectively flattens that dimension during iteration.

### 3. DuckDB Adapter (`eider_extension`)
- Expose pinning parameters in the `read_zarr` table function.
- Example API: `SELECT * FROM read_zarr('data.zarr', altitude_index := 5, ensemble_member := 0)`.
- In `ReadZarrVTab::bind`, parse these dynamic named parameters. If a parameter matches a dimension name (but isn't a `_min`/`_max` bounds query), treat it as a dimension pin.
- Pass the parsed pins to `dataset.compute_bounds()`.
- *Optimization*: Optionally omit pinned dimensions from the DuckDB output schema, or yield them as constant scalar columns to save memory.

### 4. CLI Enhancements (`eider`)
- Add a `--pin <DIM>=<INDEX>` flag to the `eider extract`, `plot`, and `info` commands.
- Example: `eider extract data.zarr region.geojson --pin altitude=5 --pin ensemble=0`.
- Pass these pins down to the underlying SQL query generation.

### 5. Verification
- Create a synthetic 4D Zarr array in the test suite.
- Write a unit test in `geozarr_core` asserting that `compute_bounds` correctly restricts the grid iterator to a single slice for pinned dimensions.
- Write an integration test executing a DuckDB query against the 4D array with and without pinning, asserting row counts.

## Acceptance Criteria
- 4D+ Zarr arrays can be queried without out-of-memory errors by pinning extra dimensions.
- The `read_zarr` table function accepts arbitrary dimension indices as named parameters.
- The `eider` CLI accepts a `--pin` flag for interactive extraction and plotting.
