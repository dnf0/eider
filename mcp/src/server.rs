//! rmcp stdio server adapter.
//!
//! Exposes the seven pure tool functions in [`crate::tools`] as MCP tools over a
//! single, sandboxed, stateful DuckDB session (eider + spatial). The session is
//! held behind a [`tokio::sync::Mutex`] so the stateful temp-table handles
//! returned by the read tools remain valid for follow-up calls.
//!
//! Each tool call is given a wall-clock budget: a watchdog task interrupts the
//! DuckDB query via [`duckdb::InterruptHandle`] if it overruns, so a runaway
//! query (e.g. an accidental cross join through `run_sql`) cannot wedge the
//! server. The interrupt handle is `Send + Sync`, so the watchdog can fire while
//! the synchronous query runs under the held connection lock.

use std::sync::Arc;
use std::time::Duration;

use color_eyre::eyre::Report;
use duckdb::Connection;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Mutex;

/// Per-call wall-clock budget. A query that overruns is interrupted.
const TOOL_TIMEOUT: Duration = Duration::from_secs(120);

/// Geographic bounding box pushed down into `read_geo`/`plan_read_geo` to prune
/// chunks. All fields are in the dataset CRS (EPSG:4326 for the demo data).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct Bbox {
    /// Western longitude bound.
    pub lon_min: f64,
    /// Southern latitude bound.
    pub lat_min: f64,
    /// Eastern longitude bound.
    pub lon_max: f64,
    /// Northern latitude bound.
    pub lat_max: f64,
}

impl Bbox {
    fn to_value(&self) -> Value {
        serde_json::json!({
            "lon_min": self.lon_min,
            "lat_min": self.lat_min,
            "lon_max": self.lon_max,
            "lat_max": self.lat_max,
        })
    }
}

/// Inclusive time-coordinate window pushed down into `read_geo`/`plan_read_geo`.
/// Values are in the dataset's native time encoding.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TimeRange {
    /// Lower time bound (inclusive).
    pub min: f64,
    /// Upper time bound (inclusive).
    pub max: f64,
}

impl TimeRange {
    fn to_value(&self) -> Value {
        serde_json::json!({ "min": self.min, "max": self.max })
    }
}

/// Parameters for `describe_dataset`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DescribeDatasetParams {
    /// Zarr dataset/array URI (local path or remote object-store URL).
    pub uri: String,
}

/// Parameters for `estimate_cost`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct EstimateCostParams {
    /// Zarr dataset/array URI.
    pub uri: String,
    /// Optional bounding box to scope (and price) the read.
    #[serde(default)]
    pub bbox: Option<Bbox>,
    /// Optional time window to scope (and price) the read.
    #[serde(default)]
    pub time: Option<TimeRange>,
}

/// Parameters for `read_region`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadRegionParams {
    /// Zarr dataset/array URI.
    pub uri: String,
    /// Optional bounding box; pushed down to prune chunks before reading.
    #[serde(default)]
    pub bbox: Option<Bbox>,
    /// Optional time window; pushed down to prune chunks before reading.
    #[serde(default)]
    pub time: Option<TimeRange>,
    /// Optional asset/variable name within the dataset.
    #[serde(default)]
    pub asset: Option<String>,
    /// Max rows to inline in the response (full result stays in the temp table).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Parameters for `zonal_stats`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ZonalStatsParams {
    /// Zarr grid URI to aggregate.
    pub grid_uri: String,
    /// Path to a vector polygon file readable by `ST_Read` (e.g. GeoJSON).
    pub polygons: String,
    /// Aggregate: one of `max`, `min`, `mean`, `sum`, `count`.
    pub metric: String,
    /// Cell-inclusion convention: `centroid`, `all_touched`, or `area_weighted`.
    pub convention: String,
    /// Value column to aggregate (defaults to `value`).
    #[serde(default)]
    pub value_col: Option<String>,
    /// Max rows to inline in the response.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Parameters for `describe_table`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DescribeTableParams {
    /// Table/handle name (alphanumeric + underscore).
    pub name: String,
}

/// Parameters for `run_sql`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunSqlParams {
    /// A single read-only statement (SELECT/WITH/DESCRIBE/SHOW/PRAGMA, or
    /// `CREATE TEMP ...` / `SET VARIABLE ...`).
    pub sql: String,
    /// Max rows to inline in the response.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// The MCP server: a single eider+spatial DuckDB session behind a mutex.
#[derive(Clone)]
pub struct EiderServer {
    conn: Arc<Mutex<Connection>>,
    tool_router: ToolRouter<Self>,
}

impl EiderServer {
    /// Build the server around an already-opened eider session.
    pub fn new(conn: Connection) -> Self {
        Self {
            conn: Arc::new(Mutex::new(conn)),
            tool_router: Self::tool_router(),
        }
    }

    /// Run a synchronous tool closure under the held connection lock, enforcing
    /// the per-call wall-clock budget by interrupting the query if it overruns.
    ///
    /// The watchdog holds an [`duckdb::InterruptHandle`] (`Send + Sync`) and
    /// fires `interrupt()` after [`TOOL_TIMEOUT`]; it is aborted once the query
    /// returns so a fast query is never affected.
    async fn run<F>(&self, f: F) -> Result<CallToolResult, ErrorData>
    where
        F: FnOnce(&Connection) -> Result<Value, Report>,
    {
        let guard = self.conn.lock().await;
        let interrupt = guard.interrupt_handle();
        let watchdog = tokio::spawn(async move {
            tokio::time::sleep(TOOL_TIMEOUT).await;
            interrupt.interrupt();
        });
        let result = f(&guard);
        watchdog.abort();
        // The tools return arbitrary JSON shapes, so we surface the payload as
        // structured content (plus a text mirror) rather than via a typed
        // output schema — `serde_json::Value` has no object root schema and rmcp
        // rejects such an output schema at tool registration.
        result
            .map(CallToolResult::structured)
            .map_err(|e| ErrorData::internal_error(format!("{e:#}"), None))
    }
}

#[tool_router(router = tool_router)]
impl EiderServer {
    /// Inspect a dataset's shape/chunking/dtype/CRS before reading it. Use this
    /// first to understand a dataset and to choose a bbox/time window.
    #[tool(
        name = "describe_dataset",
        description = "Inspect a dataset's array/chunk shape, dtype and CRS before reading. Call this first to understand a dataset."
    )]
    pub async fn describe_dataset(
        &self,
        Parameters(p): Parameters<DescribeDatasetParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(move |c| crate::tools::describe_dataset(c, &p.uri))
            .await
    }

    /// Estimate read cost (chunk/byte counts) for a region. Call this before a
    /// large `read_region` to gate cost.
    #[tool(
        name = "estimate_cost",
        description = "Estimate the read cost (total_chunks, total_bytes) for a bbox/time window. Call before a large read_region to gate cost."
    )]
    pub async fn estimate_cost(
        &self,
        Parameters(p): Parameters<EstimateCostParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let bbox = p.bbox.as_ref().map(Bbox::to_value);
        let time = p.time.as_ref().map(TimeRange::to_value);
        self.run(move |c| crate::tools::estimate_cost(c, &p.uri, bbox.as_ref(), time.as_ref()))
            .await
    }

    /// Read a bbox/time/asset window. The bbox is pushed down to prune chunks;
    /// the full result is parked in a temp table and a capped head is returned.
    #[tool(
        name = "read_region",
        description = "Read a bbox/time/asset window of a dataset. The bbox is pushed down to prune chunks. Returns a capped head plus a temp-table handle for the full result. Run estimate_cost first for large reads."
    )]
    pub async fn read_region(
        &self,
        Parameters(p): Parameters<ReadRegionParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let bbox = p.bbox.as_ref().map(Bbox::to_value);
        let time = p.time.as_ref().map(TimeRange::to_value);
        self.run(move |c| {
            crate::tools::read_region(
                c,
                &p.uri,
                bbox.as_ref(),
                time.as_ref(),
                p.asset.as_deref(),
                p.limit,
            )
        })
        .await
    }

    /// Per-polygon zonal statistics over a grid, with an explicit cell-inclusion
    /// convention.
    #[tool(
        name = "zonal_stats",
        description = "Per-polygon zonal stats over a grid. convention: use all_touched for worst-case MAX exposure (counts any cell the polygon touches); area_weighted for an area-true mean/sum (weights cells by overlap, mean/sum only); centroid is the cheapest (a cell counts only if its center is inside). metric: max|min|mean|sum|count. The polygons' bbox is pushed down to prune chunks."
    )]
    pub async fn zonal_stats(
        &self,
        Parameters(p): Parameters<ZonalStatsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(move |c| {
            crate::tools::zonal_stats(
                c,
                &p.grid_uri,
                &p.polygons,
                &p.metric,
                &p.convention,
                p.value_col.as_deref(),
                p.limit,
            )
        })
        .await
    }

    /// List the tables in the session, including temp-table result handles from
    /// prior read tools.
    #[tool(
        name = "list_tables",
        description = "List tables in the session, including the temp-table result handles produced by read_region/zonal_stats/run_sql."
    )]
    pub async fn list_tables(&self) -> Result<CallToolResult, ErrorData> {
        self.run(crate::tools::list_tables).await
    }

    /// Describe a table/handle's schema and a small sample.
    #[tool(
        name = "describe_table",
        description = "Describe a table or result handle: its schema plus a 5-row sample. Use it to inspect a handle returned by read_region/zonal_stats/run_sql."
    )]
    pub async fn describe_table(
        &self,
        Parameters(p): Parameters<DescribeTableParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(move |c| crate::tools::describe_table(c, &p.name))
            .await
    }

    /// Read-only SQL escape hatch over the session.
    #[tool(
        name = "run_sql",
        description = "Read-only SQL escape hatch over the session: a single SELECT/WITH/DESCRIBE/SHOW/PRAGMA, or CREATE TEMP ... / SET VARIABLE ... only. Writes, attaches, copies and installs are rejected. Use it to query result handles from other tools."
    )]
    pub async fn run_sql(
        &self,
        Parameters(p): Parameters<RunSqlParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(move |c| crate::tools::run_sql(c, &p.sql, p.limit))
            .await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for EiderServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(
            "Curated geospatial tools over a sandboxed DuckDB session (eider + spatial). \
             Typical flow: describe_dataset to inspect, estimate_cost to gate a large read, \
             read_region or zonal_stats to compute. Large results are parked in temp tables; \
             use list_tables/describe_table/run_sql to follow up. run_sql is read-only."
                .to_string(),
        );
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_bbox_to_value() {
        let bbox = Bbox {
            lon_min: -10.0,
            lat_min: 20.0,
            lon_max: 10.0,
            lat_max: 40.0,
        };
        let expected = json!({
            "lon_min": -10.0,
            "lat_min": 20.0,
            "lon_max": 10.0,
            "lat_max": 40.0,
        });
        assert_eq!(bbox.to_value(), expected);
    }

    #[test]
    fn test_time_range_to_value() {
        let tr = TimeRange {
            min: 100.0,
            max: 200.0,
        };
        let expected = json!({
            "min": 100.0,
            "max": 200.0,
        });
        assert_eq!(tr.to_value(), expected);
    }

    #[tokio::test]
    async fn test_server_run_success() {
        let conn = eider_session::open_session().unwrap();
        let server = EiderServer::new(conn);
        let res = server
            .run(|_conn| Ok(json!({"success": true})))
            .await
            .unwrap();

        assert_eq!(
            res.content[0],
            rmcp::model::Annotated::<rmcp::model::RawContent>::text("{\"success\":true}")
        );
    }

    #[tokio::test]
    async fn test_server_run_error() {
        let conn = eider_session::open_session().unwrap();
        let server = EiderServer::new(conn);
        let err_res = server
            .run(|_conn| Err(color_eyre::eyre::eyre!("duckdb connection error")))
            .await;

        assert!(err_res.is_err());
        let err = err_res.unwrap_err();
        assert_eq!(err.message, "duckdb connection error");
    }
}
