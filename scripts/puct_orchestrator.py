import os
import sys
import json
import math
import subprocess
import shutil
import asyncio
from typing import List, Dict, Optional
from pydantic import BaseModel, Field

try:
    from google import genai
    from google.genai import types
except ImportError:
    print("Error: google-genai is not installed. Run: pip install google-genai")
    sys.exit(1)

# PUCT Configuration
C_PUCT = 1.0
MAX_EXPANSIONS_PER_NODE = 8
EVAL_TIMEOUT_SECS = 60

class OptimizationCandidate(BaseModel):
    patch_code: str = Field(description="The complete, optimized Rust file content.")
    strategy: str = Field(description="A brief description of the optimization strategy used.")
    prior_prob: float = Field(description="Your confidence (0.0 to 1.0) that this optimization will improve performance while remaining mathematically correct.")

class PUCTNode:
    def __init__(self, node_id: str, code_content: str, parent_id: Optional[str] = None, prior_prob: float = 1.0):
        self.node_id = node_id
        self.code_content = code_content
        self.parent_id = parent_id
        
        self.benchmark_time: Optional[float] = None
        self.test_passed: bool = False
        
        self.visits = 0
        self.total_value = 0.0
        self.prior_prob = prior_prob
        self.children: List[str] = []

    def q_value(self) -> float:
        if self.visits == 0:
            return 0.0
        return self.total_value / self.visits

    def puct_score(self, parent_visits: int) -> float:
        q = self.q_value()
        u = C_PUCT * self.prior_prob * (math.sqrt(parent_visits) / (1 + self.visits))
        return q + u

    def to_dict(self):
        return {
            "node_id": self.node_id,
            "parent_id": self.parent_id,
            "benchmark_time": self.benchmark_time,
            "test_passed": self.test_passed,
            "visits": self.visits,
            "total_value": self.total_value,
            "prior_prob": self.prior_prob,
            "children": self.children,
            "code_content": self.code_content
        }

    @classmethod
    def from_dict(cls, data: dict):
        node = cls(data["node_id"], data["code_content"], data.get("parent_id"), data.get("prior_prob", 1.0))
        node.benchmark_time = data.get("benchmark_time")
        node.test_passed = data.get("test_passed", False)
        node.visits = data.get("visits", 0)
        node.total_value = data.get("total_value", 0.0)
        node.children = data.get("children", [])
        return node

class OrchestratorState(BaseModel):
    total_evaluations: int = 0
    best_time: float = float('inf')
    steps_without_improvement: int = 0


class PUCTOrchestrator:
    def __init__(self, state_file: str, target_file: str):
        self.state_file = state_file
        self.target_file = target_file
        self.nodes: Dict[str, PUCTNode] = {}
        
        client_kwargs = {}
        if not os.environ.get("GEMINI_API_KEY"):
            client_kwargs = {
                "vertexai": True,
                "location": os.environ.get("VERTEX_LOCATION", "eu-west1")
            }
        
        try:
            self.client = genai.Client(**client_kwargs)
            self.model_id = os.environ.get("VERTEX_MODEL_ID", "gemini-2.5-flash")
            print(f"Initialized GenAI Client with model: {self.model_id}")
        except Exception as e:
            print(f"Failed to initialize GenAI Client: {e}")
            sys.exit(1)
            
        self.load_state()

    def load_state(self):
        self.state_meta = OrchestratorState()
        if os.path.exists(self.state_file):
            with open(self.state_file, 'r') as f:
                data = json.load(f)
                
                # Load metadata if present
                if "_meta" in data:
                    self.state_meta = OrchestratorState(**data["_meta"])
                    del data["_meta"]
                    
                for node_id, node_data in data.items():
                    self.nodes[node_id] = PUCTNode.from_dict(node_data)
            print(f"Loaded {len(self.nodes)} nodes from state file.")
        else:
            print("No state file found. Starting fresh.")
            self._init_root()

    def save_state(self):
        data = {n_id: node.to_dict() for n_id, node in self.nodes.items()}
        data["_meta"] = self.state_meta.model_dump()
        with open(self.state_file, 'w') as f:
            json.dump(data, f, indent=2)

    def _init_root(self):
        with open(self.target_file, 'r') as f:
            base_code = f.read()
        
        root = PUCTNode(node_id="root", code_content=base_code)
        self.nodes["root"] = root
        
        time_us = self.evaluate_node(root)
        if time_us:
            root.benchmark_time = time_us
            root.test_passed = True
            
        self.save_state()

    async def self_correct_node(self, node: PUCTNode, error_log: str) -> bool:
        print(f"  Attempting iterative self-correction for {node.node_id}...")
        prompt = f"""
You are an expert Rust performance engineer. You recently proposed an optimization strategy for a critical inner loop, but it failed to compile or pass tests.

Here is your currently failing Rust code:
```rust
{node.code_content}
```

Here is the compiler or test output detailing the exact errors:
```text
{error_log}
```

Please fix the errors in your code. Return the ENTIRE modified, corrected Rust file.
Ensure the function signatures remain exactly identical to the original baseline.
"""
        try:
            response = await self.client.aio.models.generate_content(
                model=self.model_id,
                contents=prompt,
                config=types.GenerateContentConfig(
                    temperature=0.2,
                    response_mime_type="application/json",
                    response_schema=OptimizationCandidate,
                ),
            )
            if not response.text:
                return False
                
            cand = OptimizationCandidate.model_validate_json(response.text)
            print(f"  Self-correction generated new code (Strategy: {cand.strategy}).")
            node.code_content = cand.patch_code
            return True
        except Exception as e:
            print(f"  Self-correction LLM call failed: {e}")
            return False

    def evaluate_node(self, node: PUCTNode) -> Optional[float]:
        print(f"Evaluating node: {node.node_id}")
        
        backup_file = f"{self.target_file}.bak"
        if not os.path.exists(backup_file):
            shutil.copy(self.target_file, backup_file)
            
        try:
            max_retries = 2
            for attempt in range(max_retries + 1):
                with open(self.target_file, 'w') as f:
                    f.write(node.code_content)
                    
                print(f"  Running tests (Attempt {attempt+1}/{max_retries+1})...")
                test_res = subprocess.run(
                    ["cargo", "test", "-p", "zarrduck_extension", "test_populate_coordinate_batch"],
                    capture_output=True, text=True, cwd="extension", timeout=EVAL_TIMEOUT_SECS
                )
                
                if test_res.returncode == 0:
                    break # Tests passed!
                    
                error_log = test_res.stderr if "error:" in test_res.stderr or "error[" in test_res.stderr else test_res.stdout
                print(f"  Tests FAILED. Length of error log: {len(error_log)} chars")
                
                if attempt < max_retries:
                    # Try to fix it!
                    success = asyncio.run(self.self_correct_node(node, error_log))
                    if not success:
                        print("  Self-correction failed to generate new code. Rejecting candidate.")
                        return None
                else:
                    print("  Max retries reached. Rejecting candidate.")
                    return None
                
            print("  Running benchmark...")
            bench_res = subprocess.run(
                ["cargo", "bench", "-p", "zarrduck_extension"],
                capture_output=True, text=True, cwd="extension", timeout=EVAL_TIMEOUT_SECS
            )
            
            import re
            match = re.search(r'time:\s+\[.*?([0-9.]+) µs .*?\]', bench_res.stdout)
            if match:
                time_us = float(match.group(1))
                print(f"  Benchmark result: {time_us} µs")
                return time_us
            else:
                print("  Failed to parse benchmark time.")
                return None
                
        except subprocess.TimeoutExpired:
            print("  Evaluation timed out.")
            return None
        except Exception as e:
            print(f"  Evaluation error: {e}")
            return None
        finally:
            shutil.copy(backup_file, self.target_file)

    def select(self) -> str:
        current = "root"
        while self.nodes[current].children:
            best_score = -float('inf')
            best_child = None
            parent_visits = self.nodes[current].visits
            
            for child_id in self.nodes[current].children:
                child = self.nodes[child_id]
                score = child.puct_score(parent_visits)
                if score > best_score:
                    best_score = score
                    best_child = child_id
                    
            if best_child:
                current = best_child
            else:
                break
                
        return current

    async def generate_single_candidate(self, prompt: str, index: int) -> Optional[OptimizationCandidate]:
        print(f"  Spawning concurrent LLM call for candidate {index} (Semaphore Acquired)...")
        try:
            response = await self.client.aio.models.generate_content(
                model=self.model_id,
                contents=prompt,
                config=types.GenerateContentConfig(
                    temperature=0.8,
                    response_mime_type="application/json",
                    response_schema=OptimizationCandidate,
                ),
            )
            if not response.text:
                return None
            return OptimizationCandidate.model_validate_json(response.text)
        except Exception as e:
            print(f"  LLM generation {index} failed: {e}")
            return None

    async def expand_async(self, parent_id: str) -> List[str]:
        print(f"Expanding node: {parent_id}")
        parent_node = self.nodes[parent_id]
        
        dimensions = [
            "Instruction Level (e.g., bit-twiddling hacks, branchless arithmetic, lookup tables)",
            "Vectorization (e.g., explicit std::simd intrinsics, auto-vectorization friendly hints like chunking)",
            "Memory Hierarchy (e.g., array-of-structs vs struct-of-arrays optimizations, pre-fetching hints, cache-line alignment)",
            "Compiler Hints (e.g., #[inline(always)], #[cold], assume macro assertions)",
            "Loop Level (e.g., extreme loop unrolling, loop fusion, induction variable elimination)",
            "Bounds Checks (e.g., unsafe get_unchecked, iterator usage instead of indexing)",
            "Data Locality (e.g., blocking, tiling, strided access patterns)",
            "Register Pressure (e.g., minimizing local variables, reusing accumulators)"
        ]
        
        # Limit concurrent API calls to avoid triggering Vertex AI rate limits
        # which can cause the SDK to silently hang during exponential backoff.
        # Increased to 4 to balance speed and rate limits.
        sem = asyncio.Semaphore(4)
        
        async def _generate_with_dim(i: int):
            print(f"  Queuing candidate {i} for generation...")
            async with sem:
                import random
                dim = dimensions[i % len(dimensions)]
                prompt = f"""
You are an expert Rust performance engineer optimizing a critical mathematical inner loop in a database extraction engine.
We are using an Empirical Research Assistance (ERA) tree search to find the fastest mathematically correct implementation.

Here is the current fastest implementation (Benchmark: {parent_node.benchmark_time} µs):

```rust
{parent_node.code_content}
```

CRITICAL INSTRUCTION:
Make exactly ONE ATOMIC TWEAK to the code to make it faster without breaking correctness.
Do NOT attempt to rewrite the whole file or apply multiple distinct optimizations at once.
Make one small change.

For this specific iteration, focus exclusively on this optimization dimension:
**{dim}**

Ensure the function signatures remain exactly identical. Return the ENTIRE modified file.
"""
                return await self.generate_single_candidate(prompt, i)

        tasks = [_generate_with_dim(i) for i in range(MAX_EXPANSIONS_PER_NODE)]
        results = await asyncio.gather(*tasks)
        
        valid_children = []
        for i, cand in enumerate(results):
            if cand is None:
                continue
                
            node_id = f"{parent_id}_child_{i+1}"
            print(f"  Generated candidate: {node_id} (Strategy: {cand.strategy})")
            
            new_node = PUCTNode(
                node_id=node_id, 
                code_content=cand.patch_code, 
                parent_id=parent_id,
                prior_prob=cand.prior_prob
            )
            
            self.nodes[node_id] = new_node
            self.nodes[parent_id].children.append(node_id)
            valid_children.append(node_id)
            
        self.save_state()
        return valid_children

    def backpropagate(self, node_id: str, time_us: Optional[float]):
        root_time = self.nodes["root"].benchmark_time or 170.0
        
        value = -1.0
        if time_us is not None:
            speedup_ratio = (root_time - time_us) / root_time
            if speedup_ratio >= 0:
                value = min(1.0, speedup_ratio)
            else:
                value = max(-1.0, speedup_ratio)
                
        current = node_id
        while current is not None:
            node = self.nodes[current]
            node.visits += 1
            node.total_value += value
            current = node.parent_id
            
        self.save_state()

    def run_step(self) -> bool:
        leaf_id = self.select()
        
        if not self.nodes[leaf_id].children:
            # Run async expansion
            new_children = asyncio.run(self.expand_async(leaf_id))
            if not new_children:
                print("Expansion failed to generate valid candidates. Terminating step.")
                self.backpropagate(leaf_id, None)
                return True
            child_to_eval = new_children[0]
        else:
            child_to_eval = leaf_id
            
        node_to_eval = self.nodes[child_to_eval]
        
        time_us = self.evaluate_node(node_to_eval)
        node_to_eval.benchmark_time = time_us
        self.state_meta.total_evaluations += 1
        
        if time_us:
            node_to_eval.test_passed = True
            if time_us < self.state_meta.best_time:
                print(f"\n🎉 NEW BEST TIME: {time_us} µs! (Previous: {self.state_meta.best_time} µs) 🎉")
                self.state_meta.best_time = time_us
                self.state_meta.steps_without_improvement = 0
            else:
                self.state_meta.steps_without_improvement += 1
        else:
            self.state_meta.steps_without_improvement += 1
            
        self.backpropagate(child_to_eval, time_us)
        return True

    def run_loop(self, max_evaluations: int = 1000, max_stagnation: int = 150):
        print(f"\n🚀 Starting MCTS Optimization Loop 🚀")
        print(f"Termination Conditions: {max_evaluations} max evaluations OR {max_stagnation} steps without improvement.\n")
        
        while True:
            if self.state_meta.total_evaluations >= max_evaluations:
                print(f"\n🛑 Terminating: Reached maximum evaluation budget of {max_evaluations}.")
                break
                
            if self.state_meta.steps_without_improvement >= max_stagnation:
                print(f"\n🛑 Terminating: Stagnated. No improvement in {max_stagnation} consecutive evaluations.")
                break
                
            print(f"\n--- [ Evaluation {self.state_meta.total_evaluations} / {max_evaluations} ] ---")
            print(f"Current Best Time: {self.state_meta.best_time if self.state_meta.best_time != float('inf') else 'None'} µs")
            print(f"Stagnation Counter: {self.state_meta.steps_without_improvement} / {max_stagnation}")
            print(f"Nodes in Tree: {len(self.nodes)}\n")
            
            self.run_step()
            
        print("\n🏆 --- FINAL OPTIMIZATION LEADERBOARD --- 🏆")
        best_nodes = sorted(
            [n for n in self.nodes.values() if n.test_passed and n.benchmark_time is not None],
            key=lambda x: x.benchmark_time
        )[:5]
        
        for i, n in enumerate(best_nodes):
            print(f"{i+1}. Node {n.node_id} - {n.benchmark_time} µs (Visits: {n.visits})")

if __name__ == "__main__":
    orchestrator = PUCTOrchestrator(
        state_file="extension/candidates/mcts_state.json",
        target_file="extension/src/vector_writer.rs"
    )
    
    # Init the best time if we loaded from state
    if orchestrator.state_meta.best_time == float('inf'):
        best = float('inf')
        for node in orchestrator.nodes.values():
            if node.test_passed and node.benchmark_time:
                best = min(best, node.benchmark_time)
        orchestrator.state_meta.best_time = best

    orchestrator.run_loop(max_evaluations=1000, max_stagnation=150)

