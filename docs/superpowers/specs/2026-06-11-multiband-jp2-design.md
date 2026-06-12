# Multi-band COG and JP2 Support

## Overview
We will update `geozarr_core` to support multi-band interleaved COGs natively in Rust. For `.jp2` and other unsupported formats, we will introduce a graceful fallback mechanism.

## Multi-band COGs
- Update `cog.rs` to parse `SamplesPerPixel` and support interleaved data.
- De-interleave the TIFF pixel data (from pixel-interleaved to planar) in the chunk decoder so that it matches Zarr's expected memory layout.
- Expose the multi-band data as an extra dimension in the Zarr metadata (e.g., `[bands, y, x]`).

## JP2 Fallback
- Detect `.jp2` files or unsupported compressions.
- We will document and possibly provide a SQL macro or helper function that delegates the reading of these files to DuckDB's native `spatial` extension (`st_read`), which uses GDAL under the hood.
