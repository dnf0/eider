Output docs/src/demo.gif

Set FontSize 16
Set Width 1200
Set Height 600
Set TypingSpeed 50ms
Set Theme "Dracula"

# Build the project to ensure the binary is fresh before recording
Hide
Type "cargo build -p zarrduck && cd extension && cargo duckdb-ext build --duckdb-version v1.5.2 -- --features loadable-extension && cd .."
Enter
Sleep 10s
Type "pip3 install zarr numpy"
Enter
Sleep 5s
Type "python3 scripts/generate_demo_data.py"
Enter
Sleep 5s
Type "export PATH=$PWD/target/debug:$PATH"
Enter
Type "rm -f *.duckdb"
Enter
Type "clear"
Enter
Show

# Step 1: STAC Discovery
Type "clear"
Enter
Type "# 1. Discover cloud-native GeoZarr assets via STAC"
Enter
Sleep 1s
Type "zarrduck search --api https://planetarycomputer.microsoft.com/api/stac/v1 --collection era5-pds --bbox -122.27,37.77,-122.22,37.81"
Enter
Sleep 3s

# Step 2: Zonal Extraction
Type "clear"
Enter
Type "# 2. Perform a massive Vector-Raster join on the cloud"
Enter
Sleep 1s
Type "zarrduck extract climate_data.zarr/air_temperature scripts/demo_region.geojson --out analysis.duckdb"
Enter
Sleep 4s

# Step 3: Temporal Resampling
Type "clear"
Enter
Type "# 3. Automatically aggregate time-series data to monthly means"
Enter
Sleep 1s
Type "zarrduck resample analysis.duckdb monthly.duckdb --freq month --agg avg"
Enter
Sleep 3s

# Step 4: Interactive Analysis
Type "clear"
Enter
Type "# 4. Drop into a spatial-enabled shell"
Enter
Sleep 1s
Type "zarrduck shell monthly.duckdb"
Enter
Sleep 3s
Type "SELECT time, lat, lon, value FROM resampled_data LIMIT 5;"
Enter
Sleep 5s

Type ".exit"
Enter
Sleep 2s