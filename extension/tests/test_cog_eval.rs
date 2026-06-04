use duckdb::{Connection, Result};
use std::fs;

#[test]
fn test_cog_virtualization_e2e() -> Result<()> {
    std::env::set_var("GEOZARR_ALLOW_PATH", "/");
    // We already have test.tif in the workspace root
    let cog_path = "../test.tif";
    
    // Ensure the file exists
    assert!(fs::metadata(cog_path).is_ok(), "test.tif not found");

    // Load the extension via native Rust API to bypass duckdb dynamic extension version mismatches
    let conn = Connection::open_in_memory()?;
    conn.register_table_function::<eider::ReadZarrVTab>("read_zarr")?;

    // Read the chunk subset through the virtual store
    // This will test if the geozarr_core can intercept the .tif extension,
    // parse the headers, and present it to zarrs correctly.
    let query = format!(
        "SELECT COUNT(*) FROM read_zarr('{}')",
        cog_path
    );

    let mut stmt = conn.prepare(&query)?;
    let mut rows = stmt.query([])?;

    if let Some(row) = rows.next()? {
        let count: i64 = row.get(0)?;
        println!("Successfully read {} rows from COG via VirtualStore!", count);
        assert!(count > 0);
    } else {
        panic!("No rows returned");
    }

    Ok(())
}
