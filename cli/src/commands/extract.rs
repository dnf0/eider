use crate::config::EiderConfig;
use crate::duckdb_utils;
use crate::ui;
use crate::ui::OutputMode;
use color_eyre::eyre::{eyre, Result as EyreResult, WrapErr};
use duckdb::Connection;
use std::io::IsTerminal;

fn fetch_bounding_box(conn: &Connection, vector_path: &str) -> EyreResult<(f64, f64, f64, f64)> {
    let mut bbox_query = conn.prepare(
        "SELECT ST_XMin(e), ST_YMin(e), ST_XMax(e), ST_YMax(e) FROM (SELECT ST_Extent(geom) as e FROM ST_Read(?))"
    ).wrap_err("Failed to prepare bounding box query")?;

    let bounds: (f64, f64, f64, f64) = bbox_query
        .query_row(duckdb::params![vector_path], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .wrap_err("Failed to compute bounding box from vector file")?;
    Ok(bounds)
}

fn check_overwrite_protection(out_path: &str, yes: bool, mode: OutputMode) -> EyreResult<bool> {
    if std::path::Path::new(out_path).exists() {
        if mode == OutputMode::AgentJson {
            return Err(eyre!(
                "Output database '{}' already exists. Aborting to prevent overwrite.",
                out_path
            ));
        } else if yes {
            std::fs::remove_file(out_path)
                .wrap_err_with(|| format!("Failed to delete existing file '{}'", out_path))?;
        } else if !std::io::stdin().is_terminal() {
            return Err(eyre!("Output database '{}' already exists. Aborting to prevent overwrite in non-interactive mode. Use --yes to force.", out_path));
        } else {
            let ans =
                inquire::Confirm::new(&format!("File '{}' already exists. Overwrite?", out_path))
                    .with_default(false)
                    .prompt()
                    .wrap_err("Failed to read user input")?;

            if !ans {
                println!("{}", ui::format_error("Aborting extraction.", mode));
                return Ok(false);
            }

            // User confirmed, so delete the file before opening it with DuckDB
            std::fs::remove_file(out_path)
                .wrap_err_with(|| format!("Failed to delete existing file '{}'", out_path))?;
        }
    }
    Ok(true)
}

fn print_extraction_plan(
    total_chunks: u64,
    total_bytes: u64,
    skip_prompts: bool,
    mode: OutputMode,
) -> EyreResult<bool> {
    // Estimate network speed at a conservative 25 MB/s
    let estimated_seconds = total_bytes as f64 / (25.0 * 1024.0 * 1024.0);

    if mode.is_human() {
        println!("\nExtraction Plan\n───────────────");
    } else {
        println!("### Extraction Plan");
    }
    let chunks_str = format!("{} chunks", total_chunks);
    let vol_str = format!("{:.2} MB", total_bytes as f64 / 1_048_576.0);
    println!(
        "{}: {}",
        ui::format_key("Target Area", mode),
        ui::format_value(&chunks_str, mode)
    );
    println!(
        "{}: {}",
        ui::format_key("Data Volume", mode),
        ui::format_value(&vol_str, mode)
    );
    println!();
    let time_str = if estimated_seconds < 60.0 {
        format!("{:.0} seconds (@ 25 MB/s)", estimated_seconds)
    } else {
        format!("{:.1} minutes (@ 25 MB/s)", estimated_seconds / 60.0)
    };
    println!(
        "{}: {}\n",
        ui::format_key("Estimated Time", mode),
        ui::format_value(&time_str, mode)
    );

    if !skip_prompts {
        let ans = inquire::Confirm::new("Proceed with extraction?")
            .with_default(true)
            .prompt()
            .wrap_err("Failed to read user input")?;

        if !ans {
            println!("{}", ui::format_error("Aborting extraction.", mode));
            return Ok(false);
        }
    }
    Ok(true)
}

pub async fn run_extract(
    zarr_uri: String,
    vector_path: String,
    out: Option<String>,
    yes: bool,
    pin: Vec<String>,
    mode: OutputMode,
    config: &EiderConfig,
) -> EyreResult<()> {
    let zarr_uri = ui::prompt_zarr_uri(&zarr_uri, mode).await?;
    let out_path = out.or_else(|| config.default_out.clone()).ok_or_else(|| {
        eyre!("Output path not specified. Use --out or set default_out in config.")
    })?;

    let skip_prompts = yes || !std::io::stdin().is_terminal() || mode == OutputMode::AgentJson;

    // Overwrite protection
    if !check_overwrite_protection(&out_path, yes, mode)? {
        return Ok(());
    }

    let db_config = duckdb::Config::default()
        .allow_unsigned_extensions()
        .wrap_err("Failed to configure unsigned extensions")?;
    let conn = Connection::open_with_flags(&out_path, db_config)
        .wrap_err_with(|| format!("Failed to open database at {}", out_path))?;

    // Load extensions
    duckdb_utils::load_geozarr_extension(&conn)?;
    duckdb_utils::inject_s3_secret(&conn, config.s3.as_ref())?;

    // Install and load official spatial extension
    if mode != OutputMode::AgentJson {
        println!("Loading DuckDB spatial extension...");
    }
    conn.execute("INSTALL spatial", [])
        .wrap_err("Failed to install spatial extension")?;
    conn.execute("LOAD spatial", [])
        .wrap_err("Failed to load spatial extension")?;

    // Calculate the bounding box of the vector file to pass to read_zarr for spatial pushdown
    let (lon_min, lat_min, lon_max, lat_max) = fetch_bounding_box(&conn, &vector_path)?;

    let pins_str = duckdb_utils::format_pins(&pin);

    let plan_query_str = format!(
        "SELECT total_chunks, total_bytes FROM plan_read_zarr(?, lon_min=?, lat_min=?, lon_max=?, lat_max=?{})",
        pins_str
    );
    let mut plan_query = conn
        .prepare(&plan_query_str)
        .wrap_err("Failed to prepare planning query")?;

    let (total_chunks, total_bytes): (u64, u64) = plan_query
        .query_row(
            duckdb::params![zarr_uri, lon_min, lat_min, lon_max, lat_max],
            |row| {
                let chunks: i64 = row.get(0)?;
                let bytes: i64 = row.get(1)?;
                Ok((chunks as u64, bytes as u64))
            },
        )
        .wrap_err("Failed to execute planning query")?;

    if mode != OutputMode::AgentJson
        && !print_extraction_plan(total_chunks, total_bytes, skip_prompts, mode)?
    {
        return Ok(());
    }
    let spinner = if mode != OutputMode::AgentJson {
        let pb =
            indicatif::ProgressBar::with_draw_target(None, indicatif::ProgressDrawTarget::stdout());
        pb.set_style(
            indicatif::ProgressStyle::default_spinner()
                .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ")
                .template("{spinner:.green} {msg}")
                .unwrap(),
        );
        pb.set_message("Extracting spatial data...");
        Some(pb)
    } else {
        None
    };

    // The magic query: Create a table by joining the GeoZarr pixels that intersect the vector polygons
    let query = format!(
        "CREATE OR REPLACE TABLE extracted_data AS
                 SELECT z.*, v.* EXCLUDE (geom)
                 FROM read_zarr(?, lon_min=?, lat_min=?, lon_max=?, lat_max=?{}) z, ST_Read(?) v
                 WHERE ST_Contains(v.geom, ST_Point(z.lon, z.lat))",
        pins_str
    );

    // Note: Since this is a blocking call, we run it in a blocking task so the tokio runtime can still tick the spinner if needed (though enable_steady_tick actually uses its own background thread).
    conn.execute(
        &query,
        duckdb::params![zarr_uri, lon_min, lat_min, lon_max, lat_max, vector_path],
    )
    .wrap_err("Spatial extraction query failed")?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
        println!("{}", ui::format_success("Extraction complete!", mode));
    }

    if mode == OutputMode::AgentJson {
        println!(r#"{{"status": "success", "db": "{}"}}"#, out_path);
    } else {
        println!(
            "Run `eider shell {}` to explore the extracted data.",
            out_path
        );
    }

    Ok(())
}
