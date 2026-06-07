---
sidebar_position: 2
---

# eider info

Inspect a Zarr array's metadata (shape, chunking, type, CRS) without reading data.

## Synopsis

```
eider info <uri> [--pin DIM=INDEX]... [--output table|json]
```

## Arguments

- `uri` — the Zarr array URI (local path, `s3://`, or `http(s)://`).

## Options

| Option | Description |
|---|---|
| `--pin DIM=INDEX` | Pin a dimension to a fixed index (repeatable), e.g. `--pin time=0`. |

## Examples

```bash
eider info climate_data.zarr/air_temperature
```

JSON mode returns the array metadata:

```bash
eider info climate_data.zarr/air_temperature --output=json
```

```json
{"uri":"climate_data.zarr/air_temperature","array_shape":"[938, 73, 144]","chunk_shape":"[12, 73, 144]","data_type":"Float32","crs":"EPSG:4326"}
```

## See also
- [read_zarr_metadata](./sql_read_zarr_metadata.md) — the same metadata from SQL.
