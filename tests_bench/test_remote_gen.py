"""Tests for the Range-capable byte-logging server + remote store generators."""

import os
import sys
from pathlib import Path

import requests

REPO_ROOT = Path(__file__).resolve().parents[1]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from scripts.bench_remote_partialread import (  # noqa: E402
    generate_cog,
    generate_zarr,
    start_server,
    window_bbox,
)


def test_server_full_get(tmp_path):
    payload = b"0123456789" * 50  # 500 bytes
    (tmp_path / "data.bin").write_bytes(payload)
    server, port, acc = start_server(tmp_path)
    try:
        acc.reset()
        resp = requests.get(f"http://127.0.0.1:{port}/data.bin", timeout=5)
        assert resp.status_code == 200
        assert resp.content == payload
        snap = acc.snapshot()
        assert snap["n_requests"] == 1
        assert snap["total_bytes"] == len(payload)
        assert snap["per_path"]["/data.bin"]["requests"] == 1
    finally:
        server.shutdown()


def test_server_range_get(tmp_path):
    payload = b"0123456789" * 50  # 500 bytes
    (tmp_path / "data.bin").write_bytes(payload)
    server, port, acc = start_server(tmp_path)
    try:
        acc.reset()
        resp = requests.get(
            f"http://127.0.0.1:{port}/data.bin",
            headers={"Range": "bytes=0-99"},
            timeout=5,
        )
        assert resp.status_code == 206
        assert len(resp.content) == 100
        assert resp.content == payload[:100]
        assert resp.headers["Content-Range"] == "bytes 0-99/500"
        snap = acc.snapshot()
        assert snap["n_requests"] == 1
        assert snap["total_bytes"] == 100
        # The recorded request is flagged as a range request.
        rec = snap["per_path"]["/data.bin"]
        assert rec["bytes"] == 100
    finally:
        server.shutdown()


def test_server_range_is_flagged(tmp_path):
    (tmp_path / "data.bin").write_bytes(b"x" * 200)
    server, port, acc = start_server(tmp_path)
    try:
        acc.reset()
        requests.get(
            f"http://127.0.0.1:{port}/data.bin",
            headers={"Range": "bytes=10-19"},
            timeout=5,
        )
        # Reach into the accumulator records to confirm is_range=True.
        records = acc._records  # noqa: SLF001 (test introspection)
        assert len(records) == 1
        assert records[0].is_range is True
    finally:
        server.shutdown()


def test_generate_zarr_reopens(tmp_path):
    import xarray as xr

    info = generate_zarr(tmp_path, shape=(512, 512), chunks=(128, 128), seed=1)
    ds = xr.open_zarr(str(info["store"]))
    var = ds[info["var"]]
    assert var.shape == (512, 512)
    assert var.dtype == "float32"
    assert "lat" in ds.coords and "lon" in ds.coords
    # Coordinates are monotonic ascending.
    import numpy as np

    assert np.all(np.diff(ds["lat"].values) > 0)
    assert np.all(np.diff(ds["lon"].values) > 0)


def test_generate_zarr_is_v2_not_v3(tmp_path):
    info = generate_zarr(tmp_path, shape=(256, 256), chunks=(128, 128), seed=1)
    store = info["store"]
    found_zarray = []
    found_zarr_json = []
    for root, _dirs, files in os.walk(store):
        for f in files:
            if f == ".zarray":
                found_zarray.append(os.path.join(root, f))
            if f == "zarr.json":
                found_zarr_json.append(os.path.join(root, f))
    # eider reads Zarr v2: expect v2 .zarray files, NOT v3 zarr.json.
    assert found_zarray, "expected zarr v2 .zarray metadata"
    assert not found_zarr_json, "found zarr v3 zarr.json; must be v2"


def test_generate_cog_reopens(tmp_path):
    import rasterio

    path = tmp_path / "cog.tif"
    generate_cog(path, shape=(512, 512), blocksize=256, seed=1)
    with rasterio.open(str(path)) as r:
        assert r.count == 1
        assert r.crs.to_epsg() == 4326
        assert r.width == 512
        assert r.height == 512
        assert r.dtypes[0] == "float32"
        # Tiled with the requested block size.
        assert r.profile.get("tiled") is True
        assert r.block_shapes[0] == (256, 256)
        # North-up: pixel height (transform.e) is negative.
        assert r.transform.e < 0


def test_window_bbox_centered():
    info = {"lon_min": -10.0, "lon_max": 10.0, "lat_min": -10.0, "lat_max": 10.0}
    lon_min, lat_min, lon_max, lat_max = window_bbox(info, 0.01)
    # Centered on (0, 0).
    assert abs((lon_min + lon_max) / 2.0) < 1e-9
    assert abs((lat_min + lat_max) / 2.0) < 1e-9
    # Area fraction ~ 0.01 => side fraction 0.1 => width 0.1 * 20 = 2.0 degrees.
    assert abs((lon_max - lon_min) - 2.0) < 1e-9
    assert abs((lat_max - lat_min) - 2.0) < 1e-9
    # Bbox stays within the grid.
    assert lon_min >= info["lon_min"] and lon_max <= info["lon_max"]
