# Design: Docs Workstream C — CLI Reference

- **Date:** 2026-06-07
- **Status:** Approved (design); implementation pending
- **Scope:** The `eider` CLI reference section of the docs site. Third of five sequenced documentation workstreams (A, B done; order A→B→C→D→E). Docs-only — no CLI code changes.

## Context

Workstream A created a "CLI Reference" sidebar category holding a single thin `cli_tui.md` stub. The `eider` CLI exposes nine subcommands (from the clap definitions in `cli/src/main.rs`) plus a global `--output table|json` flag. The CLI is dual-mode: an interactive TUI for humans (inquire prompts/menus) and an agent-friendly machine mode via `--output=json` (which emits a `{"status": …}` envelope). The current stub documents only a few commands and predates recent changes (e.g. `export` now uses `--dest`, not `--output`).

Authoritative command surface (verify each against `eider <cmd> --help` at implementation):

| Command | Positional args | Options |
|---|---|---|
| `info` | `uri` | `--pin DIM=INDEX` (repeatable) |
| `extract` | `zarr_uri`, `vector_path` | `--out`, `-y/--yes`, `--pin` |
| `shell` | `db_path` | — |
| `export` | — | `--db`, `--query`, `--dest`, `--value-column`, `--chunks` |
| `completions` | `shell` | — |
| `search` | — | `--api`, `--collection`, `--bbox`, `--datetime` |
| `resample` | `input_db`, `output_db` | `--freq`, `--agg` |
| `plot` | `db_path` | `--plot-type`, `--table` (default `extracted_data`), `--value`, `--group-by`, `--pin` |
| `ingest` | `input_file`, `output_zarr_uri` | `--chunks`, `--value-column` |

Global: `--output table|json`.

## Decisions (from brainstorming)

| Decision | Choice |
|---|---|
| Structure | CLI Reference landing + one page per command |
| Landing file | Repurpose existing `cli_tui.md` (rewrite in place; path stable, no broken links) |
| End-to-end workflows | Out of scope — deferred to Guides (D); C stays pure reference |

## Page structure

All pages flat under `docs/docs/usage/` (no file moves). The `sidebars.ts` "CLI Reference" category lists, in workflow order:

1. `usage/cli_tui` — **landing** (rewritten)
2. `usage/cli_info`
3. `usage/cli_search`
4. `usage/cli_extract`
5. `usage/cli_ingest`
6. `usage/cli_export`
7. `usage/cli_resample`
8. `usage/cli_plot`
9. `usage/cli_shell`
10. `usage/cli_completions`

## Page content

### `cli_tui.md` (landing)
- Invocation and install pointer (link to Installation).
- **Global options / agent mode**: the human-TUI vs `--output=json` duality; the JSON envelope contract — success (`{"status":"success", …}`) and error (`{"status":"error","message":"…"}`) shapes — that scripts/LLM agents rely on; how non-interactive mode requires flags that the TUI would otherwise prompt for.
- A command index table (command → one-line purpose → link to its page).
- A one-line "typical workflow" pointer to Guides (D) for end-to-end tutorials.

### Per-command pages (`cli_<command>.md`)
Each follows a consistent template:
- **Synopsis** — `eider <command> <positional> [options]`.
- **Arguments** — positional args (name, meaning).
- **Options** — table of flags (flag, value/type, meaning, default), from the clap definitions.
- **Description** — what the command does.
- **Behavior** — interactive prompts shown in human/TUI mode where applicable (`search` provider/collection selection; `resample` freq/agg menus; `plot` type menu; `extract` extraction-plan + overwrite confirmation), and what `--output=json` emits (the relevant envelope/fields).
- **Examples** — verified invocations (and, where useful, the `--output=json` output).

Accuracy specifics to honor: `export` uses `--dest`; `extract` uses `--out`/`-y`/`--pin`; `plot --table` defaults to `extracted_data`; `completions` takes a positional shell name; `search` currently emits STAC **feature self-links** (documented as-is, with a cross-reference to the experimental `read_geo` STAC note in the SQL Reference so the current `search → read_geo` gap is honest, not overclaimed).

## Accuracy & verification

- Every command/flag is reconciled against `eider <cmd> --help` (the clap-generated help is the source of truth); any divergence from this spec's table is resolved in favor of the real `--help`.
- Example outputs — especially `--output=json` envelopes — are captured from real runs over the local sample (`climate_data.zarr`, `scripts/demo_region.geojson`) with the extension built; documented as-found. No aspirational flags/output.
- `cd docs && npm run build` succeeds with no broken links (Docusaurus `onBrokenLinks: 'throw'`). CLI-internal cross-links and links to Getting Started / SQL Reference resolve; references to not-yet-deepened sections (Guides, Engineering) stay plain text.

## Non-goals (deferred)

- End-to-end / workflow tutorials → Guides (D).
- SQL/extension reference (B, done); engineering deep-dives (E).
- Any CLI code change. The `search → read_geo` STAC-consumption gap is documented (not fixed) and already noted for separate tracking.
- File moves/renames of existing pages (only `cli_tui.md` is rewritten in place).
