import os
import shutil
import urllib.request
import xarray as xr
import zarr

def main():
    store_path = "climate_data.zarr"
    nc_path = "air.mon.mean.nc"

    if os.path.exists(store_path):
        shutil.rmtree(store_path)

    if not os.path.exists(nc_path):
        print("Downloading real climate dataset from NOAA...")
        url = "https://downloads.psl.noaa.gov/Datasets/ncep.reanalysis.derived/surface/air.mon.mean.nc"
        temp_nc_path = nc_path + ".tmp"
        urllib.request.urlretrieve(url, temp_nc_path)
        shutil.move(temp_nc_path, nc_path)

    print("Converting NetCDF to Zarr...")
    ds = xr.open_dataset(nc_path, engine='netcdf4')

    # Convert longitude from 0-360 to -180 to 180 to match standard geojson/stac
    ds.coords['lon'] = (ds.coords['lon'] + 180) % 360 - 180
    ds = ds.sortby(ds.lon)

    # Optional: we can chunk it nicely
    ds = ds.chunk({'time': 12, 'lat': 73, 'lon': 144})

    # Let's quickly ensure the arrays map well for eider
    # We rename 'air' to 'air_temperature' so it matches the tape
    if 'air' in ds:
        ds = ds.rename({'air': 'air_temperature'})

    # Write to Zarr
    ds.to_zarr(store_path, mode='w', consolidated=True)

    # Inject GeoZarr spatial metadata into the array attributes
    # The resolution for NCEP is 2.5 degrees.
    root = zarr.open(store_path, mode='a')
    air_temp = root['air_temperature']
    air_temp.attrs['geozarr'] = {
        "spatial_reference": {"crs": "EPSG:4326"},
        "spatial_transform": {
            "scale": [1.0, -2.5, 2.5],
            "translation": [0.0, 90.0, -180.0]
        }
    }

    print("Generated actual data demo store at climate_data.zarr")

if __name__ == "__main__":
    main()
