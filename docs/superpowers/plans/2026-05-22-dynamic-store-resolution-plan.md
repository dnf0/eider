# Dynamic Cloud Storage Resolution Plan

## Objective
Remove the fragile, hardcoded cloud provider URLs (e.g., `s3.amazonaws.com`, `blob.core.windows.net`) from `geozarr_core::store::resolve_sync_store`. The goal is to support generic, configurable cloud endpoints (like MinIO, Cloudflare R2, or custom AWS regions) without relying on manual URL string manipulation.

## Current Architecture Friction
Currently, `resolve_sync_store` intercepts `s3://` and `abfs://` paths and rewrites them into hardcoded HTTP URLs so they can be fetched by the custom `ureq`-based `SyncHttpStore`. While `resolve_async_store` (used by the CLI) properly leverages `opendal`'s robust connection builders, the synchronous DuckDB extension is stuck with the custom HTTP store because the extension must run synchronously.

## Proposed Solution: Leverage OpenDAL Blocking API
OpenDAL natively supports a `blocking` API that abstracts away synchronous I/O over async runtimes.

By enabling the `blocking` feature in OpenDAL, we can completely delete our custom `SyncHttpStore` and use OpenDAL for both synchronous and asynchronous operations. OpenDAL natively reads environment variables (like `AWS_ENDPOINT_URL`, `AWS_REGION`) to configure connections dynamically.

## Implementation Steps

### 1. Update Dependencies
- In `geozarr_core/Cargo.toml`, update `opendal` to include the `blocking` feature: `features = ["services-s3", "services-http", "services-fs", "blocking"]`.
- Remove `ureq` from `geozarr_core/Cargo.toml` as it will no longer be needed.

### 2. Implement `SyncOpendalStore`
- Create a synchronous wrapper for OpenDAL in `geozarr_core/src/store.rs` that implements `zarrs::storage::ReadableStorageTraits`.
- This wrapper will hold an `opendal::BlockingOperator` and translate Zarr `get`, `get_partial_values`, and `size` requests into blocking OpenDAL calls.

### 3. Refactor `resolve_sync_store`
- Rewrite `resolve_sync_store` to mirror the logic of `resolve_async_store`.
- When an `s3://` path is provided, use `opendal::services::S3::default().bucket(...).root(...)`. OpenDAL will automatically construct the correct endpoints using the system's AWS environment configuration.
- Return the new `SyncOpendalStore` wrapped in an `Arc`.

### 4. Delete Legacy Code
- Remove the `SyncHttpStore` implementation entirely from `geozarr_core/src/store.rs`.
- Remove the hardcoded URL string manipulation blocks.

### 5. Verification
- Run `cargo test -p geozarr_core`.
- Execute an E2E test querying an S3 bucket to ensure the blocking OpenDAL operator functions correctly within the DuckDB thread pool.

## Acceptance Criteria
- `geozarr_core` no longer contains the strings `s3.amazonaws.com` or `blob.core.windows.net`.
- Queries to `s3://` paths work seamlessly using standard AWS environment variables for endpoint resolution.
- The codebase complexity is further reduced by eliminating the custom `ureq` store implementation.
