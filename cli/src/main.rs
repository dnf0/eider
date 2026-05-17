use clap::{Parser, Subcommand};
use duckdb::{Connection, Result};

#[derive(Parser)]
#[command(name = "geozarr-cli")]
#[command(about = "Companion CLI tool for exporting DuckDB tables to Zarr", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Export the results of a SQL query to a Zarr array
    Export {
        /// Path to the DuckDB database file (or leave empty for in-memory)
        #[arg(long)]
        db: Option<String>,

        /// The SQL query to execute
        #[arg(long)]
        query: String,

        /// The destination path for the Zarr array (e.g., s3://bucket/output.zarr)
        #[arg(long)]
        output: String,

        /// The column containing the actual values (all others are coordinates)
        #[arg(long)]
        value_column: String,

        /// Optional JSON mapping of dimension name to chunk size (e.g. '{"time": 10}')
        #[arg(long)]
        chunks: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Export {
            db,
            query,
            output,
            value_column,
            chunks,
        } => {
            println!("Exporting to Zarr...");
            println!("Database: {:?}", db);
            println!("Query: {}", query);
            println!("Output: {}", output);
            println!("Value Column: {}", value_column);
            if let Some(c) = &chunks {
                println!("Chunks: {}", c);
            }

            let _conn = match db {
                Some(path) => Connection::open(path)?,
                None => Connection::open_in_memory()?,
            };

            // 1. Get the columns from the query
            let query_info = format!("DESCRIBE {}", query);
            let mut info_stmt = _conn.prepare(&query_info)?;
            let mut rows = info_stmt.query([])?;

            let mut all_columns = Vec::new();
            let mut coord_columns = Vec::new();

            while let Some(row) = rows.next()? {
                let col_name: String = row.get(0)?;
                all_columns.push(col_name.clone());
                if col_name != value_column {
                    coord_columns.push(col_name);
                }
            }

            if !all_columns.contains(&value_column) {
                return Err(
                    format!("Value column '{}' not found in query results", value_column).into(),
                );
            }

            // 2. Pass 1: Infer Shape
            println!("Pass 1: Inferring shape...");
            let mut shape = Vec::new();

            if !coord_columns.is_empty() {
                let mut agg_selects = Vec::new();
                for coord in &coord_columns {
                    agg_selects.push(format!(
                        "COUNT(DISTINCT \"{}\")",
                        coord.replace("\"", "\"\"")
                    ));
                }

                let inference_query = format!(
                    "SELECT {} FROM ({}) AS _geozarr_subq",
                    agg_selects.join(", "),
                    query
                );
                let mut inf_stmt = _conn.prepare(&inference_query)?;

                inf_stmt.query_row([], |row| {
                    for i in 0..coord_columns.len() {
                        let count: u64 = row.get(i)?;
                        shape.push(count);
                    }
                    Ok(())
                })?;
            }

            println!("Inferred Shape: {:?}", shape);

            // 3. Initialize Zarr Store
            let store = if output.starts_with("s3://") {
                let bucket_and_path = output.strip_prefix("s3://").unwrap();
                let bucket = bucket_and_path.split('/').next().unwrap_or(bucket_and_path);
                let root = bucket_and_path.strip_prefix(bucket).unwrap_or("/");
                let builder = opendal::services::S3::default().bucket(bucket).root(root);
                let operator = opendal::Operator::new(builder)?.finish();
                std::sync::Arc::new(zarrs::storage::store::AsyncOpendalStore::new(operator))
                    as std::sync::Arc<dyn zarrs::storage::AsyncWritableStorageTraits>
            } else {
                let builder = opendal::services::Fs::default().root(&output);
                let operator = opendal::Operator::new(builder)?.finish();
                std::sync::Arc::new(zarrs::storage::store::AsyncOpendalStore::new(operator))
                    as std::sync::Arc<dyn zarrs::storage::AsyncWritableStorageTraits>
            };

            // Write metadata (assuming Float32 for simplicity in this MVP)
            let chunk_shape = vec![100; shape.len()];
            if let Some(_c) = chunks {
                // Simplified chunk parsing fallback
                println!("Chunk parsing not fully implemented, using defaults [100, ...]");
            }

            let array_builder = zarrs::array::ArrayBuilder::new(
                shape.clone(),
                zarrs::array::DataType::Float32,
                chunk_shape.clone().try_into().unwrap(),
                zarrs::array::FillValue::from(f32::NAN),
            );

            let array = array_builder.build(store.clone(), "/").unwrap();
            array.async_store_metadata().await?;
            println!("Initialized Zarr Array.");

            let array = std::sync::Arc::new(array);

            // 4. Setup Async Upload Workers
            println!("Pass 2: Streaming data...");
            let (tx, mut rx) = tokio::sync::mpsc::channel::<(Vec<u64>, Vec<f32>)>(16);
            let array_clone = array.clone();

            let upload_task = tokio::spawn(async move {
                while let Some((chunk_grid, chunk_data)) = rx.recv().await {
                    array_clone
                        .async_store_chunk_elements(&chunk_grid, &chunk_data)
                        .await
                        .expect("Failed to upload chunk");
                }
            });

            let mut active_chunks: std::collections::HashMap<Vec<u64>, Vec<f32>> =
                std::collections::HashMap::new();
            let chunk_len = chunk_shape.iter().product::<u64>() as usize;

            // 5. Stream data from DuckDB
            let order_by = coord_columns
                .iter()
                .map(|c| format!("\"{}\"", c))
                .collect::<Vec<_>>()
                .join(", ");
            let coords_str = coord_columns
                .iter()
                .map(|c| format!("\"{}\"", c))
                .collect::<Vec<_>>()
                .join(", ");
            let stream_query = format!(
                "SELECT {}, \"{}\" FROM ({}) ORDER BY {}",
                coords_str, value_column, query, order_by
            );
            let mut stream_stmt = _conn.prepare(&stream_query)?;

            let mut rows = stream_stmt.query([])?;
            let mut row_count = 0;

            while let Some(row) = rows.next()? {
                // Map the flat row to a chunk grid coordinate
                let mut grid_coord = Vec::new();
                for i in 0..coord_columns.len() {
                    let val: i64 = row.get(i)?;
                    if val < 0 {
                        return Err("Coordinates must be positive 0-based integer indices".into());
                    }
                    let grid_idx = (val as u64) / chunk_shape[i];
                    grid_coord.push(grid_idx);
                }

                // Get the value (assuming f32 for this MVP)
                let value: f32 = row.get(coord_columns.len())?;

                if !active_chunks.contains_key(&grid_coord) {
                    active_chunks.insert(grid_coord.clone(), Vec::with_capacity(chunk_len));
                }
                let buffer = active_chunks.get_mut(&grid_coord).unwrap();
                buffer.push(value);

                // Flush if full
                if buffer.len() == chunk_len {
                    let full_chunk = active_chunks.remove(&grid_coord).unwrap();
                    tx.send((grid_coord, full_chunk))
                        .await
                        .map_err(|_| "Upload worker failed or disconnected")?;
                }

                row_count += 1;
                if row_count % 100_000 == 0 {
                    println!("Streamed {} rows...", row_count);
                }
            }

            // 6. Flush remaining edge chunks
            for (grid_coord, mut buffer) in active_chunks.into_iter() {
                // Pad with fill_value
                while buffer.len() < chunk_len {
                    buffer.push(f32::NAN);
                }
                tx.send((grid_coord, buffer))
                    .await
                    .map_err(|_| "Upload worker failed or disconnected")?;
            }

            // 7. Drop sender and wait for uploads to finish
            drop(tx);
            upload_task
                .await
                .map_err(|e| format!("Upload task panicked: {}", e))?;

            println!("Finished streaming {} rows.", row_count);

            println!("Export successful!");
        }
    }

    Ok(())
}
