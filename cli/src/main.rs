use clap::{Parser, Subcommand, ValueEnum};
use duckdb::{Connection, Result};

#[derive(Clone, Debug, PartialEq, Eq, ValueEnum)]
enum OutputFormat {
    Table,
    Json,
}

#[derive(Parser)]
#[command(name = "zarrduck")]
#[command(about = "Agentic Spatial Data Engine for GeoZarr and DuckDB", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Output format (table or json)
    #[arg(global = true, long, default_value = "table")]
    output: OutputFormat,
}

enum ChunkData {
    Bool(Vec<bool>),
    Int8(Vec<i8>),
    Int16(Vec<i16>),
    Int32(Vec<i32>),
    Int64(Vec<i64>),
    UInt8(Vec<u8>),
    UInt16(Vec<u16>),
    UInt32(Vec<u32>),
    UInt64(Vec<u64>),
    Float32(Vec<f32>),
    Float64(Vec<f64>),
    String(Vec<String>),
}

#[derive(Subcommand)]
enum Commands {
    /// Discover dataset metadata
    Info {
        /// The Zarr array URI
        uri: String,
    },
    /// Extract Zarr data intersecting with vector polygons
    Extract {
        /// The Zarr array URI
        zarr_uri: String,
        /// Path to the vector boundaries (GeoJSON, Shapefile)
        vector_path: String,
        /// Output DuckDB database file
        #[arg(long)]
        out: String,
    },
    /// Open an interactive DuckDB shell loaded with the data
    Shell {
        /// The DuckDB database file to open
        db_path: String,
    },
    /// Export DuckDB query results to a Zarr array
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

fn setup_duckdb() -> Result<Connection, Box<dyn std::error::Error>> {
    let config = duckdb::Config::default().allow_unsigned_extensions()?;
    let conn = Connection::open_in_memory_with_flags(config)?;
    
    // We need to construct the path to our compiled extension
    let ext_path = if cfg!(target_os = "windows") {
        "../target/debug/geozarr.duckdb_extension"
    } else {
        "../target/debug/libgeozarr.duckdb_extension"
    };
    
    conn.execute(&format!("LOAD '{}'", ext_path), [])?;
    Ok(conn)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Info { uri } => {
            let conn = setup_duckdb()?;
            let escaped_uri = uri.replace("'", "''");
            let query = format!("SELECT array_shape, chunk_shape, data_type, crs FROM read_zarr_metadata('{}')", escaped_uri);
            
            let mut stmt = conn.prepare(&query)?;
            let mut rows = stmt.query([])?;
            
            if let Some(row) = rows.next()? {
                let array_shape: String = row.get(0)?;
                let chunk_shape: String = row.get(1)?;
                let data_type: String = row.get(2)?;
                let crs: String = row.get(3)?;
                
                // NOTE: Use the OutputFormat enum you implemented in Task 1!
                if cli.output == OutputFormat::Json {
                    let json_out = serde_json::json!({
                        "uri": uri,
                        "array_shape": array_shape,
                        "chunk_shape": chunk_shape,
                        "data_type": data_type,
                        "crs": crs
                    });
                    println!("{}", json_out.to_string());
                } else {
                    println!("GeoZarr Dataset Info:");
                    println!("URI: {}", uri);
                    println!("Shape: {}", array_shape);
                    println!("Chunks: {}", chunk_shape);
                    println!("Type: {}", data_type);
                    println!("CRS: {}", crs);
                }
            } else {
                eprintln!("Failed to read metadata for {}", uri);
                std::process::exit(1);
            }
        }
        Commands::Extract { zarr_uri, vector_path, out } => {
            let conn = Connection::open(&out)?;
            
            // Load extensions
            conn.execute("SET allow_unsigned_extensions = true", [])?;
            let ext_path = if cfg!(target_os = "windows") {
                "../target/debug/geozarr.duckdb_extension"
            } else {
                "../target/debug/libgeozarr.duckdb_extension"
            };
            conn.execute(&format!("LOAD '{}'", ext_path), [])?;
            
            // Install and load official spatial extension
            println!("Loading DuckDB spatial extension...");
            conn.execute("INSTALL spatial", [])?;
            conn.execute("LOAD spatial", [])?;
            
            println!("Extracting data... This may take a while depending on the bounding box.");
            
            // The magic query: Create a table by joining the GeoZarr pixels that intersect the vector polygons
            let query = format!(
                "CREATE TABLE extracted_data AS \n                 SELECT z.*, v.* EXCLUDE (geom) \n                 FROM read_zarr('{}') z, ST_Read('{}') v \n                 WHERE ST_Contains(v.geom, ST_Point(z.lon, z.lat))",
                zarr_uri.replace("'", "''"), vector_path.replace("'", "''")
            );
            
            match conn.execute(&query, []) {
                Ok(_) => {
                    // NOTE: Use OutputFormat::Json from Task 1
                    if cli.output == OutputFormat::Json {
                        println!(r#"{{"status": "success", "db": "{}"}}"#, out);
                    } else {
                        println!("Extraction complete! Data saved to table 'extracted_data' in {}", out);
                        println!("Run `zarrduck shell {}` to explore it.", out);
                    }
                },
                Err(e) => {
                    eprintln!("Extraction failed: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Shell { db_path } => {
            println!("Opening shell for {}", db_path);
            // TODO in Task 4
        }
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

            let data_type = match value_type_str.as_str() {
                "BOOLEAN" => zarrs::array::DataType::Bool,
                "TINYINT" => zarrs::array::DataType::Int8,
                "SMALLINT" => zarrs::array::DataType::Int16,
                "INTEGER" => zarrs::array::DataType::Int32,
                "BIGINT" => zarrs::array::DataType::Int64,
                "UTINYINT" => zarrs::array::DataType::UInt8,
                "USMALLINT" => zarrs::array::DataType::UInt16,
                "UINTEGER" => zarrs::array::DataType::UInt32,
                "UBIGINT" => zarrs::array::DataType::UInt64,
                "FLOAT" | "REAL" => zarrs::array::DataType::Float32,
                "DOUBLE" | "FLOAT8" | "DECIMAL" | "NUMERIC" => zarrs::array::DataType::Float64,
                "VARCHAR" => zarrs::array::DataType::String,
                _ => return Err(format!("Unsupported DuckDB type: {}", value_type_str).into()),
            };

            let fill_value = match data_type {
                zarrs::array::DataType::Bool => zarrs::array::FillValue::from(false),
                zarrs::array::DataType::Int8 => zarrs::array::FillValue::from(0i8),
                zarrs::array::DataType::Int16 => zarrs::array::FillValue::from(0i16),
                zarrs::array::DataType::Int32 => zarrs::array::FillValue::from(0i32),
                zarrs::array::DataType::Int64 => zarrs::array::FillValue::from(0i64),
                zarrs::array::DataType::UInt8 => zarrs::array::FillValue::from(0u8),
                zarrs::array::DataType::UInt16 => zarrs::array::FillValue::from(0u16),
                zarrs::array::DataType::UInt32 => zarrs::array::FillValue::from(0u32),
                zarrs::array::DataType::UInt64 => zarrs::array::FillValue::from(0u64),
                zarrs::array::DataType::Float32 => zarrs::array::FillValue::from(f32::NAN),
                zarrs::array::DataType::Float64 => zarrs::array::FillValue::from(f64::NAN),
                zarrs::array::DataType::String => zarrs::array::FillValue::from(""),
                _ => return Err("Unsupported DataType for FillValue".into()),
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
                        ChunkData::Bool(data) => {
                            array_clone
                                .async_store_chunk_elements(&chunk_grid, &data)
                                .await
                        }
                        ChunkData::Int8(data) => {
                            array_clone
                                .async_store_chunk_elements(&chunk_grid, &data)
                                .await
                        }
                        ChunkData::Int16(data) => {
                            array_clone
                                .async_store_chunk_elements(&chunk_grid, &data)
                                .await
                        }
                        ChunkData::Int32(data) => {
                            array_clone
                                .async_store_chunk_elements(&chunk_grid, &data)
                                .await
                        }
                        ChunkData::Int64(data) => {
                            array_clone
                                .async_store_chunk_elements(&chunk_grid, &data)
                                .await
                        }
                        ChunkData::UInt8(data) => {
                            array_clone
                                .async_store_chunk_elements(&chunk_grid, &data)
                                .await
                        }
                        ChunkData::UInt16(data) => {
                            array_clone
                                .async_store_chunk_elements(&chunk_grid, &data)
                                .await
                        }
                        ChunkData::UInt32(data) => {
                            array_clone
                                .async_store_chunk_elements(&chunk_grid, &data)
                                .await
                        }
                        ChunkData::UInt64(data) => {
                            array_clone
                                .async_store_chunk_elements(&chunk_grid, &data)
                                .await
                        }
                        ChunkData::Float32(data) => {
                            array_clone
                                .async_store_chunk_elements(&chunk_grid, &data)
                                .await
                        }
                        ChunkData::Float64(data) => {
                            array_clone
                                .async_store_chunk_elements(&chunk_grid, &data)
                                .await
                        }
                        ChunkData::String(data) => {
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

            let bytes_per_element = match data_type {
                zarrs::array::DataType::Float64
                | zarrs::array::DataType::Int64
                | zarrs::array::DataType::UInt64 => 8,
                zarrs::array::DataType::Float32
                | zarrs::array::DataType::Int32
                | zarrs::array::DataType::UInt32 => 4,
                zarrs::array::DataType::Int16 | zarrs::array::DataType::UInt16 => 2,
                zarrs::array::DataType::String => 64, // 24 byte struct + estimated heap allocation
                _ => 1,
            };
            let chunk_byte_size = chunk_len
                .checked_mul(bytes_per_element)
                .ok_or("Chunk byte size overflow")?;
            let max_memory_bytes = 512 * 1024 * 1024; // 512 MB

            // 5. Stream data from DuckDB
            let mut order_by_parts = Vec::new();
            // First, group by the chunk grid coordinate (integer division)
            for (i, c) in coord_columns.iter().enumerate() {
                let chunk_dim = chunk_shape.get(i).unwrap_or(&1);
                order_by_parts.push(format!(
                    "CAST(\"{}\" AS BIGINT) / {}",
                    c.replace("\"", "\"\""),
                    chunk_dim
                ));
            }
            // Second, order by the raw coordinates to maintain internal chunk sequence
            for c in coord_columns.iter() {
                order_by_parts.push(format!("\"{}\"", c.replace("\"", "\"\"")));
            }
            let order_by = order_by_parts.join(", ");
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

            let stream_result: Result<(), Box<dyn std::error::Error>> = (|| {
                while let Some(row) = rows.next()? {
                    let mut grid_coord = Vec::new();
                    for (i, &chunk_dim) in chunk_shape.iter().enumerate().take(coord_columns.len())
                    {
                        let val: i64 = row.get(i)?;
                        if val < 0 {
                            return Err(
                                "Coordinates must be positive 0-based integer indices".into()
                            );
                        }
                        if (val as u64) >= shape[i] {
                            return Err(format!(
                                "Coordinate index {} exceeds maximum bound of dimension {}",
                                val, shape[i]
                            )
                            .into());
                        }
                        let grid_idx = (val as u64) / chunk_dim;
                        grid_coord.push(grid_idx);
                    }

                    let mut flat_idx = 0;
                    let mut stride = 1;
                    for i in (0..coord_columns.len()).rev() {
                        flat_idx += ((row.get::<_, i64>(i)? as u64) % chunk_shape[i]) * stride;
                        stride *= chunk_shape[i];
                    }

                    let val_col_idx = coord_columns.len();
                    match data_type {
                        zarrs::array::DataType::Bool => {
                            let value: Option<bool> = row.get(val_col_idx)?;
                            let buffer =
                                active_chunks.entry(grid_coord.clone()).or_insert_with(|| {
                                    let mut b = Vec::with_capacity(chunk_len);
                                    b.resize(chunk_len, false);
                                    ChunkData::Bool(b)
                                });
                            if let Some(v) = value {
                                if let ChunkData::Bool(b) = buffer {
                                    b[flat_idx as usize] = v;
                                }
                            }
                        }
                        zarrs::array::DataType::Int8 => {
                            let value: Option<i8> = row.get(val_col_idx)?;
                            let buffer =
                                active_chunks.entry(grid_coord.clone()).or_insert_with(|| {
                                    let b = vec![0; chunk_len];
                                    ChunkData::Int8(b)
                                });
                            if let Some(v) = value {
                                if let ChunkData::Int8(b) = buffer {
                                    b[flat_idx as usize] = v;
                                }
                            }
                        }
                        zarrs::array::DataType::Int16 => {
                            let value: Option<i16> = row.get(val_col_idx)?;
                            let buffer =
                                active_chunks.entry(grid_coord.clone()).or_insert_with(|| {
                                    let b = vec![0; chunk_len];
                                    ChunkData::Int16(b)
                                });
                            if let Some(v) = value {
                                if let ChunkData::Int16(b) = buffer {
                                    b[flat_idx as usize] = v;
                                }
                            }
                        }
                        zarrs::array::DataType::Int32 => {
                            let value: Option<i32> = row.get(val_col_idx)?;
                            let buffer =
                                active_chunks.entry(grid_coord.clone()).or_insert_with(|| {
                                    let b = vec![0; chunk_len];
                                    ChunkData::Int32(b)
                                });
                            if let Some(v) = value {
                                if let ChunkData::Int32(b) = buffer {
                                    b[flat_idx as usize] = v;
                                }
                            }
                        }
                        zarrs::array::DataType::Int64 => {
                            let value: Option<i64> = row.get(val_col_idx)?;
                            let buffer =
                                active_chunks.entry(grid_coord.clone()).or_insert_with(|| {
                                    let b = vec![0; chunk_len];
                                    ChunkData::Int64(b)
                                });
                            if let Some(v) = value {
                                if let ChunkData::Int64(b) = buffer {
                                    b[flat_idx as usize] = v;
                                }
                            }
                        }
                        zarrs::array::DataType::UInt8 => {
                            let value: Option<u8> = row.get(val_col_idx)?;
                            let buffer =
                                active_chunks.entry(grid_coord.clone()).or_insert_with(|| {
                                    let b = vec![0; chunk_len];
                                    ChunkData::UInt8(b)
                                });
                            if let Some(v) = value {
                                if let ChunkData::UInt8(b) = buffer {
                                    b[flat_idx as usize] = v;
                                }
                            }
                        }
                        zarrs::array::DataType::UInt16 => {
                            let value: Option<u16> = row.get(val_col_idx)?;
                            let buffer =
                                active_chunks.entry(grid_coord.clone()).or_insert_with(|| {
                                    let b = vec![0; chunk_len];
                                    ChunkData::UInt16(b)
                                });
                            if let Some(v) = value {
                                if let ChunkData::UInt16(b) = buffer {
                                    b[flat_idx as usize] = v;
                                }
                            }
                        }
                        zarrs::array::DataType::UInt32 => {
                            let value: Option<u32> = row.get(val_col_idx)?;
                            let buffer =
                                active_chunks.entry(grid_coord.clone()).or_insert_with(|| {
                                    let b = vec![0; chunk_len];
                                    ChunkData::UInt32(b)
                                });
                            if let Some(v) = value {
                                if let ChunkData::UInt32(b) = buffer {
                                    b[flat_idx as usize] = v;
                                }
                            }
                        }
                        zarrs::array::DataType::UInt64 => {
                            let value: Option<u64> = row.get(val_col_idx)?;
                            let buffer =
                                active_chunks.entry(grid_coord.clone()).or_insert_with(|| {
                                    let b = vec![0; chunk_len];
                                    ChunkData::UInt64(b)
                                });
                            if let Some(v) = value {
                                if let ChunkData::UInt64(b) = buffer {
                                    b[flat_idx as usize] = v;
                                }
                            }
                        }
                        zarrs::array::DataType::Float32 => {
                            let value: Option<f32> = row.get(val_col_idx)?;
                            let buffer =
                                active_chunks.entry(grid_coord.clone()).or_insert_with(|| {
                                    let mut b = Vec::with_capacity(chunk_len);
                                    b.resize(chunk_len, f32::NAN);
                                    ChunkData::Float32(b)
                                });
                            if let Some(v) = value {
                                if let ChunkData::Float32(b) = buffer {
                                    b[flat_idx as usize] = v;
                                }
                            }
                        }
                        zarrs::array::DataType::Float64 => {
                            let value: Option<f64> = row.get(val_col_idx)?;
                            let buffer =
                                active_chunks.entry(grid_coord.clone()).or_insert_with(|| {
                                    let mut b = Vec::with_capacity(chunk_len);
                                    b.resize(chunk_len, f64::NAN);
                                    ChunkData::Float64(b)
                                });
                            if let Some(v) = value {
                                if let ChunkData::Float64(b) = buffer {
                                    b[flat_idx as usize] = v;
                                }
                            }
                        }
                        zarrs::array::DataType::String => {
                            let value: Option<String> = row.get(val_col_idx)?;
                            let buffer =
                                active_chunks.entry(grid_coord.clone()).or_insert_with(|| {
                                    ChunkData::String(vec![String::new(); chunk_len])
                                });
                            if let Some(v) = value {
                                if let ChunkData::String(b) = buffer {
                                    b[flat_idx as usize] = v;
                                }
                            }
                        }
                        _ => return Err("Unsupported DataType".into()),
                    }

                    // Eviction check for sparse chunks
                    // Evict chunks until our estimated memory usage is below the 512MB threshold.
                    while active_chunks.len().saturating_mul(chunk_byte_size) >= max_memory_bytes {
                        let (oldest_key, evicted_buffer) = active_chunks.pop_first().unwrap();
                        let tx_clone = tx.clone();
                        tokio::task::block_in_place(move || {
                            tx_clone
                                .blocking_send((oldest_key, evicted_buffer))
                                .map_err(|_| "Upload worker failed or disconnected")
                        })?;
                    }

                    row_count += 1;
                    if row_count % 100_000 == 0 {
                        println!("Streamed {} rows...", row_count);
                    }
                }
                Ok(())
            })();

            // 6. Flush remaining edge chunks (runs even if stream_result is an Error!)
            tokio::task::block_in_place(move || {
                for (grid_coord, buffer) in active_chunks.into_iter() {
                    let _ = tx.blocking_send((grid_coord, buffer));
                }
            });

            // 7. Wait for uploads to finish
            upload_task
                .await
                .map_err(|e| format!("Upload task panicked: {}", e))?;

            // If the stream encountered an error, propagate it now
            stream_result?;

            println!("Finished streaming {} rows.", row_count);

            println!("Export successful!");
        }
    }

    Ok(())
}
