use duckdb::core::{DataChunkHandle, LogicalTypeHandle, LogicalTypeId};
use duckdb::vtab::{BindInfo, InitInfo, VTab};
use duckdb::Result;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use zarrs::array::{Array, ArrayMetadata};
use zarrs::storage::store::FilesystemStore;

pub struct ReadZarrBindData {
    path: String,
}

pub struct ReadZarrInitData {
    done: AtomicBool,
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
        let array =
            Array::open(Arc::new(store), "/").map_err(|e| format!("zarrs error (array): {}", e))?;

        let shape = array.shape();
        let rank = shape.len();
        let metadata = array.metadata();

        let dim_names = resolve_dimension_names(metadata, rank);

        // Add coordinate columns (DuckDB integers for now)
        for name in dim_names {
            bind.add_result_column(&name, LogicalTypeId::Bigint.into());
        }

        // Add the value column based on the array's data type
        let value_type = match array.data_type() {
            zarrs::array::DataType::Float32 => LogicalTypeId::Float,
            zarrs::array::DataType::Float64 => LogicalTypeId::Double,
            zarrs::array::DataType::Int32 => LogicalTypeId::Integer,
            zarrs::array::DataType::Int64 => LogicalTypeId::Bigint,
            _ => LogicalTypeId::Varchar, // Fallback
        };
        bind.add_result_column("value", value_type.into());

        Ok(ReadZarrBindData { path })
    }

    fn init(_init: &InitInfo) -> Result<Self::InitData, Box<dyn std::error::Error>> {
        Ok(ReadZarrInitData {
            done: AtomicBool::new(false),
        })
    }

    fn func(
        func: &duckdb::vtab::TableFunctionInfo<ReadZarrVTab>,
        output: &mut DataChunkHandle,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let _bind_data = func.get_bind_data();
        let init_data = func.get_init_data();

        if init_data.done.swap(true, Ordering::SeqCst) {
            output.set_len(0);
            return Ok(());
        }

        output.set_len(0);
        Ok(())
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
}
