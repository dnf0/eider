"""Generate tiny deterministic COG fixtures for geozarr_core tests.

Run once to (re)produce the committed fixtures:
    pip install rasterio numpy
    python scripts/generate_cog_fixture.py

Emits, under geozarr_core/tests/fixtures/:
  - cog_int16_uncompressed.tif  (EPSG:4326, 4x2, Int16, no compression, predictor=1)
  - cog_int16_deflate.tif       (same data, Deflate, predictor=1)
Affine: origin (-180, 90), pixel size 2.0; so lon = -180 + 2*col, lat = 90 - 2*row.
Values: v[row, col] = row*10 + col  -> deterministic, easy to assert.
"""
import os
import numpy as np
import rasterio
from rasterio.transform import from_origin

OUT = os.path.join(os.path.dirname(__file__), "..", "geozarr_core", "tests", "fixtures")
os.makedirs(OUT, exist_ok=True)

data = np.array([[0, 1, 2, 3], [10, 11, 12, 13]], dtype=np.int16)  # rows=2, cols=4
transform = from_origin(-180.0, 90.0, 2.0, 2.0)  # west, north, xsize, ysize


def write(path, **extra):
    with rasterio.open(
        path, "w", driver="GTiff", height=2, width=4, count=1,
        dtype="int16", crs="EPSG:4326", transform=transform,
        tiled=True, blockxsize=16, blockysize=16, predictor=1, **extra,
    ) as dst:
        dst.write(data, 1)


write(os.path.join(OUT, "cog_int16_uncompressed.tif"), compress="none")
write(os.path.join(OUT, "cog_int16_deflate.tif"), compress="deflate")
print("Wrote fixtures to", os.path.abspath(OUT))
