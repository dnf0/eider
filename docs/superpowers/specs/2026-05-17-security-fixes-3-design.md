# Red Team Security Fixes 3 Design

**Date:** 2026-05-17
**Status:** Approved

## 1. Purpose & Context
A 3rd Red Team assessment identified a critical data corruption issue in the CLI's chunk streaming logic, a flaw in the LFI denylist, and some other issues. This design outlines the remediation strategy for these findings.

## 2. Remediation Architecture

### 2.1 [CRITICAL] Data Corruption in CLI Chunking
**Location:** `cli/src/main.rs` (Streaming Loop)
**Fix:** The CLI currently appends values to a flat `Vec` regardless of their true multi-dimensional coordinates, leading to incorrect placement and overwriting.
We will refactor `ChunkBuffer` to allocate a full `Vec` of `f32::NAN` up to `chunk_len` when instantiated. For every incoming row, we will calculate the `local_coords` (row coords modulo chunk shape) and map that to a 1D flat index using standard C-contiguous array layout math. We will then insert the value at that exact index: `buffer[flat_idx] = value;`.
*Note: This does not solve the eviction overwrite issue if a chunk is revisited. A true fix for that requires partial chunk updates in the storage backend or an external sorter, which is out of scope for this MVP. We will add a warning comment.*

### 2.2 [HIGH] Denylist Bypass for Local File Access (LFI)
**Location:** `extension/src/table_function.rs` (`resolve_store`)
**Fix:** Denylists are fundamentally insecure. We will replace the component-based denylist with a strict allowlist. If `GEOZARR_ALLOW_PATH` is set, we use that. If it is NOT set, we will default to the current working directory (`std::env::current_dir()`). The canonicalized requested path MUST start with the canonicalized allow path, completely neutralizing LFI vectors across all OS platforms.

### 2.3 [MEDIUM] Hardcoded Type Casts in CLI
**Location:** `cli/src/main.rs`
**Fix:** The CLI currently assumes all values are `f32`. We will parse the `DataType` of the `value_column` from the DuckDB `DESCRIBE` statement. We will use a match statement to instantiate the Zarr array with the correct type and cast the incoming rows accordingly (supporting `f32` and `f64` for now, failing gracefully on others).

### 2.4 [MEDIUM] Unbounded Dimension Iteration Risk
**Location:** `extension/src/table_function.rs` (`bind`)
**Fix:** The extension checks individual dimension coordinate array sizes but doesn't limit the *number* of dimensions. A Zarr array with 1,000 dimensions could OOM the host. We will add a hard check in `bind`: `if array.shape().len() > 16 { return Err(...); }`.

### 2.5 [LOW] Integer Overflow in Chunk Arithmetic
**Location:** `extension/src/table_function.rs` (`calculate_global_indices`)
**Fix:** Replace `(chunk_grid[i] * chunk_shape[i]) + local_coords[i]` with `chunk_grid[i].saturating_mul(chunk_shape[i]).saturating_add(local_coords[i])` to prevent theoretically possible panics on astronomically malformed metadata.

## 3. Spec Self-Review
1. **Placeholder scan:** No TBDs.
2. **Internal consistency:** The fixes directly address the identified edge cases. The limitation of the eviction overwrite is explicitly acknowledged.
3. **Scope check:** Bounded to the identified red team findings.