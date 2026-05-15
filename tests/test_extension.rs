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

    let array = builder.build(Arc::clone(&store), "/").unwrap();
    array.store_metadata().unwrap();

    // Create a 1D physical coordinate array for `lat`
    let lat_builder = ArrayBuilder::new(
        vec![20],
        DataType::Float32,
        vec![20].try_into().unwrap(),
        FillValue::from(0.0f32),
    );
    let lat_array = lat_builder.build(Arc::clone(&store), "/lat").unwrap();
    lat_array.store_metadata().unwrap();
    // Write physical values to the lat array (e.g. 45.0, 46.0, ...)
    let lat_data: Vec<f32> = (0..20).map(|i| 45.0 + i as f32).collect();
    lat_array
        .store_chunk_elements::<f32>(&[0], &lat_data)
        .unwrap();

    let query = format!("SELECT * FROM read_zarr('{}')", store_path.display());
    let _stmt = conn.prepare(&query).expect("Prepare failed");

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

    // Verify the schema changed to DOUBLE for `lat`
    let mut info_stmt = conn.prepare(&query_info)?;
    let mut rows = info_stmt.query([])?;

    let mut column_types = Vec::new();
    while let Some(row) = rows.next()? {
        let col_type: String = row.get(1)?; // Column type is the second field in DESCRIBE
        column_types.push(col_type);
    }
    // `time` has no array, so BIGINT. `lat` has array, so DOUBLE. `value` is FLOAT.
    assert_eq!(column_types, vec!["BIGINT", "DOUBLE", "FLOAT"]);

    // Verify actual physical coordinate is yielded
    let max_lat: f64 = conn.query_row(
        &format!("SELECT max(lat) FROM read_zarr('{}')", store_path.display()),
        [],
        |row| row.get(0),
    )?;
    assert_eq!(max_lat, 64.0); // 45.0 + 19.0

    let count: i64 = conn.query_row(
        &format!("SELECT count(*) FROM read_zarr('{}')", store_path.display()),
        [],
        |row| row.get(0),
    )?;
    // The array has shape [10, 20], so total elements is 200
    assert_eq!(count, 200);

    Ok(())
}
