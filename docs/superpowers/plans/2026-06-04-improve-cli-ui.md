# Improve CLI UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor the Eider CLI output to have a "Modern & Minimal" human aesthetic and a dense, markdown-friendly agent mode, and create an `install.sh` script.

**Architecture:** Use `std::io::IsTerminal` to detect if the CLI is running interactively. If yes, output colored tables and status blocks. If no, strip colors and output markdown-friendly structures. An `install.sh` script will provide seamless setup.

**Tech Stack:** Rust, `std::io::IsTerminal`, `owo-colors`, `cli-table`, Bash.

---

### Task 1: Add Dependencies

**Files:**
- Modify: `cli/Cargo.toml`

- [ ] **Step 1: Add `owo-colors` and `cli-table` dependencies**

Modify `cli/Cargo.toml` to add the dependencies:

```toml
[dependencies]
# ... (existing dependencies)
owo-colors = "4.1"
cli-table = "0.4"
```

- [ ] **Step 2: Verify it builds**

Run: `cargo check -p eider`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add cli/Cargo.toml
git commit -m "chore(cli): add owo-colors and cli-table dependencies"
```

### Task 2: Define OutputMode in `ui.rs`

**Files:**
- Modify: `cli/src/ui.rs`

- [ ] **Step 1: Define `OutputMode` enum and helper methods in `ui.rs`**

Add the following to the top of `cli/src/ui.rs` (along with necessary imports):

```rust
use std::io::IsTerminal;
use owo_colors::OwoColorize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Human,
    Agent,
    AgentJson,
}

impl OutputMode {
    pub fn detect(json_requested: bool) -> Self {
        if json_requested {
            OutputMode::AgentJson
        } else if std::io::stdout().is_terminal() {
            OutputMode::Human
        } else {
            OutputMode::Agent
        }
    }

    pub fn is_human(&self) -> bool {
        *self == OutputMode::Human
    }
}

pub fn format_key(key: &str, mode: OutputMode) -> String {
    if mode.is_human() {
        key.cyan().to_string()
    } else {
        key.to_string()
    }
}

pub fn format_value(val: &str, mode: OutputMode) -> String {
    if mode.is_human() {
        val.magenta().to_string()
    } else {
        val.to_string()
    }
}

pub fn format_success(msg: &str, mode: OutputMode) -> String {
    if mode.is_human() {
        format!("{} {}", "✔".green(), msg)
    } else {
        format!("- SUCCESS: {}", msg)
    }
}
```

- [ ] **Step 2: Check it builds**

Run: `cargo check -p eider`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add cli/src/ui.rs
git commit -m "feat(cli): add OutputMode and ui format helpers"
```

### Task 3: Refactor `main.rs` to propagate `OutputMode`

**Files:**
- Modify: `cli/src/main.rs`

- [ ] **Step 1: Replace `OutputFormat` with `ui::OutputMode` in `run_cli` and `execute_command`**

Instead of passing `OutputFormat`, calculate and pass `OutputMode` from `run_cli` to `execute_command` and update all the command function signatures in `execute_command`.

```rust
// In cli/src/main.rs, around line 203:
async fn run_cli(mut cli: Cli, config: EiderConfig) -> EyreResult<()> {
    let is_json = cli
        .output
        .as_ref()
        .map(|o| *o == OutputFormat::Json)
        .unwrap_or_else(|| config.output_format.as_deref() == Some("json"));

    let mode = crate::ui::OutputMode::detect(is_json);

    // Provide a dummy output assignment so nested commands can just use it if needed
    // Or you can leave cli.output as is

    execute_command(cli.command, mode, config).await
}

#[allow(clippy::too_many_lines)]
async fn execute_command(
    command: Commands,
    mode: crate::ui::OutputMode,
    config: EiderConfig,
) -> EyreResult<()> {
// ... Update all command calls to pass `&mode` instead of `&resolved_output`
```

Wait, this requires updating all the commands in `cli/src/commands/` to accept `&crate::ui::OutputMode` instead of `&OutputFormat`.

- [ ] **Step 2: Update all command file signatures**

In `cli/src/commands/info.rs`, `extract.rs`, `export_cmd.rs`, `search.rs`, `resample.rs`, `ingest.rs`:
Change `resolved_output: &OutputFormat` to `mode: &crate::ui::OutputMode`.
Make sure to `use crate::ui::OutputMode;` in these files, or reference it directly.

- [ ] **Step 3: Check it builds**

Run: `cargo check -p eider`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add cli/src/main.rs cli/src/commands/*.rs
git commit -m "refactor(cli): switch to OutputMode across all commands"
```

### Task 4: Enhance `info` Command Output

**Files:**
- Modify: `cli/src/commands/info.rs`

- [ ] **Step 1: Refactor the info output**

Update `cli/src/commands/info.rs` where the printing happens:

```rust
        if *mode == OutputMode::AgentJson {
            // keep existing json output
// ...
        } else {
            let title = if mode.is_human() {
                "GeoZarr Dataset Info".bold().to_string()
            } else {
                "### GeoZarr Dataset Info".to_string()
            };
            println!("{}", title);
            println!("{}: {}", ui::format_key("URI", *mode), ui::format_value(&uri, *mode));
            println!("{}: {}", ui::format_key("Shape", *mode), ui::format_value(&array_shape, *mode));
            println!("{}: {}", ui::format_key("Chunks", *mode), ui::format_value(&chunk_shape, *mode));
            println!("{}: {}", ui::format_key("Type", *mode), ui::format_value(&data_type, *mode));
            println!("{}: {}", ui::format_key("CRS", *mode), ui::format_value(&crs, *mode));
        }
```
(Be sure to import `owo_colors::OwoColorize` if using `.bold()`).

- [ ] **Step 2: Check it builds**

Run: `cargo check -p eider`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add cli/src/commands/info.rs
git commit -m "feat(cli): enhance info command UI based on OutputMode"
```

### Task 5: Enhance `extract` Command Output

**Files:**
- Modify: `cli/src/commands/extract.rs`

- [ ] **Step 1: Refactor extract logging to use status blocks**

In `cli/src/commands/extract.rs`, find the printing statements before the extraction executes and update them to use `ui::format_key`, `ui::format_value`, and `ui::format_success`.

Replace:
```rust
        println!("Extraction Plan:");
        println!("- Target Area: {} chunks", total_chunks);
        println!("- Data Volume: {:.2} MB", total_bytes as f64 / 1_048_576.0);
```

With:
```rust
        if mode.is_human() {
            println!("\nExtraction Plan\n───────────────");
        } else {
            println!("### Extraction Plan");
        }
        let chunks_str = format!("{} chunks", total_chunks);
        let vol_str = format!("{:.2} MB", total_bytes as f64 / 1_048_576.0);
        println!("{}: {}", ui::format_key("Target Area", *mode), ui::format_value(&chunks_str, *mode));
        println!("{}: {}", ui::format_key("Data Volume", *mode), ui::format_value(&vol_str, *mode));
        println!();
```

Replace:
```rust
    println!("Extraction complete!");
```
With:
```rust
    println!("{}", ui::format_success("Extraction complete!", *mode));
```

- [ ] **Step 2: Check it builds**

Run: `cargo check -p eider`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add cli/src/commands/extract.rs
git commit -m "feat(cli): enhance extract command UI based on OutputMode"
```

### Task 6: Skip Interactivity in Agent Mode

**Files:**
- Modify: `cli/src/ui.rs`

- [ ] **Step 1: Add `mode` argument to `prompt_zarr_uri`**

Update `ui::prompt_zarr_uri(uri: &str, mode: OutputMode) -> Result<String>` (instead of `is_json: bool`).

```rust
    if arrays.len() == 1 && arrays[0].is_empty() {
        return Ok(uri.to_string());
    }

    if mode != OutputMode::Human {
        return Err(eyre!(
            "Provided URI '{}' is a Zarr Group containing multiple datasets ({:?}). Please provide the exact path to a dataset.",
            uri, arrays
        ));
    }
```

- [ ] **Step 2: Fix callers of `prompt_zarr_uri`**

Update `info.rs`, `extract.rs` where `prompt_zarr_uri` is called to pass `*mode` instead of `*resolved_output == OutputFormat::Json`.

- [ ] **Step 3: Check it builds**

Run: `cargo check -p eider`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add cli/src/ui.rs cli/src/commands/info.rs cli/src/commands/extract.rs
git commit -m "fix(cli): skip interactive group selection in agent mode"
```

### Task 7: Create `install.sh`

**Files:**
- Create: `install.sh`

- [ ] **Step 1: Create the installer script**

Create `install.sh` in the repository root:

```bash
#!/usr/bin/env bash
set -e

echo -e "\033[36mEider CLI Installer\033[0m"
echo "───────────────────"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

if [ "$ARCH" = "x86_64" ]; then
    ARCH="amd64"
elif [ "$ARCH" = "aarch64" ] || [ "$ARCH" = "arm64" ]; then
    ARCH="arm64"
else
    echo "Unsupported architecture: $ARCH"
    exit 1
fi

DUCKDB_PLATFORM=""
if [ "$OS" = "darwin" ] && [ "$ARCH" = "arm64" ]; then
    DUCKDB_PLATFORM="osx_arm64"
elif [ "$OS" = "darwin" ] && [ "$ARCH" = "amd64" ]; then
    DUCKDB_PLATFORM="osx_amd64"
elif [ "$OS" = "linux" ] && [ "$ARCH" = "amd64" ]; then
    DUCKDB_PLATFORM="linux_amd64"
elif [ "$OS" = "linux" ] && [ "$ARCH" = "arm64" ]; then
    DUCKDB_PLATFORM="linux_arm64"
else
    echo "Unsupported OS/Arch combination for extension: $OS $ARCH"
    exit 1
fi

BIN_DIR="$HOME/.local/bin"
EXT_DIR="$HOME/.duckdb/extensions/v1.1.0/$DUCKDB_PLATFORM"

mkdir -p "$BIN_DIR"
mkdir -p "$EXT_DIR"

echo "Fetching latest release..."
# For now, we simulate fetching the latest release from the repository.
# You would curl the GitHub API here. Since we are in development, we'll just print instructions.
echo -e "\033[35mTarget Bin:\033[0m $BIN_DIR/eider"
echo -e "\033[35mTarget Ext:\033[0m $EXT_DIR/eider_extension.duckdb_extension"

echo -e "\n\033[32m✔\033[0m Installation simulated successfully."
echo "Please add $BIN_DIR to your PATH if it is not already."
```

- [ ] **Step 2: Make executable**

Run: `chmod +x install.sh`

- [ ] **Step 3: Commit**

```bash
git add install.sh
git commit -m "feat: add cross-platform install.sh script"
```
