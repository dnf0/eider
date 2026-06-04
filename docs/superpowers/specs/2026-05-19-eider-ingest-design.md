# Eider Data Ingestion Engine Design

**Date:** 2026-05-19

## 1. Context & Purpose
Phase 3 of the Eider Future Roadmap introduces the Data Ingestion Engine. While the CLI currently excels at reading and extracting from Zarr, users often need to convert legacy geospatial formats (like NetCDF, GeoTIFF, or CSV) into cloud-native Zarr.

The purpose of this sub-project is to build the `eider ingest` command, effectively turning the CLI into an ETL pipeline. It will leverage DuckDB's spatial extensions to ingest legacy files and use our robust export engine to write them to S3 as GeoZarr, utilizing a hybrid auto-chunking strategy.

## 2. Core Architecture & Commands

We will add a new `ingest` subcommand to the `eider` CLI.

### 2.1 The `ingest` Command
- **Purpose:** Convert legacy spatial files into cloud-native GeoZarr.
- **Input:**
  - `<input_file>`: The local file to ingest (e.g., `data.nc`, `raster.tif`).
  - `<output_zarr_uri>`: The destination Zarr URI (e.g., `s3://bucket/data.zarr`).
  - `--chunks`: Optional JSON string to override automatic chunk sizes for specific dimensions.

### 2.2 Execution Flow
1. **Data Ingestion:** The CLI initializes an in-memory DuckDB connection, loads the `spatial` extension, and imports the `<input_file>` using DuckDB's `ST_Read()` or relevant spatial table functions.
2. **Schema Introspection:** The CLI analyzes the imported data to infer dimensions (time, lat, lon) and their overall shapes.
3. **Hybrid Auto-Chunking:**
   - The CLI heuristically calculates optimal chunk sizes (aiming for 10MB–50MB per chunk) based on the total dimensions to balance spatial vs. temporal query performance.
   - If the user provides the `--chunks` flag, those specific values override the heuristically derived chunk shapes.
4. **Metadata Generation:** The CLI constructs valid GeoZarr metadata, including `_ARRAY_DIMENSIONS` and any inferred spatial affine transformations.
5. **Streaming Export:** The CLI delegates the final step to the existing export logic, streaming the data chunk-by-chunk to the remote Zarr store on S3.

## 3. Agent Compatibility & TUI
- **Progress Feedback:** Since the export logic is being reused, the ingestion process will natively benefit from the `indicatif` progress bar implemented in the TUI phase.
- **JSON Mode:** If `--output=json` is provided, all visual progress bars are disabled, and the command finishes by emitting a clean JSON success payload indicating the successful creation of the Zarr store.

## 4. Development Strategy
1. Add the `Ingest` variant to the `Commands` enum in `cli/src/main.rs`.
2. Implement the DuckDB data loading and schema introspection logic.
3. Implement the hybrid auto-chunking algorithm.
4. Integrate the new logic with the existing streaming export pipeline.
