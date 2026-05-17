use clap::{Parser, Subcommand};
use duckdb::{Connection, Result};

#[derive(Parser)]
#[command(name = "geozarr-cli")]
#[command(about = "Companion CLI tool for exporting DuckDB tables to Zarr", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

enum ChunkData {
    F32(Vec<f32>),
    F64(Vec<f64>),
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
            let mut chunk_shape = Vec::new();
            let mut current_volume = 1u64;
            for &dim in &shape {
                let chunk_dim = if current_volume.saturating_mul(dim) <= 10_000_000 {
                    dim
                } else {
                    std::cmp::max(1, 10_000_000 / current_volume)
                };
                chunk_shape.push(chunk_dim);
                current_volume = current_volume.saturating_mul(chunk_dim);
            }
            if let Some(_c) = chunks {
                // Simplified chunk parsing fallback
                println!(
                    "Chunk parsing not fully implemented, using auto-chunking: {:?}",
                    chunk_shape
                );
            }

            if chunk_shape.contains(&0) {
                return Err("Chunk dimension size cannot be 0".into());
            }

            // Infer type from DuckDB schema
            let mut type_stmt = _conn.prepare(&query_info)?;
            let mut t_rows = type_stmt.query([])?;
            let mut value_type_str = "FLOAT".to_string();
            while let Some(row) = t_rows.next()? {
                let col_name: String = row.get(0)?;
                if col_name == value_column {
                    value_type_str = row.get(1)?;
                }
            }

            let data_type = if value_type_str == "DOUBLE" {
                zarrs::array::DataType::Float64
            } else {
                zarrs::array::DataType::Float32
            };

            let fill_value = if value_type_str == "DOUBLE" {
                zarrs::array::FillValue::from(f64::NAN)
            } else {
                zarrs::array::FillValue::from(f32::NAN)
            };

            let array_builder = zarrs::array::ArrayBuilder::new(
                shape.clone(),
                data_type.clone(),
                chunk_shape.clone().try_into().unwrap(),
                fill_value,
            );

            let array = array_builder.build(store.clone(), "/").unwrap();
            array.async_store_metadata().await?;
            println!("Initialized Zarr Array.");

            let array = std::sync::Arc::new(array);

            // 4. Setup Async Upload Workers
            println!("Pass 2: Streaming data...");
            let (tx, mut rx) = tokio::sync::mpsc::channel::<(Vec<u64>, ChunkData)>(16);
            let array_clone = array.clone();

            let upload_task = tokio::spawn(async move {
                while let Some((chunk_grid, chunk_data)) = rx.recv().await {
                    let res = match chunk_data {
                        ChunkData::F32(data) => {
                            array_clone
                                .async_store_chunk_elements(&chunk_grid, &data)
                                .await
                        }
                        ChunkData::F64(data) => {
                            array_clone
                                .async_store_chunk_elements(&chunk_grid, &data)
                                .await
                        }
                    };
                    if let Err(e) = res {
                        eprintln!("Failed to upload chunk: {}", e);
                        std::process::exit(1);
                    }
                }
            });

            let mut active_chunks: std::collections::BTreeMap<Vec<u64>, ChunkData> =
                std::collections::BTreeMap::new();
            let chunk_len = chunk_shape
                .iter()
                .try_fold(1u64, |acc, &x| acc.checked_mul(x))
                .ok_or("Chunk volume overflow")? as usize;

            let bytes_per_element = if data_type == zarrs::array::DataType::Float64 {
                8
            } else {
                4
            };
            let chunk_byte_size = chunk_len
                .checked_mul(bytes_per_element)
                .ok_or("Chunk byte size overflow")?;
            let max_memory_bytes = 512 * 1024 * 1024; // 512 MB

            // 5. Stream data from DuckDB
            let order_by = coord_columns
                .iter()
                .map(|c| format!("\"{}\"", c.replace("\"", "\"\"")))
                .collect::<Vec<_>>()
                .join(", ");
            let coords_str = coord_columns
                .iter()
                .map(|c| format!("\"{}\"", c.replace("\"", "\"\"")))
                .collect::<Vec<_>>()
                .join(", ");
            let stream_query = format!(
                "SELECT {}, \"{}\" FROM ({}) ORDER BY {}",
                coords_str,
                value_column.replace("\"", "\"\""),
                query,
                order_by
            );
            let mut stream_stmt = _conn.prepare(&stream_query)?;

            let mut rows = stream_stmt.query([])?;
            let mut row_count = 0;

            while let Some(row) = rows.next()? {
                // Map the flat row to a chunk grid coordinate
                let mut grid_coord = Vec::new();
                let mut local_coords = Vec::new();
                for (i, &chunk_dim) in chunk_shape.iter().enumerate().take(coord_columns.len()) {
                    let val: i64 = row.get(i)?;
                    if val < 0 {
                        return Err("Coordinates must be positive 0-based integer indices".into());
                    }
                    if (val as u64) >= shape[i] {
                        return Err(format!(
                            "Coordinate index {} exceeds maximum bound of dimension {}",
                            val, shape[i]
                        )
                        .into());
                    }
                    let grid_idx = (val as u64) / chunk_dim;
                    let local_c = (val as u64) % chunk_dim;
                    grid_coord.push(grid_idx);
                    local_coords.push(local_c);
                }

                let is_double = value_type_str == "DOUBLE";

                let buffer = active_chunks.entry(grid_coord.clone()).or_insert_with(|| {
                    if is_double {
                        let mut b = Vec::with_capacity(chunk_len);
                        b.resize(chunk_len, f64::NAN);
                        ChunkData::F64(b)
                    } else {
                        let mut b = Vec::with_capacity(chunk_len);
                        b.resize(chunk_len, f32::NAN);
                        ChunkData::F32(b)
                    }
                });

                // Calculate local coordinates and flat C-contiguous index

                let mut flat_idx = 0;
                let mut stride = 1;
                for i in (0..coord_columns.len()).rev() {
                    flat_idx += local_coords[i] * stride;
                    stride *= chunk_shape[i];
                }

                match buffer {
                    ChunkData::F32(b) => {
                        let value: f32 = row.get(coord_columns.len())?;
                        b[flat_idx as usize] = value;
                    }
                    ChunkData::F64(b) => {
                        let value: f64 = row.get(coord_columns.len())?;
                        b[flat_idx as usize] = value;
                    }
                }

                // Note: Eviction will handle flushing. For MVP, we don't attempt to track "fullness"
                // perfectly since sparse arrays won't fill up linearly.
                // The eviction logic below will flush chunks when the active map gets too large.

                // Eviction check for sparse chunks
                // Evict chunks until our estimated memory usage is below the 512MB threshold.
                while active_chunks.len().saturating_mul(chunk_byte_size) >= max_memory_bytes {
                    let (oldest_key, evicted_buffer) = active_chunks.pop_first().unwrap();
                    tx.send((oldest_key, evicted_buffer))
                        .await
                        .map_err(|_| "Upload worker failed or disconnected")?;
                }

                row_count += 1;
                if row_count % 100_000 == 0 {
                    println!("Streamed {} rows...", row_count);
                }
            }

            // 6. Flush remaining edge chunks
            for (grid_coord, buffer) in active_chunks.into_iter() {
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
..", row_count);
                }
            }

            // 6. Flush remaining edge chunks
            for (grid_coord, buffer) in active_chunks.into_iter() {
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
