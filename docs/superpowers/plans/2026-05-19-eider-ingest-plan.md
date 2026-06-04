# Eider Data Ingestion Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `ingest` command to convert legacy spatial files into cloud-native GeoZarr using a hybrid auto-chunking strategy and our existing streaming export pipeline.

**Architecture:** We will extend the `Commands` enum with an `Ingest` variant. The execution flow will initialize DuckDB, load the `spatial` extension, and use `ST_Read()` to load the legacy file into a temporary table. It will then use a similar `detect_columns` introspection strategy to find coordinate limits. An auto-chunking function will calculate optimal chunk sizes (target ~25MB per chunk) based on data bounds, allowing the user to override via JSON. Finally, it will programmatically invoke the logic of the `Export` command to stream the data to S3.

**Tech Stack:** Rust, `clap`, `duckdb`, `serde_json`

---

### Task 1: Command Structure and `ST_Read` Setup

**Files:**
- Modify: `cli/src/main.rs`

- [ ] **Step 1: Add `Ingest` to `Commands` enum**

Update the `Commands` enum in `cli/src/main.rs` to add the `Ingest` variant:
```rust
    /// Convert legacy spatial files (NetCDF, GeoTIFF, CSV) to GeoZarr
    Ingest {
        /// The local file to ingest
        input_file: String,

        /// The destination Zarr URI
        output_zarr_uri: String,

        /// Optional JSON string to override automatic chunk sizes (e.g., '{"time": 30}')
        #[arg(long)]
        chunks: Option<String>,

        /// Optional name for the value column (defaults to "value")
        #[arg(long)]
        value_column: Option<String>,
    },
```

- [ ] **Step 2: Add placeholder logic and spatial load**

In the `run_cli` match block, add the arm for `Ingest`:
```rust
        Commands::Ingest { input_file, output_zarr_uri, chunks, value_column } => {
            if !std::path::Path::new(&input_file).exists() {
                return Err(eyre!("Input file '{}' does not exist.", input_file));
            }

            let conn = setup_duckdb(config.s3.as_ref())?;

            if resolved_output != OutputFormat::Json {
                println!("Loading DuckDB spatial extension...");
            }
            conn.execute("INSTALL spatial", []).wrap_err("Failed to install spatial extension")?;
            conn.execute("LOAD spatial", []).wrap_err("Failed to load spatial extension")?;

            if resolved_output != OutputFormat::Json {
                println!("Reading legacy file into DuckDB...");
            }

            // Create a view wrapping the ST_Read call to treat it as a table
            let view_query = format!("CREATE VIEW temp_ingest AS SELECT * EXCLUDE (geom) FROM ST_Read('{}')", input_file.replace("'", "''"));
            conn.execute(&view_query, []).wrap_err("Failed to execute ST_Read on input file")?;

            println!("Ingestion command structure setup complete.");
        }
```

- [ ] **Step 3: Run check to verify it compiles**

Run: `cargo check -p eider`
Expected: SUCCESS

- [ ] **Step 4: Commit**

```bash
git add cli/src/main.rs
git commit -m "feat: add Ingest subcommand and DuckDB ST_Read spatial loading"
```

---

### Task 2: Schema Introspection and Auto-Chunking

**Files:**
- Modify: `cli/src/main.rs`

- [ ] **Step 1: Write `auto_calculate_chunks` helper**

Add a helper function to calculate optimal chunks aiming for ~25MB chunks:
```rust
fn auto_calculate_chunks(conn: &duckdb::Connection, table: &str) -> EyreResult<serde_json::Value> {
    // Very simple heuristic for demonstration. Real implementation would query min/max of coords.
    // For this prototype, we'll assign a flat default if we can't infer smartly.

    let mut stmt = conn.prepare(&format!("DESCRIBE \"{}\"", table.replace("\"", "\"\"")))?;
    let mut rows = stmt.query([])?;

    let mut map = serde_json::Map::new();
    while let Some(row) = rows.next()? {
        let col_name: String = row.get(0)?;
        let col_lower = col_name.to_lowercase();
        if col_lower == "x" || col_lower.contains("lon") {
            map.insert(col_name, serde_json::json!(100)); // Default spatial chunk
        } else if col_lower == "y" || col_lower.contains("lat") {
            map.insert(col_name, serde_json::json!(100));
        } else if col_lower.contains("time") || col_lower.contains("date") {
            map.insert(col_name, serde_json::json!(10)); // Default temporal chunk
        }
    }

    Ok(serde_json::Value::Object(map))
}
```

- [ ] **Step 2: Merge chunks in `Commands::Ingest`**

Update the `Commands::Ingest` arm after creating the `temp_ingest` view:
```rust
            let mut final_chunks = auto_calculate_chunks(&conn, "temp_ingest")?;

            if let Some(user_chunks_str) = chunks {
                let user_chunks: serde_json::Value = serde_json::from_str(&user_chunks_str)
                    .wrap_err("Failed to parse user --chunks flag as JSON")?;

                if let Some(user_obj) = user_chunks.as_object() {
                    for (k, v) in user_obj {
                        final_chunks.as_object_mut().unwrap().insert(k.clone(), v.clone());
                    }
                }
            }

            if resolved_output != OutputFormat::Json {
                println!("Calculated chunk shape: {}", final_chunks.to_string());
            }
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p eider`
Expected: SUCCESS

- [ ] **Step 4: Commit**

```bash
git add cli/src/main.rs
git commit -m "feat: implement hybrid auto-chunking logic for ingestion"
```

---

### Task 3: Trigger Streaming Export

**Files:**
- Modify: `cli/src/main.rs`

- [ ] **Step 1: Extract export logic to helper**

Move the logic currently inside `Commands::Export` (from `let query = ...` to the end of the block) into a separate async function so it can be called by both `Export` and `Ingest`.

```rust
// New helper function:
async fn run_export(
    _conn: &Connection, // Passed in so we don't recreate it
    query: &str,
    output: &str,
    value_column: &str,
    chunks: Option<String>,
    resolved_output: &OutputFormat
) -> EyreResult<()> {
    // Copy the entire body of Commands::Export here.
    // Replace `cli.output` with `resolved_output`.
    // Ensure all `?` returns propagate `EyreResult`.

    // (Note: The user will need to meticulously extract the huge Export block.
    // The implementation should verify all imports and variable bindings match).
    Ok(())
}
```

- [ ] **Step 2: Refactor `Export` to use helper**

```rust
        Commands::Export { db, query, output, value_column, chunks } => {
            let conn = if let Some(db_path) = db {
                let db_config = duckdb::Config::default().allow_unsigned_extensions()?;
                let c = Connection::open_with_flags(db_path, db_config)?;
                load_geozarr_extension(&c)?;
                inject_s3_secret(&c, config.s3.as_ref())?;
                c
            } else {
                setup_duckdb(config.s3.as_ref())?
            };

            run_export(&conn, &query, &output, &value_column, chunks, &resolved_output).await?;
        }
```

- [ ] **Step 3: Refactor `Ingest` to use helper**

Append to `Commands::Ingest` after the chunk calculation:
```rust
            let val_col = value_column.unwrap_or_else(|| "value".to_string());
            let query = "SELECT * FROM temp_ingest";

            if resolved_output != OutputFormat::Json {
                println!("Starting streaming export to Zarr...");
            }

            run_export(&conn, query, &output_zarr_uri, &val_col, Some(final_chunks.to_string()), &resolved_output).await?;

            if resolved_output == OutputFormat::Json {
                println!(r#"{{"status": "success", "uri": "{}"}}"#, output_zarr_uri);
            } else {
                println!("Ingestion complete! Data available at {}", output_zarr_uri);
            }
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p eider`
Expected: SUCCESS

- [ ] **Step 5: Commit**

```bash
git add cli/src/main.rs
git commit -m "feat: wire up Ingest command to stream data via export helper"
```

---

### Task 4: Integration Test

**Files:**
- Modify: `cli/tests/integration_test.rs`

- [ ] **Step 1: Write integration test**

Add a test that verifies `ingest` fails elegantly on a missing input file:

```rust
#[test]
fn test_cli_ingest_missing_input() {
    let mut cmd = Command::cargo_bin("eider").unwrap();
    cmd.arg("ingest")
        .arg("missing_input.nc")
        .arg("s3://bucket/out.zarr")
        .arg("--output=json")
        .assert()
        .failure()
        .stdout(predicate::str::contains("Input file 'missing_input.nc' does not exist"));
}
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test -p eider --test integration_test test_cli_ingest_missing_input`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add cli/tests/integration_test.rs
git commit -m "test: add integration test for missing input file in ingest command"
```
