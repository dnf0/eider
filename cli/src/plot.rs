use color_eyre::eyre::{eyre, Result};
use duckdb::Connection;
use inquire::{MultiSelect, Select};

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum PlotType {
    Hist,
    Heatmap,
    Line,
}

fn detect_value_column(conn: &Connection, table: &str) -> Result<String> {
    let mut stmt = conn.prepare(&format!("DESCRIBE \"{}\"", table.replace("\"", "\"\"")))?;
    let mut rows = stmt.query([])?;

    let mut columns = Vec::new();
    while let Some(row) = rows.next()? {
        let col_name: String = row.get(0)?;
        let col_lower = col_name.to_lowercase();
        columns.push((col_name, col_lower));
    }

    let val_col = columns
        .iter()
        .find(|(_, lower)| {
            !lower.contains("time")
                && !lower.contains("date")
                && !lower.contains("lat")
                && lower != "y"
                && !lower.contains("lon")
                && lower != "x"
                && lower != "geom"
        })
        .map(|(name, _)| name.clone())
        .ok_or_else(|| eyre!("Could not automatically detect a value column"))?;

    Ok(val_col)
}

fn plot_hist(
    conn: &Connection,
    table: &str,
    val_col: &str,
    group_by: Option<&str>,
    pin: &[String],
) -> Result<()> {
    let group_select = if let Some(g) = group_by {
        format!("\"{}\",", g)
    } else {
        String::new()
    };

    let group_by_clause = if group_by.is_some() {
        "GROUP BY 1, 2"
    } else {
        "GROUP BY 1"
    };

    let where_clause = crate::duckdb_utils::format_pins_where(pin);

    let query = format!(
        "WITH stats AS (
             SELECT min(\"{v}\") as v_min, max(\"{v}\") as v_max FROM \"{t}\" {where_clause}
         ),
         bins AS (
             SELECT
                 {g}
                 COALESCE(floor((\"{v}\" - v_min) / NULLIF((v_max - v_min) / 10.0, 0)), 0) as bin_idx,
                 count(*) as freq
             FROM \"{t}\", stats
             {where_clause_with_and}
             {gb}
         )
         SELECT {g} bin_idx, freq FROM bins ORDER BY {g} bin_idx",
        v = val_col.replace("\"", "\"\""),
        t = table.replace("\"", "\"\""),
        g = group_select,
        gb = group_by_clause,
        where_clause = where_clause,
        where_clause_with_and = if where_clause.is_empty() { String::new() } else { where_clause.replace("WHERE", "WHERE ") } // wait, if it's "FROM table, stats", the JOIN condition is implicit cross join. We just need to add the where condition on table columns.
    );

    let mut stmt = conn.prepare(&query)?;
    let mut rows = stmt.query([])?;

    let mut max_freq = 0;
    let mut grouped_results: std::collections::BTreeMap<String, Vec<(i32, i64)>> =
        std::collections::BTreeMap::new();

    while let Some(row) = rows.next()? {
        let (group_name, bin_idx_opt, freq) = if group_by.is_some() {
            let g: Option<String> = match row.get(0) {
                Ok(val) => Some(val),
                Err(_) => {
                    // Try to fetch as some other type and convert, or just assume NULL if it fails.
                    // duckdb-rs might not auto-convert numeric groups to string if we just ask for String.
                    // But for now, let's keep it simple. If it's not a string, we can format it.
                    // Actually, duckdb-rs `row.get` with String usually does type coercion if possible.
                    let val: Option<String> = row.get(0).ok().flatten();
                    val
                }
            };
            let g_str = g.unwrap_or_else(|| "NULL".to_string());
            let b: Option<f64> = row.get(1)?;
            let f: i64 = row.get(2)?;
            (g_str, b, f)
        } else {
            let b: Option<f64> = row.get(0)?;
            let f: i64 = row.get(1)?;
            ("All".to_string(), b, f)
        };

        if let Some(b) = bin_idx_opt {
            max_freq = max_freq.max(freq);
            grouped_results
                .entry(group_name)
                .or_default()
                .push((b as i32, freq));
        }
    }

    let max_bars = 40;
    for (group_name, results) in grouped_results {
        if group_by.is_some() {
            println!("Group: {}", group_name);
        }
        for (bin, freq) in results {
            let bars = if max_freq > 0 {
                (freq as f64 / max_freq as f64 * max_bars as f64) as usize
            } else {
                0
            };
            let bar_str = "█".repeat(bars);
            println!("Bin {:2} │ {} ({})", bin, bar_str, freq);
        }
        if group_by.is_some() {
            println!();
        }
    }

    Ok(())
}

fn plot_line(
    conn: &Connection,
    table: &str,
    val_col: &str,
    group_by: Option<&str>,
    pin: &[String],
) -> Result<()> {
    if group_by.is_some() {
        println!("Warning: group-by is not yet supported for line plots in this MVP. Showing overall line.");
    }

    // We assume there's a time/date column. Let's find it.
    let mut stmt = conn.prepare(&format!("DESCRIBE \"{}\"", table.replace("\"", "\"\"")))?;
    let mut rows = stmt.query([])?;
    let mut time_col = String::from("time"); // Fallback
    while let Some(row) = rows.next()? {
        let col_name: String = row.get(0)?;
        let col_lower = col_name.to_lowercase();
        if col_lower.contains("time") || col_lower.contains("date") {
            time_col = col_name;
            break;
        }
    }

    let where_clause = crate::duckdb_utils::format_pins_where(pin);

    let query = format!(
        "SELECT \"{v}\" FROM \"{t}\" {where_clause} ORDER BY \"{time}\"",
        v = val_col.replace("\"", "\"\""),
        t = table.replace("\"", "\"\""),
        time = time_col.replace("\"", "\"\""),
        where_clause = where_clause
    );

    let mut stmt = conn.prepare(&query)?;
    let mut rows = stmt.query([])?;

    let mut data: Vec<f64> = Vec::new();
    while let Some(row) = rows.next()? {
        if let Ok(val) = row.get::<_, f64>(0) {
            data.push(val);
        }
    }

    if data.is_empty() {
        return Err(eyre!("No numeric data found for line plot."));
    }

    let max_points = 100;
    let plot_data: Vec<f64> = if data.len() > max_points {
        let chunk_size = (data.len() as f64 / max_points as f64).ceil() as usize;
        data.chunks(chunk_size)
            .map(|chunk| chunk.iter().sum::<f64>() / chunk.len() as f64)
            .collect()
    } else {
        data
    };

    let graph = rasciigraph::plot(
        plot_data,
        rasciigraph::Config::default()
            .with_height(15)
            .with_caption(format!("{} over {}", val_col, time_col)),
    );

    println!("\n{}\n", graph);

    Ok(())
}

fn detect_grid_bounds(
    conn: &Connection,
    table: &str,
    lat_col: &str,
    lon_col: &str,
    where_clause: &str,
) -> Result<(f64, f64, f64, f64)> {
    let bounds_query = format!(
        "SELECT min(\"{lat}\"), max(\"{lat}\"), min(\"{lon}\"), max(\"{lon}\") FROM \"{t}\" {where_clause}",
        lat = lat_col.replace("\"", "\"\""),
        lon = lon_col.replace("\"", "\"\""),
        t = table.replace("\"", "\"\""),
        where_clause = where_clause
    );
    let mut bounds_stmt = conn.prepare(&bounds_query)?;
    let mut bounds_rows = bounds_stmt.query([])?;
    let mut min_lat = 0.0;
    let mut max_lat = 0.0;
    let mut min_lon = 0.0;
    let mut max_lon = 0.0;
    if let Some(row) = bounds_rows.next()? {
        min_lat = row.get(0).unwrap_or(0.0);
        max_lat = row.get(1).unwrap_or(0.0);
        min_lon = row.get(2).unwrap_or(0.0);
        max_lon = row.get(3).unwrap_or(0.0);
    }
    Ok((min_lat, max_lat, min_lon, max_lon))
}

fn plot_heatmap(
    conn: &Connection,
    table: &str,
    val_col: &str,
    group_by: Option<&str>,
    pin: &[String],
) -> Result<()> {
    if group_by.is_some() {
        println!("Warning: group-by is ignored for spatial heatmaps.");
    }

    // Attempt to find lat/lon columns
    let mut stmt = conn.prepare(&format!("DESCRIBE \"{}\"", table.replace("\"", "\"\"")))?;
    let mut rows = stmt.query([])?;
    let mut lat_col = String::from("lat");
    let mut lon_col = String::from("lon");
    while let Some(row) = rows.next()? {
        let col_name: String = row.get(0)?;
        let col_lower = col_name.to_lowercase();
        if col_lower.contains("lat") || col_lower == "y" {
            lat_col = col_name.clone();
        }
        if col_lower.contains("lon") || col_lower == "x" {
            lon_col = col_name.clone();
        }
    }

    let where_clause = crate::duckdb_utils::format_pins_where(pin);

    let distinct_query = format!(
        "SELECT count(distinct \"{lat}\"), count(distinct \"{lon}\") FROM \"{t}\" {where_clause}",
        lat = lat_col.replace("\"", "\"\""),
        lon = lon_col.replace("\"", "\"\""),
        t = table.replace("\"", "\"\""),
        where_clause = where_clause
    );
    let mut distinct_stmt = conn.prepare(&distinct_query)?;
    let mut distinct_rows = distinct_stmt.query([])?;
    let mut actual_rows = 12;
    let mut actual_cols = 25;
    if let Some(row) = distinct_rows.next()? {
        let lat_count: i64 = row.get(0).unwrap_or(12);
        let lon_count: i64 = row.get(1).unwrap_or(25);
        actual_rows = lat_count as usize;
        actual_cols = lon_count as usize;
    }

    // Cap dimensions to prevent blowing up the terminal
    let rows_count = actual_rows.clamp(1, 40);
    let cols_count = actual_cols.clamp(1, 80);

    let (min_lat_bound, max_lat_bound, min_lon_bound, max_lon_bound) =
        detect_grid_bounds(conn, table, &lat_col, &lon_col, &where_clause)?;

    let query = format!(
        "WITH bounds AS (
            SELECT min(\"{lat}\") as min_lat, max(\"{lat}\") as max_lat,
                   min(\"{lon}\") as min_lon, max(\"{lon}\") as max_lon
            FROM \"{t}\" {where_clause}
        ),
        grid AS (
            SELECT
                COALESCE(floor((\"{lat}\" - min_lat) / NULLIF((max_lat - min_lat) / {rows_count}.0, 0)), 0) as row_idx,
                COALESCE(floor((\"{lon}\" - min_lon) / NULLIF((max_lon - min_lon) / {cols_count}.0, 0)), 0) as col_idx,
                avg(\"{v}\") as cell_val
            FROM \"{t}\", bounds
            {where_clause_with_and}
            GROUP BY 1, 2
        )
        SELECT row_idx, col_idx, cell_val FROM grid",
        lat = lat_col.replace("\"", "\"\""),
        lon = lon_col.replace("\"", "\"\""),
        v = val_col.replace("\"", "\"\""),
        t = table.replace("\"", "\"\""),
        where_clause = where_clause,
        where_clause_with_and = if where_clause.is_empty() { String::new() } else { where_clause.replace("WHERE", "WHERE ") }
    );

    let mut stmt = conn.prepare(&query)?;
    let mut rows = stmt.query([])?;

    let mut grid_data = vec![vec![f64::NAN; cols_count]; rows_count];
    let mut global_min = f64::MAX;
    let mut global_max = f64::MIN;

    while let Some(row) = rows.next()? {
        let r: Option<f64> = row.get(0)?;
        let c: Option<f64> = row.get(1)?;
        let v: Option<f64> = row.get(2)?;

        if let (Some(r), Some(c), Some(v)) = (r, c, v) {
            let r_idx = r.max(0.0).min((rows_count - 1) as f64) as usize;
            let c_idx = c.max(0.0).min((cols_count - 1) as f64) as usize;
            grid_data[r_idx][c_idx] = v;
            global_min = global_min.min(v);
            global_max = global_max.max(v);
        }
    }

    draw_heatmap(
        val_col,
        rows_count,
        cols_count,
        &grid_data,
        min_lat_bound,
        max_lat_bound,
        min_lon_bound,
        max_lon_bound,
        global_min,
        global_max,
    );

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_heatmap(
    val_col: &str,
    rows_count: usize,
    cols_count: usize,
    grid_data: &[Vec<f64>],
    min_lat_bound: f64,
    max_lat_bound: f64,
    min_lon_bound: f64,
    max_lon_bound: f64,
    global_min: f64,
    global_max: f64,
) {
    // ANSI Truecolor mapping from blue to red
    let get_color = |t: f64| -> String {
        let colors = [
            (0, 0, 255),   // Blue
            (0, 255, 255), // Cyan
            (0, 255, 0),   // Green
            (255, 255, 0), // Yellow
            (255, 0, 0),   // Red
        ];
        let t_val = t.clamp(0.0, 1.0) * (colors.len() - 1) as f64;
        let idx = t_val.floor() as usize;
        if idx >= colors.len() - 1 {
            let c = colors.last().unwrap();
            return format!("\x1b[38;2;{};{};{}m", c.0, c.1, c.2);
        }
        let frac = t_val - idx as f64;
        let c1 = colors[idx];
        let c2 = colors[idx + 1];
        let r = (c1.0 as f64 + (c2.0 as f64 - c1.0 as f64) * frac) as u8;
        let g = (c1.1 as f64 + (c2.1 as f64 - c1.1 as f64) * frac) as u8;
        let b = (c1.2 as f64 + (c2.2 as f64 - c1.2 as f64) * frac) as u8;
        format!("\x1b[38;2;{};{};{}m", r, g, b)
    };

    println!("\nHeatmap of {} (Spatial):\n", val_col);
    for r in (0..rows_count).rev() {
        // Print top-to-bottom
        if r == rows_count - 1 {
            print!("{:>8.2} ┤ ", max_lat_bound);
        } else if r == 0 {
            print!("{:>8.2} ┤ ", min_lat_bound);
        } else {
            print!("         │ ");
        }

        for val in grid_data[r].iter().take(cols_count) {
            let val = *val;
            if val.is_nan() {
                print!("  ");
            } else {
                let normalized = if global_max > global_min {
                    (val - global_min) / (global_max - global_min)
                } else {
                    0.5
                };
                let color_escape = get_color(normalized);
                print!("{}██\x1b[0m", color_escape);
            }
        }
        println!();
    }

    // Print X-axis
    let padding = " ".repeat(11);
    let plot_width = cols_count * 2;
    print!("{}└", " ".repeat(10));
    for _ in 0..cols_count {
        print!("──");
    }
    println!();

    let start_lon_str = format!("{:.2}", min_lon_bound);
    let end_lon_str = format!("{:.2}", max_lon_bound);
    let start_len = start_lon_str.chars().count();
    let end_len = end_lon_str.chars().count();

    if plot_width > start_len + end_len {
        let space_between = plot_width - start_len - end_len;
        println!(
            "{}{}{}{}",
            padding,
            start_lon_str,
            " ".repeat(space_between),
            end_lon_str
        );
    } else {
        println!("{}{}", padding, start_lon_str);
        println!(
            "{}{}",
            " ".repeat(11 + plot_width.saturating_sub(end_len)),
            end_lon_str
        );
    }

    println!("\nLegend:");
    if global_min <= global_max {
        let steps = 10;
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            let val = global_min + (global_max - global_min) * t;
            let color = get_color(t);
            print!("{}██\x1b[0m {:.2}   ", color, val);
            if i == 4 {
                println!();
            }
        }
        println!();
    }
}

pub struct PlotConfig {
    pub plot_type: PlotType,
    pub table: String,
    pub value_column: String,
    pub group_by: Option<String>,
    pub pin: Vec<String>,
}

pub fn render_plot(conn: &Connection, config: &PlotConfig) -> Result<()> {
    match config.plot_type {
        PlotType::Hist => plot_hist(
            conn,
            &config.table,
            &config.value_column,
            config.group_by.as_deref(),
            &config.pin,
        )?,
        PlotType::Heatmap => plot_heatmap(
            conn,
            &config.table,
            &config.value_column,
            config.group_by.as_deref(),
            &config.pin,
        )?,
        PlotType::Line => plot_line(
            conn,
            &config.table,
            &config.value_column,
            config.group_by.as_deref(),
            &config.pin,
        )?,
    }
    Ok(())
}

pub fn collect_wizard_config(conn: &Connection, default_table: &str) -> Result<Option<PlotConfig>> {
    println!("Launching Interactive Plot Wizard...\n");

    // 1. Select Table
    let mut stmt = conn.prepare("SHOW TABLES")?;
    let mut rows = stmt.query([])?;
    let mut tables = Vec::new();
    while let Some(row) = rows.next()? {
        let table_name: String = row.get(0)?;
        tables.push(table_name);
    }

    if tables.is_empty() {
        return Err(eyre!("No tables found in database."));
    }

    let default_idx = tables.iter().position(|t| t == default_table).unwrap_or(0);

    let selected_table = Select::new("Which table would you like to plot?", tables)
        .with_starting_cursor(default_idx)
        .prompt()?;

    // 2. Select Variables
    let mut stmt = conn.prepare(&format!(
        "DESCRIBE \"{}\"",
        selected_table.replace("\"", "\"\"")
    ))?;
    let mut rows = stmt.query([])?;
    let mut columns = Vec::new();
    while let Some(row) = rows.next()? {
        let col_name: String = row.get(0)?;
        let col_type: String = row.get(1)?;
        columns.push(format!("{} ({})", col_name, col_type));
    }

    let selected_vars = MultiSelect::new(
        "Select variables to analyze (Space to select, Enter to confirm):",
        columns,
    )
    .prompt()?;

    if selected_vars.is_empty() {
        println!("No variables selected. Exiting.");
        return Ok(None);
    }

    // Extract just the column names
    let var_names: Vec<String> = selected_vars
        .iter()
        .map(|v| v.split(' ').next().unwrap().to_string())
        .collect();

    // 3. Recommend Plot Type
    let num_vars = var_names.len();
    println!("\nDetected {} variable(s).", num_vars);

    let plot_options = match num_vars {
        1 => vec![
            "Heatmap (2D Spatial)",
            "Histogram (Distribution)",
            "Line Plot (Time Series)",
        ],
        _ => vec!["Line Plot (Time Series)", "Histogram (Distribution)"],
    };

    let selected_plot_str = Select::new("Choose plot type:", plot_options)
        .with_starting_cursor(0)
        .prompt()?;

    // Map selection to enum
    let plot_type = if selected_plot_str.contains("Histogram") {
        PlotType::Hist
    } else if selected_plot_str.contains("Heatmap") {
        PlotType::Heatmap
    } else if selected_plot_str.contains("Line") {
        PlotType::Line
    } else {
        println!(
            "Plot type '{}' is not fully implemented yet in the renderer. Exiting.",
            selected_plot_str
        );
        return Ok(None);
    };

    // Determine value column (pick the last selected variable as a heuristic)
    let val_col = var_names
        .last()
        .expect("var_names is guaranteed to be non-empty")
        .to_string();
    Ok(Some(PlotConfig {
        plot_type,
        table: selected_table,
        value_column: val_col,
        group_by: None,
        pin: vec![],
    }))
}

pub fn run_plot(
    db_path: &str,
    plot_type: Option<PlotType>,
    table: &str,
    value_column: Option<&str>,
    group_by: Option<&str>,
    pin: Vec<String>,
) -> Result<()> {
    if !std::path::Path::new(db_path).exists() {
        return Err(eyre!("Database '{}' does not exist.", db_path));
    }

    let conn = Connection::open(db_path)?;

    let config = if let Some(pt) = plot_type {
        let val_col = match value_column {
            Some(v) => v.to_string(),
            None => detect_value_column(&conn, table)?,
        };

        println!(
            "Plotting {} from table {} (Value: {})",
            format!("{:?}", pt).to_lowercase(),
            table,
            val_col
        );

        PlotConfig {
            plot_type: pt,
            table: table.to_string(),
            value_column: val_col,
            group_by: group_by.map(|s| s.to_string()),
            pin,
        }
    } else {
        match collect_wizard_config(&conn, table)? {
            Some(c) => {
                println!("\nExecuting generated command:");
                println!(
                    "eider plot <db> --plot-type {} --table {} --value {}\n",
                    format!("{:?}", c.plot_type).to_lowercase(),
                    c.table,
                    c.value_column
                );
                c
            }
            None => return Ok(()),
        }
    };

    render_plot(&conn, &config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use duckdb::Connection;

    #[test]
    fn test_plot_hist_no_group_by() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE test_data (val DOUBLE)", [])
            .unwrap();
        conn.execute(
            "INSERT INTO test_data VALUES (1.0), (2.0), (3.0), (1.5)",
            [],
        )
        .unwrap();

        let result = plot_hist(&conn, "test_data", "val", None, &[]);
        assert!(result.is_ok(), "plot_hist failed: {:?}", result.err());
    }

    #[test]
    fn test_plot_hist_with_group_by() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE test_group (val DOUBLE, category VARCHAR)", [])
            .unwrap();
        conn.execute(
            "INSERT INTO test_group VALUES (1.0, 'A'), (2.0, 'A'), (3.0, 'B'), (1.5, 'B')",
            [],
        )
        .unwrap();

        let result = plot_hist(&conn, "test_group", "val", Some("category"), &[]);
        assert!(
            result.is_ok(),
            "plot_hist with group_by failed: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_plot_hist_div_by_zero() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE test_zero (val DOUBLE)", [])
            .unwrap();
        conn.execute("INSERT INTO test_zero VALUES (2.0), (2.0), (2.0)", [])
            .unwrap();

        let result = plot_hist(&conn, "test_zero", "val", None, &[]);
        assert!(
            result.is_ok(),
            "plot_hist div by zero failed: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_plot_line_basic() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE test_data (time TIMESTAMP, val DOUBLE)", [])
            .unwrap();
        conn.execute("INSERT INTO test_data VALUES ('2023-01-01', 1.0), ('2023-01-02', 2.0), ('2023-01-03', 3.0)", []).unwrap();

        let result = plot_line(&conn, "test_data", "val", None, &[]);
        assert!(result.is_ok(), "plot_line failed: {:?}", result.err());
    }

    #[test]
    fn test_plot_line_no_data() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE test_empty (time TIMESTAMP, val DOUBLE)", [])
            .unwrap();

        let result = plot_line(&conn, "test_empty", "val", None, &[]);
        assert!(result.is_err(), "plot_line should fail on empty data");
    }

    #[test]
    fn test_plot_heatmap_div_by_zero() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE test_hm_zero (lat DOUBLE, lon DOUBLE, val DOUBLE)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO test_hm_zero VALUES (2.0, 3.0, 1.0), (2.0, 3.0, 2.0)",
            [],
        )
        .unwrap();

        let result = plot_heatmap(&conn, "test_hm_zero", "val", None, &[]);
        assert!(
            result.is_ok(),
            "plot_heatmap div by zero failed: {:?}",
            result.err()
        );
    }

    fn mem(ddl: &str) -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(ddl).unwrap();
        c
    }

    #[test]
    fn detects_non_coordinate_value_column() {
        let c =
            mem("CREATE TABLE t (time BIGINT, lat DOUBLE, lon DOUBLE, air_temperature DOUBLE);");
        assert_eq!(detect_value_column(&c, "t").unwrap(), "air_temperature");
    }

    #[test]
    fn errors_when_only_coordinates() {
        let c = mem("CREATE TABLE t (time BIGINT, lat DOUBLE, lon DOUBLE);");
        assert!(detect_value_column(&c, "t").is_err());
    }
}
