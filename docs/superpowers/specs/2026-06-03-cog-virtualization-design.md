# Native COG Virtualization Design

## Overview
This feature introduces a native Cloud Optimized GeoTIFF (COG) parser within `eider`'s core Rust library (`geozarr_core`). By generating virtual Zarr metadata in-memory and translating Zarr chunk requests into precise COG tile byte-range reads over HTTP, `eider` can seamlessly query `.tif` datasets without any external Kerchunk/VirtualiZarr Python steps.

## Architecture & Components

### 1. `VirtualCogStore`
A new struct residing in `geozarr_core/src/store.rs` that implements the `zarrs::storage::ReadableStorageTraits` trait. This acts as a man-in-the-middle between the Zarr codec engine and the raw OpenDAL byte-stream of the `.tif` file.

### 2. Initialization & Data Flow
When `resolve_sync_store` encounters a path ending in `.tif` or `.tiff`, the following occurs:
1. **Header Fetch:** We use an OpenDAL byte-range read to fetch the first ~16KB of the COG file to parse the TIFF Header and the first Image File Directory (IFD).
2. **Metadata Extraction:** A minimal custom (or lightweight crate-based) TIFF parser extracts the essential tags:
   - `ImageWidth`, `ImageLength`
   - `TileWidth`, `TileLength`
   - `TileOffsets`, `TileByteCounts`
   - `GeoDoubleParamsTag` (for spatial bounding box extraction/projection)
3. **Virtual Metadata Generation:** The `VirtualCogStore` synthesizes a `.zmetadata` (or Zarr V3 `zarr.json`) document in-memory. It sets the Zarr shape and chunk boundaries to perfectly match the COG's dimensions and tile layout, presenting the COG to `zarrs` as a valid Zarr array.

### 3. Chunk Retrieval Interception
During query execution:
1. `zarrs` receives spatial pruning constraints from DuckDB and requests specific spatial chunks (e.g., `chunk(0, 5, 5)`).
2. The `VirtualCogStore`'s `get_partial_values` or `get` method is invoked.
3. The store translates the multi-dimensional Zarr chunk indices into a flat index for the COG's `TileOffsets` array.
4. Using the offset and `TileByteCounts` length, it issues a precise OpenDAL HTTP byte-range request (`Range: bytes=offset-(offset+length-1)`) directly to the original COG URL.
5. The fetched byte buffer is handed back to `zarrs` for native decompression and decoding.

## Trade-offs & Advantages
- **Speed over Memory:** By parsing the binary TIFF tags directly into compact numerical arrays (`Vec<u64>`), we eliminate the start-up latency (100s of milliseconds) inherent in parsing massive JSON-based Kerchunk reference maps for large COGs.
- **Zero-Copy Streaming:** Network bytes stream straight from HTTP into DuckDB memory through `zarrs`, bypassing Python and intermediate disks completely.

## Limitations & Future Work
- Initially, we will support the standard compression types used by COGs (e.g., Deflate, LZW) by mapping them to equivalent `zarrs` compressors, but unsupported compressions may fail.
- Multi-band COGs will be exposed as 3D Zarr arrays `(band, y, x)`. Overviews (pyramids) are ignored for standard extraction queries but could be mapped as separate Zarr groups in the future.
