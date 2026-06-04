import glob
import subprocess
import shutil
import re
import os
import csv
import time

def extract_bench_time(output):
    # e.g., "populate_lat_batch_2048 time:   [1.234 ms 1.250 ms 1.270 ms]"
    match = re.search(r'time:\s+\[.*?([0-9.]+ \w+) .*?\]', output)
    if match:
        return match.group(1)
    return "Failed"

def run_loop():
    baseline_file = "extension/src/vector_writer.rs"
    backup_file = "extension/src/vector_writer.rs.bak"
    candidates = glob.glob("extension/candidates/vector_writer_seed_*.rs")

    if not candidates:
        print("No candidates found.")
        return

    # Backup baseline
    shutil.copy(baseline_file, backup_file)

    results = []

    try:
        # Run baseline
        print("Running Baseline...")
        res = subprocess.run(["cargo", "bench", "-p", "eider_extension"], capture_output=True, text=True, cwd="extension")
        baseline_time = extract_bench_time(res.stdout)
        print(f"Baseline Time: {baseline_time}")
        results.append(("baseline", baseline_time))

        for cand in sorted(candidates):
            name = os.path.basename(cand)
            print(f"Evaluating {name}...")
            shutil.copy(cand, baseline_file)

            # Run tests
            print("  Running tests...")
            test_res = subprocess.run(
                ["cargo", "test", "-p", "eider_extension", "test_populate_coordinate_batch"],
                capture_output=True, text=True, cwd="extension"
            )
            if test_res.returncode != 0:
                print("  Tests FAILED. Discarding.")
                results.append((name, "Test Failed"))
                continue

            # Run bench
            print("  Running bench...")
            bench_res = subprocess.run(["cargo", "bench", "-p", "eider_extension"], capture_output=True, text=True, cwd="extension")
            time_val = extract_bench_time(bench_res.stdout)
            print(f"  Bench Time: {time_val}")
            results.append((name, time_val))

    finally:
        # Restore baseline
        shutil.copy(backup_file, baseline_file)
        os.remove(backup_file)

    print("\n--- ERA Leaderboard Results ---")
    for name, time_val in results:
        print(f"{name}: {time_val}")

if __name__ == "__main__":
    run_loop()
