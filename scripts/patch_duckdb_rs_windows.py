import sys
import re
import os

def patch_file(file_path):
    print(f"Patching {file_path}...")
    if not os.path.exists(file_path):
        print(f"Error: {file_path} does not exist.")
        sys.exit(1)

    with open(file_path, 'r') as f:
        code = f.read()

    # Change the match expression to cast to i32 for Windows compat
    code = code.replace("match value {", "match value as i32 {")

    # Cast match arms back to u32 so they compile
    code = re.sub(r"(= DUCKDB_TYPE_DUCKDB_TYPE_[A-Z0-9_]+),", r"\1 as u32,", code)

    # Add try_into().unwrap() to handle type conversions
    code = code.replace(
        "duckdb_create_logical_type(id as u32)",
        "duckdb_create_logical_type((id as u32).try_into().unwrap())"
    )
    code = code.replace(
        "duckdb_get_type_id(self.ptr)",
        "duckdb_get_type_id(self.ptr).try_into().unwrap()"
    )
    code = code.replace(
        "duckdb_create_logical_type(DUCKDB_TYPE_DUCKDB_TYPE_INVALID)",
        "duckdb_create_logical_type((DUCKDB_TYPE_DUCKDB_TYPE_INVALID as i32).try_into().unwrap())"
    )
    code = code.replace(
        "raw_id(), DUCKDB_TYPE_DUCKDB_TYPE_INVALID)",
        "raw_id(), DUCKDB_TYPE_DUCKDB_TYPE_INVALID as u32)"
    )

    with open(file_path, 'w') as f:
        f.write(code)

    print("Patch applied successfully.")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python patch_duckdb_rs_windows.py <path_to_logical_type.rs>")
        sys.exit(1)

    patch_file(sys.argv[1])
