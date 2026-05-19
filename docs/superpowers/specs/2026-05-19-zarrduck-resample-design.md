# Zarrduck CLI Temporal Analytics Design

**Date:** 2026-05-19

## 1. Context & Purpose
Following Phase 1 (STAC Discovery) of the Zarrduck Future Roadmap, Phase 2 focuses on Temporal Analytics. Geospatial data extracted from Zarr stores is almost always multi-dimensional, with time being a critical axis. 

Currently, users must manually write complex DuckDB SQL using window functions or `date_trunc` to aggregate this data (e.g., converting daily temperature grids into monthly averages). The purpose of this sub-project is to introduce a high-level `zarrduck resample` command that automates this temporal aggregation, leaning into our "High-Level Automation" philosophy.

## 2. Core Architecture & Commands

We will add a new `resample` subcommand to the `zarrduck` CLI.

### 2.1 The `resample` Command
- **Purpose:** Temporally aggregate extracted GeoZarr data.
- **Input:** 
  - `<input_db>`: A local DuckDB file (typically the output of `zarrduck extract`).
  - `<output_db>`: A target DuckDB file for the aggregated results.
  - `--freq`: The temporal frequency for the aggregation (e.g., `month`, `year`, `day`).
  - `--agg`: The SQL aggregate function to apply (e.g., `mean`, `sum`, `max`, `min`).

### 2.2 Auto-Detection Logic
To provide a seamless experience, the CLI will automatically infer the schema of the source data.
When the command runs, it will connect to the `<input_db>` and query the schema of the `extracted_data` table.
1. **Time Column:** It identifies the time axis by looking for columns named `time`, `date`, or `datetime`.
2. **Spatial Columns:** It identifies spatial coordinates by looking for columns named `lat`, `lon`, `x`, or `y`.
3. **Value Column:** It identifies the data value by assuming any numeric column that is not a recognized coordinate is the target.

If the schema is ambiguous (e.g., multiple unknown numeric columns), the command will fail gracefully with an `eyre` error prompting the user to use the `shell` for manual querying.

### 2.3 SQL Generation & Execution
Using the detected columns, the CLI generates and executes an optimized aggregation query inside the `<output_db>`:

```sql
ATTACH '<input_db>' AS source_db;
CREATE OR REPLACE TABLE resampled_data AS 
SELECT 
    date_trunc('<freq>', <time_col>) as time,
    <lat_col>, 
    <lon_col>, 
    <agg_func>(<value_col>) as value 
FROM source_db.extracted_data 
GROUP BY 1, 2, 3;
```

## 3. Agent Compatibility & TUI
- **Interactive Progress:** Like the `extract` command, temporal resampling executes as a single, blocking DuckDB query that may take significant time on large datasets. We will display an `indicatif` spinner (`"🔄 Resampling time-series data..."`) while the query processes.
- **JSON Mode:** If the global `--output=json` flag is provided (e.g., by an LLM agent), the spinner is entirely bypassed, and the command concludes by outputting a clean JSON success payload (`{"status": "success", "db": "<output_db>"}`).

## 4. Development Strategy
1. Add the `Resample` variant to the `Commands` enum in `cli/src/main.rs`.
2. Implement the schema introspection logic to auto-detect time, spatial, and value columns.
3. Implement the dynamic SQL generation and cross-database `ATTACH` execution.
4. Integrate the `indicatif` spinner and JSON output guard.