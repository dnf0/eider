Output docs/src/demo.gif

Set FontSize 16
Set Width 1200
Set Height 800
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
Type "export ZARRDUCK_LOCAL_STAC='http://localhost:8080 - Local Demo Catalog'"
Enter
Type "export PATH=$PWD/target/debug:$PATH"
Enter
Type "rm -f *.duckdb"
Enter
Ctrl+L
Show

# Step 1: STAC Discovery
Hide
Type "echo -e '\033[1A\033[2K\e[1;36m# 1. Discover cloud-native GeoZarr assets via STAC\e[0m'"
Enter
Show
Sleep 1s
Type "zarrduck search --bbox -122.27,37.77,-122.22,37.81"
Enter
Sleep 2s
Type "demo"
Sleep 1s
Enter
Sleep 2s
Type "mock"
Sleep 1s
Enter
Sleep 2s
Type "local"
Sleep 1s
Enter
Sleep 2s
Type "air_temp"
Sleep 1s
Enter
Sleep 4s

# Step 2: Zonal Extraction
Hide
Type "echo -e '\033[1A\033[2K\n\e[1;36m# 2. Extract and join spatial data\e[0m'"
Enter
Show
Sleep 1s
Type "zarrduck extract climate_data.zarr/air_temperature scripts/demo_region.geojson --out analysis.duckdb"
Enter
Sleep 4s

# Step 3: Temporal Resampling
Hide
Type "echo -e '\033[1A\033[2K\n\e[1;36m# 3. Automatically aggregate time-series data using interactive menus\e[0m'"
Enter
Show
Sleep 1s
Type "zarrduck resample analysis.duckdb monthly.duckdb"
Enter
Sleep 2s
Type "mon"
Sleep 1s
Enter
Sleep 2s
Type "avg"
Sleep 1s
Enter
Sleep 3s

# Step 4: Interactive Analysis
Hide
Type "echo -e '\033[1A\033[2K\n\e[1;36m# 4. Drop into a spatial-enabled shell\e[0m'"
Enter
Show
Sleep 1s
Type "zarrduck shell monthly.duckdb"
Enter
Sleep 3s
Type "SELECT time, lat, lon, value FROM resampled_data LIMIT 5;"
Enter
Sleep 4s

Type ".exit"
Enter
Sleep 2s