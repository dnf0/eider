# Unified `read_geo` Table Function Design Specification

## Overview
This specification details the architecture to refactor the DuckDB extension from a single-array `read_zarr` function into a unified `read_geo` function capable of querying both standard Zarr/COG datasets and massive STAC `FeatureCollection` endpoints, directly pushing down spatial and temporal filters to the STAC API.

## Core Architectural Changes

### 1. The `GeoDataset` Trait Boundary
Currently, `extension/src/table_function.rs` is tightly coupled to `geozarr_core::dataset::GeoZarrDataset`. We will introduce a trait boundary `GeoDataset` in `geozarr_core` that abstracts away the underlying storage.

```rust
pub trait GeoDataset {
    /// Returns the schema of the dataset (e.g., ["lat": Float64, "lon": Float64, "time": Float64, "value": Float32])
    fn schema(&self) -> Result<Vec<(String, zarrs::array::DataType)>, Box<dyn std::error::Error>>;
    
    /// Prepares the dataset for scanning based on DuckDB's pushdown constraints
    fn plan_scan(&self, constraints: &QueryConstraints) -> Result<ScanPlan, Box<dyn std::error::Error>>;
    
    /// Returns the total number of chunks (used by DuckDB to allocate threads)
    fn num_chunks(&self, plan: &ScanPlan) -> u64;

    /// Reads a specific chunk from the plan into DuckDB vectors
    fn read_chunk(&self, plan: &ScanPlan, chunk_idx: u64, output: &mut ChunkBuffer) -> Result<(), Box<dyn std::error::Error>>;
}
```

### 2. Implementation: `ZarrDataset`
The existing `GeoZarrDataset` will be renamed to `ZarrDataset` and will implement the `GeoDataset` trait. 
- It uses the standard Zarr multidimensional array metadata.
- `plan_scan` calculates the bounding box over the array indices.
- `read_chunk` fetches the chunk via byte-range requests and decodes it.

### 3. Implementation: `FeatureCollectionDataset`
A new implementation `FeatureCollectionDataset` will handle STAC API queries.
- **Initialization**: When `read_geo(url)` is called and the URL is a STAC Search endpoint, it instantiates this dataset.
- **Filter Pushdown**: When `plan_scan` receives constraints (e.g., `lat_min`, `time_max`), it appends them to the STAC URL (`&bbox=...&datetime=...`) and makes a single HTTP GET request to the STAC API.
- **Zero-Header Planning**: The returned STAC `FeatureCollection` contains a list of Items. The `ScanPlan` simply assigns each Item's COG asset (e.g., `swir22`) to the iteration space. 
- **Threading**: If the STAC API returns 50 items, `num_chunks()` could return 50. DuckDB assigns 50 threads. 
- **Lazy Execution**: Inside `read_chunk()`, the thread reads the STAC Item JSON from the plan, resolves the actual S3/HTTP URL, dynamically creates a `VirtualCogStore` for that single COG, fetches its 16KB header, reads the pixel data, transforms it to `(lat, lon, time, value)` using the item's STAC bounding box/transform, and streams it to DuckDB.

### 4. Table Function Updates
The DuckDB extension entry points will be updated:
- Rename `read_zarr` -> `read_geo`.
- `read_geo`'s `bind` function will parse the URL, instantiate either `ZarrDataset` or `FeatureCollectionDataset` (returning a `Box<dyn GeoDataset>`), and store it in `BindData`.
- `read_geo`'s `func` function will blindly invoke `dataset.read_chunk()` across the available threads.

## STAC Pushdown Mapping
DuckDB Constraints mapping to STAC parameters:
- `lat_min`, `lat_max`, `lon_min`, `lon_max` -> `bbox=[lon_min, lat_min, lon_max, lat_max]`
- `time_min`, `time_max` -> `datetime=time_min/time_max` (Requires translating UNIX epoch back to ISO8601 strings).

## Conclusion
This spec completely decouples the DuckDB extension from array mechanics, allowing STAC collections to be iterated effortlessly. DuckDB handles the heavy lifting of appending overlapping coordinate rows.
