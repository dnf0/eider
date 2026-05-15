use duckdb::{Connection, Result};
use std::path::Path;

#[test]
fn test_read_zarr_function_compiles() {
    let ext_path = "target/debug/duckdb_geozarr.duckdb_extension";

    // We just verify the extension file was successfully packaged by `cargo duckdb-ext build`.
    // Loading it dynamically into the host duckdb CLI is not tested here because
    // DuckDB extensions are strictly tied to the exact minor version (v1.1.x vs v1.5.x)
    // and will panic on load if there's a mismatch.
    assert!(
        Path::new(ext_path).exists(),
        "Extension file not found. Please run `cargo duckdb-ext build` before `cargo test`"
    );
}

#[test]
fn test_read_zarr_schema() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.register_table_function::<geozarr::ReadZarrVTab>("read_zarr")?;

    // Create a temporary zarr store
    let temp_dir = tempfile::tempdir().unwrap();
    let store_path = temp_dir.path().join("test.zarr");

    use std::sync::Arc;
    use zarrs::array::{ArrayBuilder, DataType, FillValue};
    use zarrs::storage::store::FilesystemStore;

    let store = Arc::new(FilesystemStore::new(&store_path).unwrap());
    let mut builder = ArrayBuilder::new(
        vec![10, 20],
        DataType::Float32,
        vec![5, 5].try_into().unwrap(),
        FillValue::from(0.0f32),
    );

    // Add _ARRAY_DIMENSIONS metadata
    let mut attributes = serde_json::Map::new();
    attributes.insert(
        "_ARRAY_DIMENSIONS".to_string(),
        serde_json::json!(["time", "lat"]),
    );
    builder.attributes(attributes);

    let array = builder.build(store, "/").unwrap();
    array.store_metadata().unwrap();

    let query = format!("SELECT * FROM read_zarr('{}')", store_path.display());
    let mut stmt = conn.prepare(&query).expect("Prepare failed");

    // Use an actual query to get the columns if column_names fails
    let query_info = format!(
        "DESCRIBE SELECT * FROM read_zarr('{}')",
        store_path.display()
    );
    let mut info_stmt = conn.prepare(&query_info)?;
    let mut rows = info_stmt.query([])?;

    let mut column_names = Vec::new();
    while let Some(row) = rows.next()? {
        let col_name: String = row.get(0)?;
        column_names.push(col_name);
    }

    println!("Columns: {:?}", column_names);
    assert_eq!(column_names, vec!["time", "lat", "value"]);

    Ok(())
}
