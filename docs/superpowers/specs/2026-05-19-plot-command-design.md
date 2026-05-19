# Design Specification: CLI Visualization Tool

## Overview
A new `plot` subcommand for the `zarrduck` CLI that generates inline terminal visualizations from local DuckDB files. This allows users to quickly inspect extracted or resampled data without leaving the terminal or loading external visualizers.

## User Experience
The user invokes the plot command specifying the type of visualization. All mathematical aggregation is pushed down to DuckDB, keeping the CLI responsive and lightweight.

### CLI Interface
```bash
zarrduck plot <db_path> \
  --type <hist|heatmap|line> \
  --table <table_name> \     # Optional: defaults to 'extracted_data'
  --value <col_name> \       # Optional: auto-detects if omitted
  --group-by <col_name>      # Optional: groups output (e.g., per polygon)
```

## Supported Visualizations

### 1. Histogram (`--type hist`)
Shows the distribution of values, optionally grouped by a category (like a polygon).
*   **SQL Strategy:** DuckDB calculates `min(value)` and `max(value)`, splits the range into ~10 bins, and performs a `GROUP BY` to count frequencies per bin (and per `--group-by` category if requested).
*   **Rust Rendering:** A custom, lightweight formatter reads the bin counts and prints horizontal ASCII bar charts (using `█`) scaled to the maximum count.

### 2. Heatmap (`--type heatmap`)
Shows a 2D spatial plot using ANSI-colored block characters.
*   **SQL Strategy:** DuckDB calculates the spatial bounding box, divides it into a terminal-friendly grid (e.g., 20x40), and calculates `AVG(value)` for each cell using `GROUP BY floor(...)`.
*   **Rust Rendering:** A custom engine determines the global min/max for the color scale, then iterates through the grid results, mapping the average value to an ANSI true-color or 256-color code, and printing it with a color bar legend.

### 3. Line Plot (`--type line`)
Shows a 1D time-series plot.
*   **SQL Strategy:** DuckDB aggregates the data over the 1D dimension (usually `time`) and orders it chronologically.
*   **Rust Rendering:** The 1D array of values is passed to the lightweight `rasciigraph` crate, which handles terminal scaling, axis generation, and drawing the line graph using standard characters.

## Architecture Guidelines
*   **Separation of Concerns:** Rust handles purely formatting and API calling; all mathematical bucketing, scaling, and aggregation MUST be handled by DuckDB SQL queries.
*   **Consistency:** All plots must render within the terminal constraints and maintain a consistent aesthetic.