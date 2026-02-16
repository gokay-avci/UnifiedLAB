import sys
import json
import os
import traceback

def main():
    if len(sys.argv) < 3:
        sys.stderr.write("Usage: cli.py [mode] [engine] [work_dir]\n")
        sys.exit(1)

    mode = sys.argv[1]   # "write" or "parse"
    engine = sys.argv[2] # "gulp", "vasp", etc.
    work_dir = sys.argv[3]

    try:
        if mode == "write":
            # 1. Read Job JSON
            raw_in = sys.stdin.read()
            data = json.loads(raw_in)
            
            # 2. Write Dummy Input File
            # In real life: Pymatgen writes INCAR/POSCAR here
            input_file = os.path.join(work_dir, "simulation.input")
            with open(input_file, "w") as f:
                f.write(f"# Mock Input for {engine}\n")
                f.write(f"# Params: {json.dumps(data.get('config', {}))}\n")
                
            # No output to stdout for 'write' mode (Rust ignores it)

        elif mode == "parse":
            # 1. Read/Mock Output
            # In real life: Pymatgen parses OUTCAR/vasprun.xml here
            
            # 2. Construct Response (Matches Rust CalculationResult)
            response = {
                "energy": -123.456,  # Mock Energy
                "forces": None,
                "stress": None,
                "t_total_ms": 500.0, # Mock timing
                "final_structure": None,
                
                # Rust hydration fallback
                "provenance": {     
                    "execution_host": "localhost",
                    "start_time": "2024-01-01T00:00:00Z",
                    "end_time": "2024-01-01T00:00:00Z",
                    "binary_hash": None,
                    "exit_code": 0,
                    "sandbox_info": "mock_sandbox"
                },
                "next_generation": None
            }
            print(json.dumps(response))

    except Exception as e:
        sys.stderr.write(f"[CLI Error] {traceback.format_exc()}\n")
        # For parse mode, we must print valid JSON or Rust panics on deserialize.
        # But if we exit(1), Rust catches the exit code anyway.
        sys.exit(1)

if __name__ == "__main__":
    main()