# DuckDB GeoZarr Data Fetching & Iteration Design

**Date:** 2026-05-15
**Status:** Approved

## 1. Purpose & Context
Following the implementation of dynamic schema generation, the `duckdb_geozarr` extension needs to fetch actual data from the Zarr store and yield it to DuckDB's execution engine. DuckDB processes data in batches (DataChunks) of up to 2048 rows. Zarr arrays are stored in multi-dimensional chunks that are usually much larger. The challenge is streaming this data without loading the entire array into memory at once, and efficiently flattening the N-dimensional data into 1D vectors.

## 2. Architecture: The Chunk Cursor

We will use a "Chunk Cursor" state machine to buffer and yield data.

### 2.1 State Representation
The iteration state will be maintained across `func` calls using a thread-safe `Mutex` inside `ReadZarrInitData`:

```rust
pub struct IterationState {
    /// The grid coordinates of the chunk currently being processed (e.g. [0, 1, 0])
    pub current_chunk_grid: Vec<u64>,
    /// The flattened 1D cursor index within the current buffered chunk
    pub local_chunk_cursor: usize,
    /// The raw typed data of the currently loaded chunk
    pub current_chunk_buffer: Option<ArrayBytes>,
    /// Flag indicating if all chunks in the array have been processed
    pub exhausted: bool,
}
```

### 2.2 The Fetching Loop (`func`)
1. **Buffer Check:** If `current_chunk_buffer` is `None`, calculate the spatial bounds of `current_chunk_grid`.
2. **Fetch:** Use the `zarrs` crate to retrieve the chunk. Decompress it into memory and store it in `current_chunk_buffer`. If `current_chunk_grid` exceeds the array's chunk grid shape, set `exhausted = true` and return `0`.
3. **Yield Batch:** Calculate the number of elements to yield: `min(2048, chunk_length - local_chunk_cursor)`. Extract these values and write them to the final `value` column in the `DataChunk`.
4. **Advance Cursor:** Increment `local_chunk_cursor`. If the cursor reaches the end of the chunk, increment `current_chunk_grid` to the next chunk in C-contiguous order and set `current_chunk_buffer = None`.

### 2.3 Coordinate Generation (Flattening)
While yielding the batch of 2048 values, the spatial coordinates (the outer dimension columns) must be calculated.
1. The `local_chunk_cursor` integer is mapped to local `[z, y, x]` coordinates using division and modulo operations based on the chunk's shape.
2. The global offset of the chunk (calculated from `current_chunk_grid * chunk_shape`) is added to the local coordinates to produce absolute integer indices.
3. These indices are written into the dynamically generated coordinate columns of the `DataChunk`.

## 3. Data Types
Initially, we will support fetching arrays of numerical data types (e.g. `Float32`, `Float64`, `Int32`, `Int64`). Handling of specialized Zarr V3 string arrays or complex types will be deferred. Coordinate columns will be yielded as `BigInt` (DuckDB's 64-bit integer type).

## 4. Error Handling
If `zarrs` fails to fetch or decode a chunk (e.g., S3 network timeout or corrupted compression), the error will be propagated up through the `vtab` interface, safely halting the DuckDB query.

## 5. Spec Self-Review
1. **Placeholder scan:** No TBDs or TODOs found.
2. **Internal consistency:** The state struct correctly aligns with the fetching loop description.
3. **Scope check:** This is focused solely on iteration and integer indices. Physical value resolution is intentionally deferred.
4. **Ambiguity check:** C-contiguous traversal order is explicitly defined for advancing the chunk grid. ArrayBytes is used as an abstract representation of the decoded buffer.