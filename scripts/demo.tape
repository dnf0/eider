Output docs/src/demo.gif

Set FontSize 16
Set Width 1200
Set Height 800
Set TypingSpeed 50ms
Set Theme "Dracula"

Hide
Type "export PATH=$PWD/target/debug:$PATH"
Enter
Type "rm -f *.duckdb"
Enter
Type "python3 scripts/mock_stac.py 8123 &"
Enter
Sleep 1s
Type "export EIDER_LOCAL_STAC='http://localhost:8123 - Local Demo Catalog'"
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
Type "eider search --bbox -122.27,37.77,-122.22,37.81"
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

# Step 2: Zonal Extraction
Hide
Type "printf '\033[1A\033[2K\n\033[1;36m# 2. Extract and join spatial data\033[0m\n'"
Enter
Show
Sleep 1s
Type "eider extract climate_data.zarr/air_temperature scripts/demo_region.geojson --out analysis.duckdb"
Enter
Sleep 3s
Enter
Sleep 4s

# Step 3: Temporal Resampling
Hide
Type "printf '\033[1A\033[2K\n\033[1;36m# 3. Automatically aggregate time-series data using interactive menus\033[0m\n'"
Enter
Show
Sleep 1s
Type "eider resample analysis.duckdb monthly.duckdb"
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
Type "eider shell monthly.duckdb"
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
Type "eider plot analysis.duckdb"
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
