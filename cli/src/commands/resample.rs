use crate::{duckdb_utils, ui::OutputMode};
use color_eyre::eyre::{eyre, Result as EyreResult, WrapErr};
use duckdb::Connection;

fn get_freq_and_agg(
    freq: Option<String>,
    agg: Option<String>,
    mode: OutputMode,
) -> EyreResult<(String, String)> {
    let selected_freq = if let Some(f) = freq {
        f
    } else {
        if mode == OutputMode::AgentJson {
            return Err(eyre!("--freq is required when using --output=json"));
        }
        inquire::Select::new(
            "Select temporal resampling frequency:",
            vec!["hour", "day", "week", "month", "year"],
        )
        .prompt()?
        .to_string()
    };

    let selected_agg = if let Some(a) = agg {
        a
    } else {
        if mode == OutputMode::AgentJson {
            return Err(eyre!("--agg is required when using --output=json"));
        }
        inquire::Select::new(
            "Select aggregation function:",
            vec![
                "avg", "min", "max", "sum", "count", "median", "mode", "stddev", "variance",
            ],
        )
        .prompt()?
        .to_string()
    };

    validate_agg(&selected_agg)?;
    Ok((selected_freq, selected_agg))
}

pub(crate) fn validate_agg(agg: &str) -> EyreResult<()> {
    let allowed_aggs = [
        "sum", "avg", "min", "max", "count", "mean", "median", "mode", "stddev", "variance",
    ];
    if !allowed_aggs.contains(&agg.to_lowercase().as_str()) {
        return Err(eyre!(
            "Invalid aggregation function: '{}'. Allowed: {:?}",
            agg,
            allowed_aggs
        ));
    }
    Ok(())
}

pub(crate) fn build_resample_query(
    freq: &str,
    agg: &str,
    time_col: &str,
    lat_col: &str,
    lon_col: &str,
    val_col: &str,
    time_is_numeric: bool,
) -> String {
    let time_expr = if time_is_numeric {
        format!("to_timestamp(CAST({} AS BIGINT))", time_col)
    } else {
        time_col.to_string()
    };
    format!(
        "CREATE TABLE resampled_data AS
         SELECT
             date_trunc('{}', {}) as {},
             {}, {},
             {}({}) as value
         FROM source_db.extracted_data
         GROUP BY 1, 2, 3",
        freq.replace('\'', "''"),
        time_expr,
        time_col,
        lat_col,
        lon_col,
        agg,
        val_col
    )
}

pub fn run_resample(
    input_db: String,
    output_db: String,
    freq: Option<String>,
    agg: Option<String>,
    mode: OutputMode,
) -> EyreResult<()> {
    let (selected_freq, selected_agg) = get_freq_and_agg(freq, agg, mode)?;

    if !std::path::Path::new(&input_db).exists() {
        return Err(eyre!("Input database '{}' does not exist.", input_db));
    }

    let input_conn = Connection::open(&input_db)
        .wrap_err_with(|| format!("Failed to open input database '{}'", input_db))?;

    let (time_col, lat_col, lon_col, val_col, time_is_numeric) =
        duckdb_utils::detect_columns(&input_conn, "extracted_data")?;

    if mode != OutputMode::AgentJson {
        println!(
            "Detected schema: Time='{}' (numeric={}), Spatial='{}', '{}', Value='{}'",
            time_col, time_is_numeric, lat_col, lon_col, val_col
        );
    }

    // Just close the input connection so we don't lock the file for the next step
    drop(input_conn);

    // Overwrite protection for output db
    if std::path::Path::new(&output_db).exists() {
        if mode == OutputMode::AgentJson {
            return Err(eyre!(
                "Output database '{}' already exists. Aborting.",
                output_db
            ));
        } else {
            let ans =
                inquire::Confirm::new(&format!("File '{}' already exists. Overwrite?", output_db))
                    .with_default(false)
                    .prompt()
                    .wrap_err("Failed to read user input")?;

            if !ans {
                println!("Aborting resampling.");
                return Ok(());
            }
            std::fs::remove_file(&output_db)
                .wrap_err_with(|| format!("Failed to delete '{}'", output_db))?;
        }
    }

    let conn = Connection::open(&output_db)
        .wrap_err_with(|| format!("Failed to open output database '{}'", output_db))?;

    let spinner = if mode != OutputMode::AgentJson {
        let pb =
            indicatif::ProgressBar::with_draw_target(None, indicatif::ProgressDrawTarget::stdout());
        pb.set_style(
            indicatif::ProgressStyle::default_spinner()
                .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ")
                .template("{spinner:.green} {msg}")
                .unwrap(),
        );
        pb.set_message("Resampling time-series data...");
        Some(pb)
    } else {
        None
    };

    conn.execute(&format!("ATTACH '{}' AS source_db", input_db), [])
        .wrap_err("Failed to attach input database")?;

    let query = build_resample_query(
        &selected_freq,
        &selected_agg,
        &time_col,
        &lat_col,
        &lon_col,
        &val_col,
        time_is_numeric,
    );

    // Note: Since this is a blocking call, we run it directly on this thread. The tokio runtime isn't heavily needed here since it's local.
    conn.execute(&query, [])
        .wrap_err("Resampling query failed")?;

    if let Some(pb) = spinner {
        pb.finish_and_clear();
        println!("Resampling complete!");
    }

    if mode == OutputMode::AgentJson {
        println!(r#"{{"status": "success", "db": "{}"}}"#, output_db);
    } else {
        println!("Data saved to table 'resampled_data' in {}", output_db);
        println!("Run `eider shell {}` to explore it.", output_db);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_agg_accepts_known() {
        for a in [
            "sum", "avg", "min", "max", "count", "mean", "median", "mode", "stddev", "variance",
        ] {
            assert!(validate_agg(a).is_ok(), "{a} should be valid");
        }
    }

    #[test]
    fn validate_agg_is_case_insensitive() {
        assert!(validate_agg("AVG").is_ok());
    }

    #[test]
    fn validate_agg_rejects_unknown() {
        assert!(validate_agg("hack; DROP TABLE").is_err());
    }

    #[test]
    fn build_resample_query_numeric_time_wraps_timestamp() {
        let q = build_resample_query("year", "avg", "time", "lat", "lon", "value", true);
        assert!(q.contains("to_timestamp(CAST(time AS BIGINT))"));
        assert!(q.contains("date_trunc('year'"));
        assert!(q.contains("avg(value)"));
    }

    #[test]
    fn build_resample_query_text_time_uses_column_directly() {
        let q = build_resample_query("month", "sum", "time", "lat", "lon", "value", false);
        assert!(q.contains("date_trunc('month', time)"));
        assert!(!q.contains("to_timestamp"));
    }
}
