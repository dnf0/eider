import json

with open("extension/candidates/mcts_state.json", "r") as f:
    state = json.load(f)

best_node = None
best_time = float("inf")

for node_id, node_data in state.items():
    if node_id == "_meta":
        continue
    if node_data.get("test_passed"):
        btime = node_data.get("benchmark_time")
        if btime is not None and btime < best_time:
            best_time = btime
            best_node = node_data

if best_node:
    print(f"Best Node: {best_node['node_id']} with time {best_time}ns")
    with open("extension/src/vector_writer.rs", "w") as f:
        f.write(best_node["code_content"])
    print("Wrote best code to vector_writer.rs")
else:
    print("No passing nodes found!")
