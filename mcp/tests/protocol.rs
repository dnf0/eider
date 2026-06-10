//! Protocol smoke test for the rmcp stdio adapter.
//!
//! Drives the real MCP server in-process over a `tokio::io::duplex` pipe with a
//! real rmcp client: `initialize` (implicit in `serve`) -> `tools/list` (assert
//! all seven tools, each with an input schema) -> one `tools/call` for
//! `describe_dataset` against the committed sample.
//!
//! Skip-guarded on the built eider extension (mirrors the `tools.rs` unit
//! tests): without it `open_session()` cannot load eider, so the test no-ops.

use rmcp::model::CallToolRequestParams;
use rmcp::ServiceExt;
use serde_json::{Map, Value};

/// The eight tools the adapter must expose.
const EXPECTED_TOOLS: [&str; 8] = [
    "describe_dataset",
    "estimate_cost",
    "read_region",
    "zonal_stats",
    "list_tables",
    "describe_table",
    "run_sql",
    "extract_point_timeseries",
];

/// Skip-guard: returns false (and prints) when the eider extension is not built.
fn extension_available() -> bool {
    // Match the path resolution used by `eider_session`/the unit tests.
    std::env::set_var(
        "GEOZARR_ALLOW_PATH",
        format!("{}/..", env!("CARGO_MANIFEST_DIR")),
    );
    if std::env::var("EIDER_EXTENSION_PATH").is_err()
        && !std::path::Path::new("../target/debug/eider.duckdb_extension").exists()
    {
        eprintln!("skip: eider extension not built");
        return false;
    }
    true
}

fn zarr_uri() -> String {
    format!(
        "{}/../climate_data.zarr/air_temperature",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[tokio::test]
async fn protocol_initialize_list_and_call() {
    if !extension_available() {
        return;
    }

    // Wire a real server and client over an in-memory duplex transport.
    let (server_transport, client_transport) = tokio::io::duplex(8192);

    let server_handle = tokio::spawn(async move {
        let conn = eider_session::open_session().expect("open eider session");
        let server = eider_mcp::server::EiderServer::new(conn);
        let running = server.serve(server_transport).await.expect("serve server");
        running.waiting().await.expect("server run loop");
    });

    // `serve` performs the initialize handshake for us.
    let client = ().serve(client_transport).await.expect("client initialize handshake");

    // tools/list: all seven tools present, each carrying a non-empty input schema.
    let listed = client.peer().list_tools(None).await.expect("tools/list");
    let names: Vec<String> = listed.tools.iter().map(|t| t.name.to_string()).collect();
    for expected in EXPECTED_TOOLS {
        assert!(
            names.contains(&expected.to_string()),
            "tools/list missing {expected}; got {names:?}"
        );
        let tool = listed
            .tools
            .iter()
            .find(|t| t.name == expected)
            .expect("tool present");
        let schema = tool.input_schema.as_ref();
        assert_eq!(
            schema.get("type").and_then(Value::as_str),
            Some("object"),
            "{expected} input schema should be an object schema"
        );
    }
    assert_eq!(
        listed.tools.len(),
        EXPECTED_TOOLS.len(),
        "exactly the eight curated tools should be exposed; got {names:?}"
    );

    // tools/call: describe_dataset against the committed sample.
    let mut args = Map::new();
    args.insert("uri".to_string(), Value::String(zarr_uri()));
    let result = client
        .peer()
        .call_tool(CallToolRequestParams::new("describe_dataset").with_arguments(args))
        .await
        .expect("tools/call describe_dataset");

    assert_ne!(result.is_error, Some(true), "describe_dataset errored");
    let structured = result
        .structured_content
        .expect("describe_dataset returns structured content");
    let text = structured.to_string();
    assert!(
        text.contains("EPSG:4326") && text.contains("Float32"),
        "describe_dataset payload missing CRS/dtype: {text}"
    );

    client.cancel().await.expect("client shutdown");
    server_handle.abort();
}
