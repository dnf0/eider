# Zarrduck Temporal Analytics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `resample` command to perform automated temporal aggregations (e.g., daily to monthly means) on extracted `.duckdb` spatial databases.

**Architecture:** We will extend the `Commands` enum with a `Resample` variant. The logic will introspect the input DuckDB schema to identify time, spatial, and value columns using a heuristic approach. It will then dynamically generate an `ATTACH` and `CREATE TABLE` query leveraging DuckDB's `date_trunc` and aggregate functions to perform the resampling inside the target database, wrapping the execution in an `indicatif` spinner.

**Tech Stack:** Rust, `clap`, `duckdb`, `indicatif`, `color-eyre`

---

### Task 1: Command Structure and Placeholder

**Files:**
- Modify: `cli/src/main.rs`

- [ ] **Step 1: Add `Resample` to `Commands` enum**

Update the `Commands` enum in `cli/src/main.rs` to add the `Resample` variant:
```rust
    /// Temporally resample extracted GeoZarr data
    Resample {
        /// The input DuckDB file containing the 'extracted_data' table
        input_db: String,
        
        /// The output DuckDB file to save the resampled data
        output_db: String,
        
        /// The temporal frequency (e.g., month, year, day)
        #[arg(long)]
        freq: String,
        
        /// The aggregate function to apply (e.g., avg, sum, max)
        #[arg(long)]
        agg: String,
    },
```

- [ ] **Step 2: Add placeholder logic in `run_cli`**

In the `run_cli` match block, add the placeholder arm:
```rust
        Commands::Resample { input_db, output_db, freq, agg } => {
            println!("Resampling {} to {} with freq {} and agg {}", input_db, output_db, freq, agg);
        }
```

- [ ] **Step 3: Run check to verify it compiles**

Run: `cargo check -p zarrduck`
Expected: SUCCESS

- [ ] **Step 4: Commit**

```bash
git add cli/src/main.rs
git commit -m "feat: add Resample subcommand to zarrduck cli"
```

---

### Task 2: Schema Introspection Logic

**Files:**
- Modify: `cli/src/main.rs`

- [ ] **Step 1: Write column detection helper**

Add a helper function to find columns based on known name patterns:
```rust
fn detect_columns(conn: &duckdb::Connection, table: &str) -> EyreResult<(String, String, String, String)> {
    let mut stmt = conn.prepare(&format!("DESCRIBE {}", table))
        .wrap_err_with(|| format!("Failed to describe table '{}'", table))?;
    
    let mut rows = stmt.query([])?;
    
    let mut columns = Vec::new();
    while let Some(row) = rows.next()? {
        let col_name: String = row.get(0)?;
        columns.push(col_name.to_lowercase());
    }

    // Heuristics
    let time_col = columns.iter().find(|c| c.contains("time") || c.contains("date"))
        .cloned().ok_or_else(|| eyre!("Could not automatically detect a time column"))?;
        
    let lat_col = columns.iter().find(|c| c.contains("lat") || c.contains("y"))
        .cloned().ok_or_else(|| eyre!("Could not automatically detect a latitude column"))?;
        
    let lon_col = columns.iter().find(|c| c.contains("lon") || c.contains("x"))
        .cloned().ok_or_else(|| eyre!("Could not automatically detect a longitude column"))?;

    let val_col = columns.iter().find(|&c| c != &time_col && c != &lat_col && c != &lon_col && c != "geom")
        .cloned().ok_or_else(|| eyre!("Could not automatically detect a value column"))?;

    Ok((time_col, lat_col, lon_col, val_col))
}
```

- [ ] **Step 2: Update `Resample` match arm**

In `Commands::Resample`:
```rust
        Commands::Resample { input_db, output_db, freq, agg } => {
            if !std::path::Path::new(&input_db).exists() {
                return Err(eyre!("Input database '{}' does not exist.", input_db));
            }

            let input_conn = Connection::open(&input_db)
                .wrap_err_with(|| format!("Failed to open input database '{}'", input_db))?;
            
            let (time_col, lat_col, lon_col, val_col) = detect_columns(&input_conn, "extracted_data")?;
            
            if resolved_output != OutputFormat::Json {
                println!("Detected schema: Time='{}', Spatial='{}', '{}', Value='{}'", time_col, lat_col, lon_col, val_col);
            }
            
            // Just close the input connection so we don't lock the file for the next step
            drop(input_conn);
        }
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p zarrduck`
Expected: SUCCESS

- [ ] **Step 4: Commit**

```bash
git add cli/src/main.rs
git commit -m "feat: implement automatic schema introspection for temporal resampling"
```

---

### Task 3: SQL Execution and TUI Integration

**Files:**
- Modify: `cli/src/main.rs`

- [ ] **Step 1: Write execution logic**

Append the following to the `Commands::Resample` match arm after `drop(input_conn);`:

```rust
            // Overwrite protection for output db
            if std::path::Path::new(&output_db).exists() {
                if resolved_output == OutputFormat::Json {
                    return Err(eyre!("Output database '{}' already exists. Aborting.", output_db));
                } else {
                    let ans = inquire::Confirm::new(&format!("File '{}' already exists. Overwrite?", output_db))
                        .with_default(false)
                        .prompt()
                        .wrap_err("Failed to read user input")?;
                        
                    if !ans {
                        println!("Aborting resampling.");
                        return Ok(());
                    }
                    std::fs::remove_file(&output_db).wrap_err_with(|| format!("Failed to delete '{}'", output_db))?;
                }
            }

            let conn = Connection::open(&output_db)
                .wrap_err_with(|| format!("Failed to open output database '{}'", output_db))?;
            
            let spinner = if resolved_output != OutputFormat::Json {
                let pb = indicatif::ProgressBar::new_spinner();
                pb.set_style(
                    indicatif::ProgressStyle::default_spinner()
                        .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ")
                        .template("{spinner:.green} {msg}")
                        .unwrap()
                );
                pb.set_message("Resampling time-series data...");
                pb.enable_steady_tick(std::time::Duration::from_millis(100));
                Some(pb)
            } else {
                None
            };
            
            conn.execute(&format!("ATTACH '{}' AS source_db", input_db), [])
                .wrap_err("Failed to attach input database")?;
                
            let query = format!(
                "CREATE TABLE resampled_data AS 
                 SELECT 
                     date_trunc('{}', {}) as {},
                     {}, {},
                     {}({}) as value
                 FROM source_db.extracted_data
                 GROUP BY 1, 2, 3",
                freq.replace("'", "''"), time_col, time_col,
                lat_col, lon_col,
                agg, val_col
            );
            
            conn.execute(&query, []).wrap_err("Resampling query failed")?;
            
            if let Some(pb) = spinner {
                pb.finish_with_message("Resampling complete!");
            }
            
            if resolved_output == OutputFormat::Json {
                println!(r#"{{"status": "success", "db": "{}"}}"#, output_db);
            } else {
                println!("Data saved to table 'resampled_data' in {}", output_db);
                println!("Run `zarrduck shell {}` to explore it.", output_db);
            }
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p zarrduck`
Expected: SUCCESS

- [ ] **Step 3: Commit**

```bash
git add cli/src/main.rs
git commit -m "feat: implement temporal resampling SQL generation and execution"
```

---

### Task 4: Integration Test

**Files:**
- Modify: `cli/tests/integration_test.rs`

- [ ] **Step 1: Write integration test**

Add a simple test that ensures the command fails if the input file does not exist:

```rust
#[test]
fn test_cli_resample_missing_input() {
    let mut cmd = Command::cargo_bin("zarrduck").unwrap();
    cmd.arg("resample")
        .arg("missing_input.duckdb")
        .arg("out.duckdb")
        .arg("--freq")
        .arg("month")
        .arg("--agg")
        .arg("avg")
        .arg("--output=json")
        .assert()
        .failure()
        .stdout(predicate::str::contains("Input database 'missing_input.duckdb' does not exist"));
}
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test -p zarrduck --test integration_test test_cli_resample_missing_input`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add cli/tests/integration_test.rs
git commit -m "test: add integration test for missing input db in resample command"
```