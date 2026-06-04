# Eider CLI User Experience Design

## 1. Core Architecture & Environment Detection

**Context:** The Eider CLI needs to serve two distinct audiences: human users running commands in interactive terminals, and LLM agents executing commands via scripts or pipes.

**Design:**
- Introduce a global "Output Mode" context leveraging the `is-terminal` crate (or `std::io::IsTerminal`).
- Automatically detect the execution environment at startup:
  - If `stdout.is_terminal()` is true -> `OutputMode::Human`
  - If `stdout.is_terminal()` is false -> `OutputMode::Agent`
- Retain the explicit `--output json` flag, which forces `OutputMode::AgentJson` regardless of environment.
- In `Human` mode, apply a "Modern & Minimal" aesthetic with subtle ANSI styling using `owo-colors` or `colored`.
- In `Agent` mode, strictly strip all ANSI codes and output dense, Markdown-formatted text for maximum token efficiency and structural clarity.

## 2. Standardized `curl` Installer

**Context:** The Eider experience requires both a standalone CLI binary and a DuckDB extension file. Users shouldn't have to manually manage these artifacts.

**Design:**
- Create a cross-platform `install.sh` script designed to be run via `curl -sSfL https://raw.githubusercontent.com/dnf0/eider/main/install.sh | bash`.
- The installer will:
  1. Auto-detect OS (macOS/Linux) and Architecture (x86_64/arm64).
  2. Fetch the latest release version from the GitHub API.
  3. Download the `eider` CLI binary to `~/.local/bin` (and advise adding to PATH if needed).
  4. Download `eider_extension.duckdb_extension` directly into `~/.duckdb/extensions/v1.5.2/osx_arm64/` (or the equivalent matching path for the detected system).
- The installer output will mirror the "Modern & Minimal" aesthetic, providing clear success checkmarks and avoiding verbose curl logs.

## 3. Command Redesign & Modern Aesthetics

**Context:** Existing CLI output lacks a cohesive visual hierarchy.

**Design:**
- **Color Palette (Human Mode):**
  - Cyan: Key entities (Zarr URIs, Table Names, File Paths)
  - Magenta: Numbers, Metrics, and Dimensions
  - Green: Success indicators (e.g., `✔`)
  - Red: Error states
- **Status Blocks:** Replace verbose sequential logging with clean, spaced layout blocks for long-running operations (like `extract`).
- **Interactive Prompts:** Commands that rely on `inquire` for interactive menus (`search`, `plot`) will gracefully fail or skip interactivity when in Agent mode, requiring explicit flags instead.
- **Table Formatting:** Use `cli-table` with clean spacing for Humans, and raw Markdown tables for Agents.
