import sys
import json
import numpy as np
import traceback

# ==========================================
# PHYSICS LOGIC (Lennard-Jones)
# ==========================================
def compute_lj(positions, cell, epsilon=5.0, sigma=2.5):
    """
    Calculates Energy (eV) and Forces (eV/A) for noble gases.
    O(N^2) pairwise interaction.
    """
    n = len(positions)
    energy = 0.0
    forces = np.zeros_like(positions)
    
    # Simple Minimum Image Convention (Orthorhombic)
    box_diag = np.diag(cell) if cell is not None else None
    
    for i in range(n):
        for j in range(i + 1, n):
            r_vec = positions[j] - positions[i]
            
            if box_diag is not None:
                r_vec = r_vec - box_diag * np.round(r_vec / box_diag)
            
            r = np.linalg.norm(r_vec)
            if r < 1e-3: continue # Prevent singularity

            sr6 = (sigma / r) ** 6
            sr12 = sr6 ** 2
            
            # Potential Energy: 4*eps * (sr12 - sr6)
            e_pair = 4 * epsilon * (sr12 - sr6)
            energy += e_pair
            
            # Force: 24*eps/r * (2*sr12 - sr6) * (r_vec/r)
            f_mag = (24 * epsilon / r) * (2 * sr12 - sr6)
            f_vec = f_mag * (r_vec / r)
            
            forces[j] += f_vec
            forces[i] -= f_vec
            
    return energy, forces

# ==========================================
# DAEMON LOOP
# ==========================================
def main():
    # 1. Handshake (Tell Rust we are alive)
    print("READY", flush=True)

    # 2. Event Loop
    for line in sys.stdin:
        try:
            req = json.loads(line)
            
            # Extract Data
            structure = req["structure"]
            pos = np.array([a["position"] for a in structure["atoms"]])
            
            # Handle Lattice (Optional in ULO)
            cell = None
            if structure.get("lattice"):
                cell = np.array(structure["lattice"]["vectors"])

            # Compute
            e, f = compute_lj(pos, cell)
            
            # Response Schema (Matches JanusResponse in Rust)
            response = {
                "energy": e,
                "forces": f.tolist(),
                "stress": None,
                "error": None
            }
            print(json.dumps(response), flush=True)

        except Exception as e:
            # Log full trace to stderr (Visible in Rust logs)
            sys.stderr.write(f"[Janus Error] {traceback.format_exc()}\n")
            
            # Send clean error to stdout (Prevents Rust panic)
            err_resp = {
                "energy": None, 
                "forces": None, 
                "stress": None,
                "error": str(e)
            }
            print(json.dumps(err_resp), flush=True)

if __name__ == "__main__":
    main()