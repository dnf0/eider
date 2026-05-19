# Zarrduck CLI Configuration Management Design

**Date:** 2026-05-18

## 1. Context & Purpose
The `zarrduck` CLI currently requires users to specify output formats and target directories on every invocation. Additionally, it relies purely on the OS environment variables to authenticate against cloud storage (S3). To reduce boilerplate and improve ergonomics, we are introducing a robust Configuration Management system.

The purpose of this sub-project is to implement a hierarchical configuration resolver that combines global defaults, local project overrides, environment variables, and CLI flags, allowing users to define persistent defaults for S3 credentials, output formats, and output paths.

## 2. Core Architecture & Dependencies

We will use the **`figment`** crate as the backbone of our configuration system because it natively supports layered merging from multiple sources (TOML files, ENV vars, etc.). We will also use `directories` to reliably locate the user's global config folder.

The new dependencies in `cli/Cargo.toml` will be:
- `figment` (with `toml` and `env` features)
- `directories`
- `serde` (already present, but heavily utilized here)

## 3. Configuration Resolution Hierarchy

Values will be resolved in the following order of precedence (Highest to Lowest):

1. **CLI Flags**: Explicitly passed on the command line (e.g., `--output=json`).
2. **Environment Variables**: Prefixed with `ZARRDUCK_` (e.g., `ZARRDUCK_OUTPUT_FORMAT=json`, `ZARRDUCK_S3_ACCESS_KEY=...`).
3. **Local Config**: A `.zarrduck.toml` file located in the current working directory.
4. **Global Config**: A `config.toml` located in `~/.config/zarrduck/` (or the equivalent OS config dir).

## 4. The `ZarrduckConfig` Data Model

The application config will map to the following structure:

```toml
# General settings
output_format = "table" # or "json"
default_out = "./analysis.duckdb" # fallback for extract command

# Cloud credentials
[s3]
endpoint = "s3.amazonaws.com"
region = "us-east-1"
access_key = "..."
secret_key = "..."
```

## 5. Integration Points

### 5.1 CLI Parser Fallbacks
The `clap` CLI definitions in `main.rs` will be updated to accept `Option<T>` for fields like `out` (in the `extract` command). If the user omits the flag, the CLI will look up `default_out` from the merged `figment` config. If both are missing, it will return a clean `color-eyre` error.

### 5.2 DuckDB Secret Injection
When `setup_duckdb()` initializes the connection, it will inspect the merged config for the `[s3]` block. If S3 keys are provided, it will execute DuckDB's `CREATE SECRET` SQL command to explicitly register the credentials for the HTTPFS extension:

```sql
CREATE SECRET (
    TYPE S3,
    KEY_ID '...',
    SECRET '...',
    REGION '...',
    ENDPOINT '...'
);
```
If the `[s3]` block is missing, DuckDB will naturally fall back to its default behavior of searching the AWS credential chain.

## 6. Testing Strategy
- Unit tests will verify that `figment` correctly merges a mock global TOML and local TOML.
- Unit tests will verify that environment variables override TOML values.