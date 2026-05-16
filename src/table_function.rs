use duckdb::core::{DataChunkHandle, Inserter, LogicalTypeHandle, LogicalTypeId};
use duckdb::vtab::{BindInfo, InitInfo, VTab};
use duckdb::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zarrs::array::{ArrayMetadata, DataType};
use zarrs::storage::ReadableStorageTraits;

fn resolve_store(path: &str) -> std::result::Result<Arc<dyn ReadableStorageTraits>, Box<dyn std::error::Error>> {
    if path.starts_with("s3://") {
        let bucket_and_path = path.strip_prefix("s3://").unwrap();
        let bucket = bucket_and_path.split('/').next().unwrap_or(bucket_and_path);
        let root = bucket_and_path.strip_prefix(bucket).unwrap_or("/");
        
        // Uses standard AWS environment variables automatically
        let builder = opendal::services::S3::default()
            .bucket(bucket)
            .root(root);
        
        let operator = opendal::Operator::new(builder)?.finish();
        let store = zarrs::storage::store::OpendalStore::new(operator.blocking());
        Ok(Arc::new(store))
    } else if path.starts_with("http://") || path.starts_with("https://") {
        let builder = opendal::services::Http::default()
            .endpoint(path);
        
        let operator = opendal::Operator::new(builder)?.finish();
        let store = zarrs::storage::store::OpendalStore::new(operator.blocking());
        Ok(Arc::new(store))
    } else {
        let store = zarrs::storage::store::FilesystemStore::new(path)?;
        Ok(Arc::new(store))
    }
}

trait FillValueCmp {
    fn is_fill_value(&self, fill_bytes: &[u8]) -> bool;
}

macro_rules! impl_fill_value_cmp {
    ($t:ty) => {
        impl FillValueCmp for $t {
            fn is_fill_value(&self, fill_bytes: &[u8]) -> bool {
                self.to_ne_bytes().as_ref() == fill_bytes
            }
        }
    };
}

impl_fill_value_cmp!(f32);
impl_fill_value_cmp!(f64);
impl_fill_value_cmp!(i8);
impl_fill_value_cmp!(i16);
impl_fill_value_cmp!(i32);
impl_fill_value_cmp!(i64);
impl_fill_value_cmp!(u8);
impl_fill_value_cmp!(u16);
impl_fill_value_cmp!(u32);
impl_fill_value_cmp!(u64);

impl FillValueCmp for bool {
    fn is_fill_value(&self, fill_bytes: &[u8]) -> bool {
        let b = if *self { 1u8 } else { 0u8 };
        [b].as_ref() == fill_bytes
    }
}

macro_rules! dispatch_yield_loop {
    ($rust_type:ty, $enum_variant:path, $output:expr, $local_state:expr, $global_state:expr, $bind_data:expr) => {{
        let rank = $bind_data.shape.len();
        let mut value_vector = $output.flat_vector(rank);
        
        let fill_bytes_slice = $bind_data.fill_value_bytes.as_deref().unwrap_or_default();

        let mut valid_rows = 0;

        loop {
            // If buffer is empty, lock global state to get a new chunk grid, then fetch
            if $local_state.current_chunk_buffer.is_none() {
                let mut g_state = $global_state.lock().map_err(|e| format!("Mutex poisoned: {}", e))?;
                if g_state.exhausted {
                    break;
                }

                // Copy the current global grid to our local assigned grid
                $local_state.assigned_grid = g_state.current_chunk_grid.clone();

                // Increment the global grid for the next thread
                let mut grid_shape = vec![0; rank];
                let mut chunk_bounds_min = vec![0; rank];
                let mut chunk_bounds_max = vec![0; rank];
                for i in 0..rank {
                    grid_shape[i] = ($bind_data.shape[i] as f64 / $bind_data.chunk_shape[i] as f64).ceil() as u64;
                    chunk_bounds_min[i] = $bind_data.bounds_min[i] / $bind_data.chunk_shape[i];
                    chunk_bounds_max[i] = $bind_data.bounds_max[i] / $bind_data.chunk_shape[i];
                }

                if !crate::table_function::increment_chunk_grid(
                    &mut g_state.current_chunk_grid,
                    &grid_shape,
                    &chunk_bounds_min,
                    &chunk_bounds_max,
                ) {
                    g_state.exhausted = true;
                }
                // Explicitly drop global lock before I/O
                drop(g_state);

                // Fetch the chunk lock-free
                let elements = $bind_data
                    .array
                    .retrieve_chunk_elements::<$rust_type>(&$local_state.assigned_grid)
                    .map_err(|e| format!("zarrs read error: {}", e))?;
                $local_state.current_chunk_buffer = Some($enum_variant(elements));
            }

            let buffer = match $local_state.current_chunk_buffer.as_ref().unwrap() {
                $enum_variant(buf) => buf,
                _ => return Err("Chunk buffer type mismatch".into()),
            };

            let chunk_len = $bind_data.chunk_shape.iter().product::<u64>() as usize;
            let elements_remaining = chunk_len - $local_state.local_chunk_cursor;
            let batch_size = std::cmp::min(2048, elements_remaining);

            let mut valid_coords = Vec::with_capacity(batch_size);

            for i in 0..batch_size {
                let local_idx = $local_state.local_chunk_cursor + i;
                let global_coords = crate::table_function::calculate_global_indices(
                    local_idx,
                    &$bind_data.chunk_shape,
                    &$local_state.assigned_grid,
                );

                let mut out_of_bounds = false;
                for dim in 0..rank {
                    if global_coords[dim] > $bind_data.bounds_max[dim] {
                        out_of_bounds = true;
                        break;
                    }
                }
                if !out_of_bounds {
                    valid_coords.push((local_idx, global_coords));
                }
            }

            for dim in 0..rank {
                if $local_state.projected_columns.contains(&dim) {
                    if let Some(coord_vals) = $bind_data.coords.get(&$bind_data.dim_names[dim]) {
                        let mut coord_vector = $output.flat_vector(dim);
                        let coord_slice = coord_vector.as_mut_slice::<f64>();
                        for (idx, (_, global_coords)) in valid_coords.iter().enumerate() {
                            coord_slice[valid_rows + idx] = coord_vals
                                .get(global_coords[dim] as usize)
                                .copied()
                                .unwrap_or(f64::NAN);
                        }
                    } else {
                        let mut coord_vector = $output.flat_vector(dim);
                        let coord_slice = coord_vector.as_mut_slice::<i64>();
                        for (idx, (_, global_coords)) in valid_coords.iter().enumerate() {
                            coord_slice[valid_rows + idx] = global_coords[dim] as i64;
                        }
                    }
                }
            }

            if $local_state.projected_columns.contains(&rank) {
                {
                    let value_slice = value_vector.as_mut_slice::<$rust_type>();
                    for (idx, (local_idx, _)) in valid_coords.iter().enumerate() {
                        value_slice[valid_rows + idx] = buffer[*local_idx];
                    }
                }
                for (idx, (local_idx, _)) in valid_coords.iter().enumerate() {
                    let val = buffer[*local_idx];
                    if val.is_fill_value(fill_bytes_slice) {
                        value_vector.set_null(valid_rows + idx);
                    }
                }
            }

            valid_rows += valid_coords.len();

            $local_state.local_chunk_cursor += batch_size;
            if $local_state.local_chunk_cursor >= chunk_len {
                $local_state.local_chunk_cursor = 0;
                $local_state.current_chunk_buffer = None;
            }

            if valid_rows > 0 {
                break;
            }
        }

        $output.set_len(valid_rows);
    }};
}

pub struct ReadZarrBindData {
    pub path: String,
    pub shape: Vec<u64>,
    pub chunk_shape: Vec<u64>,
    pub data_type: DataType,
    pub dim_names: Vec<String>,
    pub coords: HashMap<String, Vec<f64>>,
    pub bounds_min: Vec<u64>,
    pub bounds_max: Vec<u64>,
    pub fill_value_bytes: Option<Vec<u8>>,
    pub array: std::sync::Arc<zarrs::array::Array<dyn zarrs::storage::ReadableStorageTraits>>,
}

pub enum ChunkBuffer {
    Float32(Vec<f32>),
    Float64(Vec<f64>),
    Int32(Vec<i32>),
    Int64(Vec<i64>),
    String(Vec<String>),
    Bool(Vec<bool>),
    Int8(Vec<i8>),
    Int16(Vec<i16>),
    UInt8(Vec<u8>),
    UInt16(Vec<u16>),
    UInt32(Vec<u32>),
    UInt64(Vec<u64>),
}

pub struct GlobalState {
    pub current_chunk_grid: Vec<u64>,
    pub exhausted: bool,
}

pub struct LocalState {
    pub assigned_grid: Vec<u64>,
    pub local_chunk_cursor: usize,
    pub current_chunk_buffer: Option<ChunkBuffer>,
    pub projected_columns: Vec<usize>,
}

pub struct ReadZarrInitData {
    pub global_state: Mutex<GlobalState>,
    pub local_states: Mutex<HashMap<std::thread::ThreadId, LocalState>>,
    pub projected_columns: Vec<usize>,
}

pub struct ReadZarrVTab;

impl VTab for ReadZarrVTab {
    type InitData = ReadZarrInitData;
    type BindData = ReadZarrBindData;

    fn parameters() -> Option<Vec<LogicalTypeHandle>> {
        Some(vec![LogicalTypeHandle::from(LogicalTypeId::Varchar)])
    }

    fn named_parameters() -> Option<Vec<(String, LogicalTypeHandle)>> {
        Some(vec![
            ("lat_min".to_string(), LogicalTypeId::Double.into()),
            ("lat_max".to_string(), LogicalTypeId::Double.into()),
            ("lon_min".to_string(), LogicalTypeId::Double.into()),
            ("lon_max".to_string(), LogicalTypeId::Double.into()),
            ("time_min".to_string(), LogicalTypeId::Double.into()), // Can be cast to timestamp later if needed
            ("time_max".to_string(), LogicalTypeId::Double.into()),
        ])
    }

    fn bind(bind: &BindInfo) -> Result<Self::BindData, Box<dyn std::error::Error>> {
        if bind.get_parameter_count() < 1 {
            return Err("read_zarr requires at least 1 parameter (path)".into());
        }

        let path = bind.get_parameter(0).to_string();

        let store_arc = resolve_store(&path)?;
        let array = zarrs::array::Array::open(Arc::clone(&store_arc), "/")
            .map_err(|e| format!("zarrs error (array): {}", e))?;

        let shape = array.shape();
        let rank = shape.len();
        let metadata = array.metadata();

        let dim_names = resolve_dimension_names(metadata, rank);

        let mut coords = std::collections::HashMap::new();
        // Eagerly load 1D coordinate arrays if they exist
        for (dim_index, name) in dim_names.iter().enumerate() {
            if let Ok(coord_array) = zarrs::array::Array::open(Arc::clone(&store_arc), &format!("/{}", name)) {
                // Ensure it's a 1D array and small enough to avoid OOM
                if coord_array.shape().len() == 1 && coord_array.shape()[0] < 1_000_000 {
                    let subset = zarrs::array_subset::ArraySubset::new_with_shape(
                        coord_array.shape().to_vec(),
                    );
                    let vals_result: Result<Vec<f64>, _> = match coord_array.data_type() {
                        zarrs::array::DataType::Float64 => {
                            coord_array.retrieve_array_subset_elements::<f64>(&subset)
                        }
                        zarrs::array::DataType::Float32 => coord_array
                            .retrieve_array_subset_elements::<f32>(&subset)
                            .map(|v| v.into_iter().map(|x| x as f64).collect()),
                        zarrs::array::DataType::Int64 => coord_array
                            .retrieve_array_subset_elements::<i64>(&subset)
                            .map(|v| v.into_iter().map(|x| x as f64).collect()),
                        zarrs::array::DataType::Int32 => coord_array
                            .retrieve_array_subset_elements::<i32>(&subset)
                            .map(|v| v.into_iter().map(|x| x as f64).collect()),
                        _ => continue,
                    };

                    // Validate that the loaded chunk covers the entire dimension length
                    if let Ok(vals) = vals_result {
                        if vals.len() as u64 == shape[dim_index] {
                            coords.insert(name.clone(), vals);
                        }
                    }
                }
            }
        }

        // Add coordinate columns (DuckDB Double if physical, Bigint if fallback)
        for name in &dim_names {
            if coords.contains_key(name) {
                bind.add_result_column(name, LogicalTypeId::Double.into());
            } else {
                bind.add_result_column(name, LogicalTypeId::Bigint.into());
            }
        }

        // Add the value column based on the array's data type
        let value_type = match array.data_type() {
            DataType::Float32 => LogicalTypeId::Float,
            DataType::Float64 => LogicalTypeId::Double,
            DataType::Int32 => LogicalTypeId::Integer,
            DataType::Int64 => LogicalTypeId::Bigint,
            DataType::String => LogicalTypeId::Varchar,
            DataType::Bool => LogicalTypeId::Boolean,
            DataType::Int8 => LogicalTypeId::Tinyint,
            DataType::Int16 => LogicalTypeId::Smallint,
            DataType::UInt8 => LogicalTypeId::UTinyint,
            DataType::UInt16 => LogicalTypeId::USmallint,
            DataType::UInt32 => LogicalTypeId::UInteger,
            DataType::UInt64 => LogicalTypeId::UBigint,
            _ => return Err(format!("Unsupported data type: {:?}", array.data_type()).into()),
        };
        bind.add_result_column("value", value_type.into());

        let shape = array.shape().to_vec();
        let chunk_shape = array
            .chunk_grid()
            .chunk_shape(&vec![0; rank], &shape)
            .map_err(|_| "zarrs error: array bounds are out of grid".to_string())?
            .ok_or_else(|| "zarrs error: array has no chunk shape".to_string())?
            .iter()
            .map(|n| n.get())
            .collect();
        let data_type = array.data_type().clone();

        let mut bounds_min = vec![0; rank];
        let mut bounds_max = vec![0; rank];
        for i in 0..rank {
            bounds_max[i] = if shape[i] > 0 { shape[i] - 1 } else { 0 };
        }

        for (dim_index, name) in dim_names.iter().enumerate() {
            if let Some(coord_vals) = coords.get(name) {
                // Check for minimum bound parameter (e.g. "lat_min")
                let min_param_name = format!("{}_min", name);
                if let Some(min_val_wrapped) = bind.get_named_parameter(&min_param_name) {
                    if let Ok(min_val) = min_val_wrapped.to_string().parse::<f64>() {
                        let (translated_min, _) = crate::table_function::translate_filter(
                            coord_vals,
                            ">=",
                            min_val,
                            bounds_min[dim_index],
                            bounds_max[dim_index],
                        );
                        bounds_min[dim_index] =
                            std::cmp::max(bounds_min[dim_index], translated_min);
                    }
                }

                // Check for maximum bound parameter (e.g. "lat_max")
                let max_param_name = format!("{}_max", name);
                if let Some(max_val_wrapped) = bind.get_named_parameter(&max_param_name) {
                    if let Ok(max_val) = max_val_wrapped.to_string().parse::<f64>() {
                        let (_, translated_max) = crate::table_function::translate_filter(
                            coord_vals,
                            "<=",
                            max_val,
                            bounds_min[dim_index],
                            bounds_max[dim_index],
                        );
                        bounds_max[dim_index] =
                            std::cmp::min(bounds_max[dim_index], translated_max);
                    }
                }
            }
        }

        let fv_bytes = array.fill_value().as_ne_bytes().to_vec();
        let fill_value_bytes = if fv_bytes.is_empty() {
            None
        } else {
            Some(fv_bytes)
        };

        Ok(ReadZarrBindData {
            path,
            shape,
            chunk_shape,
            data_type,
            dim_names,
            coords,
            bounds_min,
            bounds_max,
            fill_value_bytes,
            array: std::sync::Arc::new(array),
        })
    }

    fn init(_init: &InitInfo) -> Result<Self::InitData, Box<dyn std::error::Error>> {
        let bind_data = unsafe { &*_init.get_bind_data::<ReadZarrBindData>() };

        let rank = bind_data.shape.len();
        let mut chunk_bounds_min = vec![0; rank];
        let mut chunk_bounds_max = vec![0; rank];
        for i in 0..rank {
            chunk_bounds_min[i] = bind_data.bounds_min[i] / bind_data.chunk_shape[i];
            chunk_bounds_max[i] = bind_data.bounds_max[i] / bind_data.chunk_shape[i];
        }

        let exhausted = bind_data.shape.contains(&0);

        Ok(ReadZarrInitData {
            global_state: std::sync::Mutex::new(GlobalState {
                current_chunk_grid: chunk_bounds_min.clone(),
                exhausted,
            }),
            local_states: std::sync::Mutex::new(HashMap::new()),
            projected_columns: _init
                .get_column_indices()
                .into_iter()
                .map(|i| i as usize)
                .collect(),
        })
    }

    fn func(
        func: &duckdb::vtab::TableFunctionInfo<ReadZarrVTab>,
        output: &mut DataChunkHandle,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let bind_data = func.get_bind_data();
        let init_data = func.get_init_data();

        let thread_id = std::thread::current().id();
        let mut local_states = init_data
            .local_states
            .lock()
            .map_err(|e| format!("Mutex poisoned: {}", e))?;

        // Initialize local state for this thread if it doesn't exist
        if !local_states.contains_key(&thread_id) {
            let mut g_state = init_data
                .global_state
                .lock()
                .map_err(|e| format!("Mutex poisoned: {}", e))?;

            // Initialize current_chunk_grid properly on first run
            if g_state.current_chunk_grid.len() != bind_data.shape.len() {
                g_state.current_chunk_grid = vec![0; bind_data.shape.len()];
            }

            local_states.insert(
                thread_id,
                LocalState {
                    assigned_grid: g_state.current_chunk_grid.clone(),
                    local_chunk_cursor: 0,
                    current_chunk_buffer: None,
                    projected_columns: init_data.projected_columns.clone(),
                },
            );
        }

        let local_state = local_states.get_mut(&thread_id).unwrap();

        // Dispatch based on data type
        match bind_data.data_type {
            zarrs::array::DataType::Float32 => {
                dispatch_yield_loop!(f32, ChunkBuffer::Float32, output, local_state, &init_data.global_state, bind_data)
            }
            zarrs::array::DataType::Float64 => {
                dispatch_yield_loop!(f64, ChunkBuffer::Float64, output, local_state, &init_data.global_state, bind_data)
            }
            zarrs::array::DataType::Int32 => {
                dispatch_yield_loop!(i32, ChunkBuffer::Int32, output, local_state, &init_data.global_state, bind_data)
            }
            zarrs::array::DataType::Int64 => {
                dispatch_yield_loop!(i64, ChunkBuffer::Int64, output, local_state, &init_data.global_state, bind_data)
            }
            zarrs::array::DataType::String => {
                // Strings must be handled explicitly since DuckDB Varchar FlatVectors don't use as_mut_slice
                let mut valid_rows = 0;
                let rank = bind_data.shape.len();
                let mut value_vector = output.flat_vector(rank);

                loop {
                    // If buffer is empty, fetch the next chunk using typed retrieval
                    if local_state.current_chunk_buffer.is_none() {
                        let mut g_state = init_data
                            .global_state
                            .lock()
                            .map_err(|e| format!("Mutex poisoned: {}", e))?;
                        if g_state.exhausted {
                            break;
                        }

                        // Copy the current global grid to our local assigned grid
                        local_state.assigned_grid = g_state.current_chunk_grid.clone();

                        let mut grid_shape = vec![0; rank];
                        let mut chunk_bounds_min = vec![0; rank];
                        let mut chunk_bounds_max = vec![0; rank];
                        for i in 0..rank {
                            grid_shape[i] = (bind_data.shape[i] as f64
                                / bind_data.chunk_shape[i] as f64)
                                .ceil() as u64;
                            chunk_bounds_min[i] = bind_data.bounds_min[i] / bind_data.chunk_shape[i];
                            chunk_bounds_max[i] = bind_data.bounds_max[i] / bind_data.chunk_shape[i];
                        }

                        if !crate::table_function::increment_chunk_grid(
                            &mut g_state.current_chunk_grid,
                            &grid_shape,
                            &chunk_bounds_min,
                            &chunk_bounds_max,
                        ) {
                            g_state.exhausted = true;
                        }
                        drop(g_state);

                        let elements = bind_data
                            .array
                            .retrieve_chunk_elements::<String>(&local_state.assigned_grid)
                            .map_err(|e| format!("zarrs read error: {}", e))?;
                        local_state.current_chunk_buffer = Some(ChunkBuffer::String(elements));
                    }

                    let buffer = match local_state.current_chunk_buffer.as_ref().unwrap() {
                        ChunkBuffer::String(buf) => buf,
                        _ => return Err("Chunk buffer type mismatch".into()),
                    };

                    let chunk_len = bind_data.chunk_shape.iter().product::<u64>() as usize;
                    let elements_remaining = chunk_len - local_state.local_chunk_cursor;
                    let batch_size = std::cmp::min(2048, elements_remaining);

                    let mut valid_coords = Vec::with_capacity(batch_size);
                    for i in 0..batch_size {
                        let local_idx = local_state.local_chunk_cursor + i;
                        let global_coords = crate::table_function::calculate_global_indices(
                            local_idx,
                            &bind_data.chunk_shape,
                            &local_state.assigned_grid,
                        );

                        let mut out_of_bounds = false;
                        for (dim, &global_coord) in global_coords.iter().enumerate().take(rank) {
                            if global_coord > bind_data.bounds_max[dim] {
                                out_of_bounds = true;
                                break;
                            }
                        }
                        if !out_of_bounds {
                            valid_coords.push((local_idx, global_coords));
                        }
                    }

                    for dim in 0..rank {
                        if local_state.projected_columns.contains(&dim) {
                            if let Some(coord_vals) =
                                bind_data.coords.get(&bind_data.dim_names[dim])
                            {
                                let mut coord_vector = output.flat_vector(dim);
                                let coord_slice = coord_vector.as_mut_slice::<f64>();
                                for (idx, (_, global_coords)) in valid_coords.iter().enumerate() {
                                    coord_slice[valid_rows + idx] = coord_vals
                                        .get(global_coords[dim] as usize)
                                        .copied()
                                        .unwrap_or(f64::NAN);
                                }
                            } else {
                                let mut coord_vector = output.flat_vector(dim);
                                let coord_slice = coord_vector.as_mut_slice::<i64>();
                                for (idx, (_, global_coords)) in valid_coords.iter().enumerate() {
                                    coord_slice[valid_rows + idx] = global_coords[dim] as i64;
                                }
                            }
                        }
                    }

                    if local_state.projected_columns.contains(&rank) {
                        for (idx, (local_idx, _)) in valid_coords.iter().enumerate() {
                            let val = &buffer[*local_idx];
                            if Some(val.as_bytes()) == bind_data.fill_value_bytes.as_deref() {
                                value_vector.set_null(valid_rows + idx);
                            } else {
                                // Insert string using the dedicated insert method
                                value_vector.insert(valid_rows + idx, val.as_str());
                            }
                        }
                    }

                    valid_rows += valid_coords.len();

                    local_state.local_chunk_cursor += batch_size;
                    if local_state.local_chunk_cursor >= chunk_len {
                        local_state.local_chunk_cursor = 0;
                        local_state.current_chunk_buffer = None;
                    }

                    if valid_rows > 0 {
                        break;
                    }
                }

                output.set_len(valid_rows);
            }
            zarrs::array::DataType::Bool => {
                dispatch_yield_loop!(bool, ChunkBuffer::Bool, output, local_state, &init_data.global_state, bind_data)
            }
            zarrs::array::DataType::Int8 => {
                dispatch_yield_loop!(i8, ChunkBuffer::Int8, output, local_state, &init_data.global_state, bind_data)
            }
            zarrs::array::DataType::Int16 => {
                dispatch_yield_loop!(i16, ChunkBuffer::Int16, output, local_state, &init_data.global_state, bind_data)
            }
            zarrs::array::DataType::UInt8 => {
                dispatch_yield_loop!(u8, ChunkBuffer::UInt8, output, local_state, &init_data.global_state, bind_data)
            }
            zarrs::array::DataType::UInt16 => {
                dispatch_yield_loop!(u16, ChunkBuffer::UInt16, output, local_state, &init_data.global_state, bind_data)
            }
            zarrs::array::DataType::UInt32 => {
                dispatch_yield_loop!(u32, ChunkBuffer::UInt32, output, local_state, &init_data.global_state, bind_data)
            }
            zarrs::array::DataType::UInt64 => {
                dispatch_yield_loop!(u64, ChunkBuffer::UInt64, output, local_state, &init_data.global_state, bind_data)
            }
            _ => return Err(format!("Unsupported data type: {:?}", bind_data.data_type).into()),
        }

        Ok(())
    }
}

#[allow(dead_code)] // Will be used in the final yielding task
fn increment_chunk_grid(
    current_grid: &mut [u64],
    _grid_shape: &[u64], // We can ignore grid_shape now, bounds_max is our limit
    bounds_min: &[u64],
    bounds_max: &[u64],
) -> bool {
    let rank = current_grid.len();
    for i in (0..rank).rev() {
        if current_grid[i] < bounds_max[i] {
            current_grid[i] += 1;
            return true; // Successfully incremented within bounds
        } else {
            current_grid[i] = bounds_min[i]; // Carry over to the minimum bound of this dimension
        }
    }
    false // All dimensions carried over, we are out of bounds (exhausted)
}

#[allow(dead_code)] // Will be used in Task 3
fn calculate_global_indices(
    local_cursor: usize,
    chunk_shape: &[u64],
    chunk_grid: &[u64],
) -> Vec<u64> {
    let rank = chunk_shape.len();
    let mut local_coords = vec![0; rank];
    let mut remainder = local_cursor as u64;

    // Calculate local coordinates (C-contiguous order, so we process from right to left)
    for i in (0..rank).rev() {
        let dim_size = chunk_shape[i];
        local_coords[i] = remainder % dim_size;
        remainder /= dim_size;
    }

    // Add global chunk offset
    let mut global_coords = vec![0; rank];
    for i in 0..rank {
        global_coords[i] = (chunk_grid[i] * chunk_shape[i]) + local_coords[i];
    }

    global_coords
}

#[allow(dead_code)]
fn translate_filter(
    coords: &[f64],
    operator: &str,
    value: f64,
    current_min: u64,
    current_max: u64,
) -> (u64, u64) {
    if coords.is_empty() {
        return (current_min, current_max);
    }

    // Determine if array is ascending or descending
    let _ascending = coords.first().unwrap_or(&0.0) <= coords.last().unwrap_or(&0.0);

    // A simple linear scan for MVP. Binary search can be added later for huge arrays.
    let mut matched_min = u64::MAX;
    let mut matched_max = u64::MIN;

    for (i, &coord) in coords.iter().enumerate() {
        let i = i as u64;
        let matches = match operator {
            "=" => (coord - value).abs() < 1e-8,
            "<" => coord < value,
            "<=" => coord <= value,
            ">" => coord > value,
            ">=" => coord >= value,
            _ => true,
        };
        if matches {
            matched_min = std::cmp::min(matched_min, i);
            matched_max = std::cmp::max(matched_max, i);
        }
    }

    if matched_min <= matched_max {
        (
            std::cmp::max(current_min, matched_min),
            std::cmp::min(current_max, matched_max),
        )
    } else {
        // No matches found, return empty bounds
        (1, 0)
    }
}

fn resolve_dimension_names(metadata: &ArrayMetadata, rank: usize) -> Vec<String> {
    let attributes = match metadata {
        ArrayMetadata::V2(meta) => &meta.attributes,
        ArrayMetadata::V3(meta) => &meta.attributes,
    };

    if let Some(Value::Array(dims)) = attributes.get("_ARRAY_DIMENSIONS") {
        if dims.len() == rank {
            let names: Option<Vec<String>> = dims
                .iter()
                .map(|dim| {
                    if let Value::String(s) = dim {
                        Some(s.clone())
                    } else {
                        None
                    }
                })
                .collect();
            if let Some(names) = names {
                return names;
            }
        }
    }

    // Fallback path
    (0..rank).map(|i| format!("dim_{}", i)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zarrs::array::ArrayMetadata;

    #[test]
    fn test_resolve_dimension_names_fallback() {
        let json_meta = r#"{
            "zarr_format": 2,
            "shape": [1, 2, 3],
            "chunks": [1, 2, 3],
            "dtype": "<i4",
            "compressor": null,
            "fill_value": null,
            "filters": null,
            "order": "C"
        }"#;
        let metadata_bare: ArrayMetadata = serde_json::from_str(json_meta).unwrap();
        let names = resolve_dimension_names(&metadata_bare, 3);
        assert_eq!(names, vec!["dim_0", "dim_1", "dim_2"]);
    }

    #[test]
    fn test_resolve_dimension_names_with_attributes() {
        let json_meta = r#"{
            "zarr_format": 2,
            "shape": [1, 2, 3],
            "chunks": [1, 2, 3],
            "dtype": "<i4",
            "compressor": null,
            "fill_value": null,
            "filters": null,
            "order": "C",
            "attributes": {
                "_ARRAY_DIMENSIONS": ["time", "lat", "lon"]
            }
        }"#;
        let metadata_attrs: ArrayMetadata = serde_json::from_str(json_meta).unwrap();
        let names = resolve_dimension_names(&metadata_attrs, 3);
        assert_eq!(names, vec!["time", "lat", "lon"]);
    }

    #[test]
    fn test_iteration_state_initialization() {
        let global_state = GlobalState {
            current_chunk_grid: vec![0, 0, 0],
            exhausted: false,
        };
        let local_state = LocalState {
            assigned_grid: vec![0, 0, 0],
            local_chunk_cursor: 0,
            current_chunk_buffer: None,
            projected_columns: vec![0, 1, 2],
        };
        assert_eq!(global_state.current_chunk_grid, vec![0, 0, 0]);
        assert_eq!(local_state.local_chunk_cursor, 0);
    }

    #[test]
    fn test_calculate_global_indices() {
        let chunk_shape = vec![10, 10, 10];
        let chunk_grid = vec![2, 0, 1]; // We are in chunk [2, 0, 1]

        // Test the first element in the chunk
        let indices = calculate_global_indices(0, &chunk_shape, &chunk_grid);
        assert_eq!(indices, vec![20, 0, 10]);

        // Test the 15th element (index 14). Since shape is [10, 10, 10],
        // index 14 is z=0, y=1, x=4 locally.
        // Global should be z=20, y=1, x=14
        let indices = calculate_global_indices(14, &chunk_shape, &chunk_grid);
        assert_eq!(indices, vec![20, 1, 14]);
    }

    #[test]
    fn test_increment_chunk_grid() {
        let grid_shape = vec![2, 3, 2];
        let bounds_min = vec![0, 0, 0];
        let bounds_max = vec![1, 2, 1];

        // Start at [0, 0, 0]
        let mut current = vec![0, 0, 0];

        // Increment should move the fastest varying dimension (the last one)
        assert!(increment_chunk_grid(
            &mut current,
            &grid_shape,
            &bounds_min,
            &bounds_max
        ));
        assert_eq!(current, vec![0, 0, 1]);

        // Increment again should carry over
        assert!(increment_chunk_grid(
            &mut current,
            &grid_shape,
            &bounds_min,
            &bounds_max
        ));
        assert_eq!(current, vec![0, 1, 0]);

        // Skip to [1, 2, 1] (the very last chunk)
        current = vec![1, 2, 1];

        // Incrementing the last chunk should return false (exhausted)
        assert!(!increment_chunk_grid(
            &mut current,
            &grid_shape,
            &bounds_min,
            &bounds_max
        ));
    }

    #[test]
    fn test_increment_chunk_grid_with_bounds() {
        // A 4x4x4 chunk grid
        let grid_shape = vec![4, 4, 4];

        // Bounding box from chunk [1, 1, 1] to [2, 2, 2] inclusive
        let bounds_min = vec![1, 1, 1];
        let bounds_max = vec![2, 2, 2];

        // Start at min bound
        let mut current = bounds_min.clone();

        // Increment should move the fastest varying dimension (the last one)
        assert!(increment_chunk_grid(
            &mut current,
            &grid_shape,
            &bounds_min,
            &bounds_max
        ));
        assert_eq!(current, vec![1, 1, 2]);

        // Increment again should carry over, but floor at bounds_min[2]
        assert!(increment_chunk_grid(
            &mut current,
            &grid_shape,
            &bounds_min,
            &bounds_max
        ));
        assert_eq!(current, vec![1, 2, 1]); // Notice the last dimension reset to 1, not 0

        // Skip to [2, 2, 2] (the very last chunk in the bounding box)
        current = vec![2, 2, 2];

        // Incrementing the last chunk in the bounds should return false (exhausted)
        assert!(!increment_chunk_grid(
            &mut current,
            &grid_shape,
            &bounds_min,
            &bounds_max
        ));
    }

    #[test]
    fn test_translate_filter() {
        // Array: [0.0, 10.0, 20.0, 30.0, 40.0, 50.0]
        let coords = vec![0.0, 10.0, 20.0, 30.0, 40.0, 50.0];

        // lat = 20.0  => min_idx: 2, max_idx: 2
        let (min, max) = translate_filter(&coords, "=", 20.0, 0, 5);
        assert_eq!((min, max), (2, 2));

        // lat < 25.0 => min_idx: 0, max_idx: 2
        let (min, max) = translate_filter(&coords, "<", 25.0, 0, 5);
        assert_eq!((min, max), (0, 2));

        // lat >= 30.0 => min_idx: 3, max_idx: 5
        let (min, max) = translate_filter(&coords, ">=", 30.0, 0, 5);
        assert_eq!((min, max), (3, 5));
    }
}
