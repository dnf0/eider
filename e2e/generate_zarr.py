import zarr
import numpy as np
import os
import shutil

def main():
    store_path = os.environ.get('ZARR_STORE_PATH', os.path.join(os.path.dirname(__file__), 'data', 'test.zarr'))
    if os.path.exists(store_path):
        shutil.rmtree(store_path)

    # Initialize a Zarr root group
    root = zarr.group(store=store_path)

    # Create a 2D array of floats with a fill value
    z = root.create_dataset(
        'data',
        shape=(1000, 1000),
        chunks=(100, 100),
        dtype='f8',
        fill_value=-9999.0
    )

    # Fill with some data, leaving some as fill_value
    data = np.ones((1000, 1000), dtype='f8') * 42.0
    # Add some fill values
    data[0:100, 0:100] = -9999.0
    z[:] = data

    # Create coordinate arrays
    lat = root.create_dataset('lat', shape=(1000,), chunks=(100,), dtype='f8')
    lat[:] = np.linspace(0, 99.9, 1000)

    lon = root.create_dataset('lon', shape=(1000,), chunks=(100,), dtype='f8')
    lon[:] = np.linspace(0, 99.9, 1000)

    print(f"Generated Zarr array at {store_path}")

if __name__ == '__main__':
    main()
