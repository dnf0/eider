Output docs/src/demo.gif

Set FontSize 16
Set Width 1200
Set Height 800
Set TypingSpeed 50ms
Set Theme "Dracula"

# Build the project to ensure the binary is fresh before recording
Hide
Type "cargo clean -p zarrduck -p duckdb_geozarr && cargo build -p zarrduck && cd extension && cargo duckdb-ext build --duckdb-version v1.5.2 -- --features loadable-extension && cd .."
Enter
Sleep 10s
Type "pip3 install -r requirements.txt"
Enter
Sleep 5s
Type "export ZARRDUCK_LOCAL_STAC='http://localhost:8080 - Local Demo Catalog'"
Enter
Type "export PATH=$PWD/target/debug:$PATH"
Enter
Type "rm -f *.duckdb"
Enter
Type "clear"
Enter
Show

# Step 1: STAC Discovery
Hide
Type "printf '\033[1A\033[2K\033[1;36m# 1. Discover cloud-native GeoZarr assets via STAC\033[0m\n'"
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
Type "cmip6"
Sleep 1s
Enter
Sleep 2s
Type "tas"
Sleep 1s
Enter
Sleep 4s

# Step 2: Zonal Extraction (streaming directly from Google Cloud Storage)
Hide
Type "printf '\033[1A\033[2K\n\033[1;36m# 2. Extract and join spatial data\033[0m\n'"
Enter
Show
Sleep 1s
Type "zarrduck extract https://storage.googleapis.com/cmip6/CMIP6/CMIP/NCAR/CESM2/historical/r1i1p1f1/Amon/tas/gn/v20190308/tas scripts/demo_region.geojson --out analysis.duckdb"
Enter
Sleep 3s
Enter
Sleep 8s

# Step 3: Temporal Resampling
Hide
Type "printf '\033[1A\033[2K\n\033[1;36m# 3. Automatically aggregate time-series data using interactive menus\033[0m\n'"
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
Type "printf '\033[1A\033[2K\n\033[1;36m# 4. Drop into a spatial-enabled shell\033[0m\n'"
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

# Step 5: Data Visualization
Hide
Type "printf '\033[1A\033[2K\n\033[1;36m# 5. Interactive data visualization\033[0m\n'"
Enter
Show
Sleep 1s
Type "zarrduck plot analysis.duckdb"
Enter
Sleep 2s
Type "extracted"
Sleep 1s
Enter
Sleep 2s
Down 3
Sleep 500ms
Space
Sleep 500ms
Enter
Sleep 2s
Type "Heatmap"
Sleep 1s
Enter
Sleep 8s
