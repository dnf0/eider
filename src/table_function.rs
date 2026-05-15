use duckdb::core::{DataChunkHandle, LogicalTypeHandle, LogicalTypeId};
use duckdb::vtab::{BindInfo, InitInfo, VTab};
use duckdb::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zarrs::array::{Array, ArrayMetadata, DataType};
use zarrs::storage::store::FilesystemStore;

macro_rules! dispatch_yield_loop {
    ($rust_type:ty, $buffer:expr, $output:expr, $state:expr, $bind_data:expr) => {{
        // Calculate how many elements we can yield this batch (max 2048)
        let chunk_len = $bind_data.chunk_shape.iter().product::<u64>() as usize;
        let elements_remaining = chunk_len - $state.local_chunk_cursor;
        let batch_size = std::cmp::min(2048, elements_remaining);

        let rank = $bind_data.shape.len();

        let coord_arrays: Vec<Option<&Vec<f64>>> = (0..rank)
            .map(|dim| $bind_data.coords.get(&$bind_data.dim_names[dim]))
            .collect();

        // Output vectors
        // Coordinates are the first `rank` columns. Value is the last column.
        let mut value_vector = $output.flat_vector(rank);
        let value_slice = value_vector.as_mut_slice::<$rust_type>();

        for i in 0..batch_size {
            let local_idx = $state.local_chunk_cursor + i;
            let global_coords = calculate_global_indices(
                local_idx,
                &$bind_data.chunk_shape,
                &$state.current_chunk_grid,
            );

            // Write coordinates
            for dim in 0..rank {
                if let Some(coord_vals) = coord_arrays[dim] {
                    let mut coord_vector = $output.flat_vector(dim);
                    let coord_slice = coord_vector.as_mut_slice::<f64>();
                    // O(1) lookup of the physical coordinate value, with graceful fallback
                    coord_slice[i] = coord_vals
                        .get(global_coords[dim] as usize)
                        .copied()
                        .unwrap_or(f64::NAN);
                } else {
                    let mut coord_vector = $output.flat_vector(dim);
                    let coord_slice = coord_vector.as_mut_slice::<i64>();
                    // Fallback to integer index
                    coord_slice[i] = global_coords[dim] as i64;
                }
            }

            // Write value
            let byte_offset = local_idx * std::mem::size_of::<$rust_type>();
            let val_bytes: [u8; std::mem::size_of::<$rust_type>()] = $buffer
                [byte_offset..byte_offset + std::mem::size_of::<$rust_type>()]
                .try_into()
                .unwrap();
            let val = <$rust_type>::from_ne_bytes(val_bytes);
            value_slice[i] = val;
        }

        // Advance state
        $state.local_chunk_cursor += batch_size;
        if $state.local_chunk_cursor >= chunk_len {
            // Chunk exhausted, move to next
            $state.local_chunk_cursor = 0;
            $state.current_chunk_buffer = None; // Drop buffer

            // Calculate chunk grid shape to know when we are done
            let mut grid_shape = vec![0; rank];
            for i in 0..rank {
                grid_shape[i] =
                    ($bind_data.shape[i] as f64 / $bind_data.chunk_shape[i] as f64).ceil() as u64;
            }

            if !increment_chunk_grid(&mut $state.current_chunk_grid, &grid_shape) {
                $state.exhausted = true;
            }
        }

        $output.set_len(batch_size);
    }};
}

pub struct ReadZarrBindData {
    pub path: String,
    pub shape: Vec<u64>,
    pub chunk_shape: Vec<u64>,
    pub data_type: DataType,
    pub dim_names: Vec<String>,
    pub coords: HashMap<String, Vec<f64>>,
}

pub struct IterationState {
    pub current_chunk_grid: Vec<u64>,
    pub local_chunk_cursor: usize,
    pub current_chunk_buffer: Option<Vec<u8>>,
    pub exhausted: bool,
}

pub struct ReadZarrInitData {
    state: Mutex<IterationState>,
}

pub struct ReadZarrVTab;

impl VTab for ReadZarrVTab {
    type InitData = ReadZarrInitData;
    type BindData = ReadZarrBindData;

    fn parameters() -> Option<Vec<LogicalTypeHandle>> {
        Some(vec![LogicalTypeHandle::from(LogicalTypeId::Varchar)])
    }

    fn bind(bind: &BindInfo) -> Result<Self::BindData, Box<dyn std::error::Error>> {
        if bind.get_parameter_count() < 1 {
            return Err("read_zarr requires at least 1 parameter (path)".into());
        }

        let path = bind.get_parameter(0).to_string();

        let store = FilesystemStore::new(&path).map_err(|e| format!("zarrs error: {}", e))?;
        let store_arc = Arc::new(store);
        let array = Array::open(Arc::clone(&store_arc), "/")
            .map_err(|e| format!("zarrs error (array): {}", e))?;

        let shape = array.shape();
        let rank = shape.len();
        let metadata = array.metadata();

        let dim_names = resolve_dimension_names(metadata, rank);

        let mut coords = std::collections::HashMap::new();
        // Eagerly load 1D coordinate arrays if they exist
        for (dim_index, name) in dim_names.iter().enumerate() {
            if let Ok(coord_array) = Array::open(Arc::clone(&store_arc), &format!("/{}", name)) {
                // Ensure it's a 1D array
                if coord_array.shape().len() == 1 {
                    // Assuming coordinate arrays are small and fit in a single chunk [0]
                    if let Ok(chunk_bytes) = coord_array.retrieve_chunk(&[0]) {
                        let bytes = chunk_bytes.into_fixed().unwrap().into_owned();
                        let vals: Vec<f64> = match coord_array.data_type() {
                            zarrs::array::DataType::Float64 => {
                                bytemuck::cast_slice::<u8, f64>(&bytes).to_vec()
                            }
                            zarrs::array::DataType::Float32 => {
                                bytemuck::cast_slice::<u8, f32>(&bytes)
                                    .iter()
                                    .map(|&v| v as f64)
                                    .collect()
                            }
                            zarrs::array::DataType::Int64 => {
                                bytemuck::cast_slice::<u8, i64>(&bytes)
                                    .iter()
                                    .map(|&v| v as f64)
                                    .collect()
                            }
                            zarrs::array::DataType::Int32 => {
                                bytemuck::cast_slice::<u8, i32>(&bytes)
                                    .iter()
                                    .map(|&v| v as f64)
                                    .collect()
                            }
                            _ => continue,
                        };
                        // Validate that the loaded chunk covers the entire dimension length
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
            _ => LogicalTypeId::Varchar, // Fallback
        };
        bind.add_result_column("value", value_type.into());

        let shape = array.shape().to_vec();
        let chunk_shape = array
            .chunk_grid()
            .chunk_shape(&vec![0; rank], &shape)
            .unwrap()
            .unwrap()
            .iter()
            .map(|n| n.get())
            .collect();
        let data_type = array.data_type().clone();

        Ok(ReadZarrBindData {
            path,
            shape,
            chunk_shape,
            data_type,
            dim_names,
            coords,
        })
    }

    fn init(_init: &InitInfo) -> Result<Self::InitData, Box<dyn std::error::Error>> {
        // We will initialize current_chunk_grid properly in func, but for now we just need a default
        Ok(ReadZarrInitData {
            state: Mutex::new(IterationState {
                current_chunk_grid: vec![0],
                local_chunk_cursor: 0,
                current_chunk_buffer: None,
                exhausted: false,
            }),
        })
    }

    fn func(
        func: &duckdb::vtab::TableFunctionInfo<ReadZarrVTab>,
        output: &mut DataChunkHandle,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let bind_data = func.get_bind_data();
        let init_data = func.get_init_data();

        let mut state = init_data.state.lock().unwrap();

        if state.exhausted {
            output.set_len(0);
            return Ok(());
        }

        // Initialize current_chunk_grid properly on first run
        if state.current_chunk_grid.len() != bind_data.shape.len() {
            state.current_chunk_grid = vec![0; bind_data.shape.len()];
        }

        // If buffer is empty, fetch the next chunk
        if state.current_chunk_buffer.is_none() {
            let store =
                FilesystemStore::new(&bind_data.path).map_err(|e| format!("zarrs error: {}", e))?;
            let array =
                Array::open(Arc::new(store), "/").map_err(|e| format!("zarrs error: {}", e))?;

            // zarrs uses `retrieve_chunk`
            let chunk_bytes = array
                .retrieve_chunk(&state.current_chunk_grid)
                .map_err(|e| format!("zarrs read error: {}", e))?;

            // Extract the raw bytes from the ArrayBytes enum.
            let bytes = chunk_bytes.into_fixed().unwrap().into_owned();
            state.current_chunk_buffer = Some(bytes);
        }

        let buffer = state.current_chunk_buffer.as_ref().unwrap();

        // Dispatch based on data type
        match bind_data.data_type {
            zarrs::array::DataType::Float32 => {
                dispatch_yield_loop!(f32, buffer, output, state, bind_data)
            }
            zarrs::array::DataType::Float64 => {
                dispatch_yield_loop!(f64, buffer, output, state, bind_data)
            }
            zarrs::array::DataType::Int32 => {
                dispatch_yield_loop!(i32, buffer, output, state, bind_data)
            }
            zarrs::array::DataType::Int64 => {
                dispatch_yield_loop!(i64, buffer, output, state, bind_data)
            }
            _ => return Err(format!("Unsupported data type: {:?}", bind_data.data_type).into()),
        }

        Ok(())
    }
}

#[allow(dead_code)] // Will be used in the final yielding task
fn increment_chunk_grid(current_grid: &mut [u64], grid_shape: &[u64]) -> bool {
    let rank = current_grid.len();
    for i in (0..rank).rev() {
        current_grid[i] += 1;
        if current_grid[i] < grid_shape[i] {
            return true; // Successfully incremented within bounds
        } else {
            current_grid[i] = 0; // Carry over to the next dimension
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
        let state = IterationState {
            current_chunk_grid: vec![0, 0, 0],
            local_chunk_cursor: 0,
            current_chunk_buffer: None,
            exhausted: false,
        };

        assert_eq!(state.current_chunk_grid, vec![0, 0, 0]);
        assert_eq!(state.local_chunk_cursor, 0);
        assert!(state.current_chunk_buffer.is_none());
        assert!(!state.exhausted);
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

        // Start at [0, 0, 0]
        let mut current = vec![0, 0, 0];

        // Increment should move the fastest varying dimension (the last one)
        assert!(increment_chunk_grid(&mut current, &grid_shape));
        assert_eq!(current, vec![0, 0, 1]);

        // Increment again should carry over
        assert!(increment_chunk_grid(&mut current, &grid_shape));
        assert_eq!(current, vec![0, 1, 0]);

        // Skip to [1, 2, 1] (the very last chunk)
        current = vec![1, 2, 1];

        // Incrementing the last chunk should return false (exhausted)
        assert!(!increment_chunk_grid(&mut current, &grid_shape));
    }
}
