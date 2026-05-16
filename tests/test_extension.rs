use duckdb::{Connection, Result};
use std::path::Path;

#[test]
fn test_new_data_types() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.register_table_function::<geozarr::ReadZarrVTab>("read_zarr")?;

    let temp_dir = tempfile::tempdir().unwrap();
    let store_path = temp_dir.path().join("test_types.zarr");

    use std::sync::Arc;
    use zarrs::array::{ArrayBuilder, DataType, FillValue};
    use zarrs::storage::store::FilesystemStore;

    let store = Arc::new(FilesystemStore::new(&store_path).unwrap());
    
    // Test Boolean
    let bool_builder = ArrayBuilder::new(
        vec![5],
        DataType::Bool,
        vec![5].try_into().unwrap(),
        FillValue::from(false),
    );
    let bool_array = bool_builder.build(Arc::clone(&store), "/bool").unwrap();
    bool_array.store_metadata().unwrap();
    let bool_data: Vec<bool> = vec![true, false, true, true, false];
    bool_array.store_chunk_elements(&[0], &bool_data).unwrap();

    let query_bool = format!("SELECT * FROM read_zarr('{}/bool')", store_path.display());
    let mut stmt_bool = conn.prepare(&query_bool)?;
    let mut rows_bool = stmt_bool.query([])?;
    
    let mut bool_results = Vec::new();
    while let Some(row) = rows_bool.next()? {
        bool_results.push(row.get::<_, Option<bool>>(1)?);
    }
    assert_eq!(bool_results, vec![Some(true), None, Some(true), Some(true), None]);

    // Test Int8
    let i8_builder = ArrayBuilder::new(
        vec![5],
        DataType::Int8,
        vec![5].try_into().unwrap(),
        FillValue::from(0i8),
    );
    let i8_array = i8_builder.build(Arc::clone(&store), "/i8").unwrap();
    i8_array.store_metadata().unwrap();
    let i8_data: Vec<i8> = vec![-10, 20, -30, 40, -50];
    i8_array.store_chunk_elements(&[0], &i8_data).unwrap();

    let query_i8 = format!("SELECT * FROM read_zarr('{}/i8')", store_path.display());
    let mut stmt_i8 = conn.prepare(&query_i8)?;
    let mut rows_i8 = stmt_i8.query([])?;
    
    let mut i8_results = Vec::new();
    while let Some(row) = rows_i8.next()? {
        i8_results.push(row.get::<_, i8>(1)?);
    }
    assert_eq!(i8_results, vec![-10, 20, -30, 40, -50]);

    Ok(())
}

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
    lat_array.store_chunk_elements(&[0], &lat_data).unwrap();

    // Write actual data to the main array so we can verify sums
    let mut val = 0.0f32;
    for t_c in 0..2 {
        for l_c in 0..4 {
            let chunk_data: Vec<f32> = (0..25)
                .map(|_| {
                    let v = val;
                    val += 1.0;
                    v
                })
                .collect();
            array
                .store_chunk_elements(&[t_c, l_c], &chunk_data)
                .unwrap();
        }
    }

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

    // Test that named parameters compile and execute and filter down the rows
    let query_params = format!(
        "SELECT count(*) FROM read_zarr('{}', lat_min := 50.0, lat_max := 55.0)",
        store_path.display()
    );
    let mut stmt_params = conn.prepare(&query_params)?;
    let count_params: i64 = stmt_params.query_row([], |row| row.get(0))?;
    // lat goes from 45.0 to 64.0 (20 elements).
    // 50.0 to 55.0 covers indices 5 through 10.
    // Chunk shape is 5. So it fetches chunks 1 and 2.
    // Chunks 1 and 2 cover indices 5 through 14.
    // The table function now prunes rows exceeding bounds_max (index 10), so it yields 6 elements (5 through 10).
    // The other dimension (time) is length 10.
    // Total expected rows yielded: 10 * 6 = 60.
    assert_eq!(count_params, 60);

    // Test Projection Pushdown: Aggregation without coordinate columns
    let query_proj = format!(
        "SELECT SUM(value) FROM read_zarr('{}')",
        store_path.display()
    );
    let mut stmt_proj = conn.prepare(&query_proj)?;
    // If projection pushdown fails, this might panic or return bad data
    let sum_val: f64 = stmt_proj.query_row([], |row| row.get(0))?;
    println!("Total sum: {}", sum_val);
    assert_eq!(sum_val, 19900.0); // sum(0..=199)

    // Test SQL NULL Mapping
    // The very first element inserted was 0.0, which matches the FillValue.
    // Therefore, count(value) should be 199 (since NULLs are not counted).
    let query_null = format!(
        "SELECT count(value) FROM read_zarr('{}')",
        store_path.display()
    );
    let mut stmt_null = conn.prepare(&query_null)?;
    let non_null_count: i64 = stmt_null.query_row([], |row| row.get(0))?;
    assert_eq!(non_null_count, 199);

    // Test Projection Pushdown: Aggregation without value column
    let query_coord_proj = format!("SELECT SUM(lat) FROM read_zarr('{}')", store_path.display());
    let mut stmt_coord_proj = conn.prepare(&query_coord_proj)?;
    let sum_lat: f64 = stmt_coord_proj.query_row([], |row| row.get(0))?;
    // lat goes from 45.0 to 64.0. The sum of 45..64 is 1090.
    // Since there are 10 time intervals, the total sum is 1090 * 10 = 10900.
    assert_eq!(sum_lat, 10900.0);

    // Test Corrupted Chunk Bytes Error Handling
    // We truncate chunk [0, 0] so it's too small for the expected data type
    let chunk_path = store_path.join("c").join("0").join("0");
    std::fs::write(&chunk_path, vec![0u8; 1]).unwrap(); // Write a 1-byte chunk file
    let query_corrupt = format!(
        "SELECT SUM(value) FROM read_zarr('{}')",
        store_path.display()
    );
    let mut stmt_corrupt = conn.prepare(&query_corrupt)?;
    let result = stmt_corrupt.query_row([], |row| row.get::<_, f64>(0));
    assert!(result.is_err());
    let err_str = result.unwrap_err().to_string();
    println!("Corrupted chunk error: {}", err_str);
    assert!(err_str.contains("zarrs read error"));

    Ok(())
}
