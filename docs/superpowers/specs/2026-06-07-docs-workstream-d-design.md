# Design: Docs Workstream D — Guides / Tutorials

- **Date:** 2026-06-07
- **Status:** Approved (design); implementation pending
- **Scope:** The task-oriented "Guides" section of the docs site. Fourth of five sequenced documentation workstreams (A, B, C done; order A→B→C→D→E). Docs-only — no product code changes.
- **Dependency:** Builds on Workstream C (CLI Reference). The guides cross-link to the per-command CLI pages, so D must be implemented on top of C (rebase onto a `main` that includes C before building/verifying).

## Context

Workstream A created a "Guides" sidebar category holding a single stale `exporting.md`, whose example uses `COPY (… read_zarr(…)) TO 's3://…' (FORMAT ZARR)`. Verified: the extension registers **no** `COPY`/`FORMAT ZARR` format — that example is fictional. The real "write Zarr" paths are the CLI commands `eider ingest` (legacy file → GeoZarr) and `eider export` (DuckDB query → Zarr). The SQL extension (B) and the full CLI (C) are now documented; Guides chain them into end-to-end, task-oriented how-tos.

## Decisions (from brainstorming)

| Decision | Choice |
|---|---|
| Guide set | Three guides: end-to-end workflow; working with cloud data; converting & exporting to Zarr |
| Structure | No separate Guides landing (three guide pages directly under the category) |
| Stale `exporting.md` | Rewritten in place as the "Converting & exporting to Zarr" guide (path stable); the fictional `COPY … FORMAT ZARR` is removed |

## Page structure

Three pages under `docs/docs/usage/` (no separate landing; `sidebars.ts` "Guides" category lists them in this order):

1. `usage/guide_workflow.md` — **End-to-end analysis workflow** (new)
2. `usage/guide_cloud.md` — **Working with cloud data** (new)
3. `usage/exporting.md` — **Converting & exporting to Zarr** (rewritten in place)

## Page content

Each guide uses a consistent shape: **Goal** → **Prerequisites** (link to Installation) → **numbered steps with verified commands/SQL** → **Result / next steps** (cross-links to relevant reference pages).

### `guide_workflow.md` — End-to-end analysis workflow
The flagship pipeline (deferred from C), over the local `climate_data.zarr` sample:
1. Inspect the array — `eider info climate_data.zarr/air_temperature`.
2. Extract a region — `eider extract climate_data.zarr/air_temperature scripts/demo_region.geojson --out analysis.duckdb --yes`.
3. Resample — `eider resample analysis.duckdb monthly.duckdb --freq month --agg avg`.
4. Visualize / explore — `eider plot analysis.duckdb --plot-type heatmap` and/or `eider shell analysis.duckdb` for ad-hoc SQL.
Narrative connects the steps; cross-links to each command's CLI Reference page. Every command verified live.

### `guide_cloud.md` — Working with cloud data
- Configure OpenDAL access: `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / `AWS_REGION` for S3; `GEOZARR_ALLOW_PATH` for local filesystem reads.
- Read remote data from **both** entry points: the SQL extension (`SELECT … FROM read_geo('s3://bucket/data.zarr', …)`) and the CLI (`eider info s3://bucket/data.zarr`, `eider extract https://…`).
- Cloud examples are explicitly labeled as **requiring credentials/network** (not locally runnable); the env-var setup and URI syntax are exact and verified for shape.

### `exporting.md` — Converting & exporting to Zarr (rewrite)
Removes the fictional `COPY … (FORMAT ZARR)`. Documents the two real paths:
- **`eider ingest`** — convert a legacy NetCDF/GeoTIFF/CSV file to a GeoZarr array.
- **`eider export`** — write a DuckDB query result to a Zarr array (`--query`, `--dest`, `--value-column`), including the 0-based-index coordinate-column contract.
Cross-links to the `ingest`/`export` CLI Reference pages. Verified against the real CLI.

## Accuracy & verification

- The runnable guides (workflow, ingest/export) are executed end-to-end against the freshly-built binary + extension over the local sample (`climate_data.zarr`, `scripts/demo_region.geojson`); documented commands and any shown output match reality.
- The cloud guide's env-var names and URI syntax are exact; remote examples are labeled as requiring cloud access (not run locally), and no fictional capability is claimed.
- `cd docs && npm run build` passes with no broken links (Docusaurus `onBrokenLinks: 'throw'`). Guides cross-link only to existing pages (Getting Started, CLI Reference [C], SQL Reference [B]); any reference to Engineering (E, not yet deepened) stays plain text.
- The fictional `COPY … FORMAT ZARR` example is eliminated from the docs.

## Non-goals (deferred)

- Engineering deep-dives (E).
- Any product/code change — the `search → read_geo` STAC-consumption gap remains documented-as-experimental and is not exercised in a guide.
- New reference content (B/C own that); guides reference, not re-document, command/function details.
- File moves/renames beyond rewriting `exporting.md` in place.
