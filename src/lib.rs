use duckdb::{Connection, Result};

pub mod table_function;
pub use table_function::ReadZarrVTab;

#[cfg(feature = "loadable-extension")]
#[duckdb::duckdb_entrypoint_c_api]
fn init(conn: Connection) -> Result<()> {
    conn.register_table_function::<ReadZarrVTab>("read_zarr")?;
    Ok(())
}
