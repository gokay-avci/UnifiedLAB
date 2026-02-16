#!/usr/bin/env python3
import sys
import json
import os
import shutil
import subprocess
import logging
import warnings
import traceback
import io
import numpy as np
import pandas as pd

# ==========================================
# 0. CONFIGURATION
# ==========================================
os.environ["OMP_NUM_THREADS"] = "1"
os.environ["MKL_NUM_THREADS"] = "1"

PLOT_DIR = "plots"
DEBUG_DIR = "gulp_debug"
SNAPSHOT_CSV = "shim_snapshot.csv"
BOUNDS_DEF = [[400.0, 1800.0], [0.15, 0.55]]
CATLOW_TRUTH = (1428.5, 0.2945)

os.makedirs(PLOT_DIR, exist_ok=True)
os.makedirs(DEBUG_DIR, exist_ok=True)

# Silence standard logging to keep stdout clean for JSON
logging.basicConfig(level=logging.INFO, stream=sys.stderr, format='[ShimLog] %(message)s')
logger = logging.getLogger()
warnings.filterwarnings("ignore")

try:
    import torch
    import matplotlib.pyplot as plt
    from scipy.stats import qmc
    from autoemulate import AutoEmulate
    HAS_ML = True
except ImportError as e:
    HAS_ML = False
    ML_ERROR = str(e)

# ==========================================
# 1. GULP WORKER
# ==========================================
class GulpWorker:
    def __init__(self):
        self.gulp_exec = shutil.which("gulp")
        self.template = """optimise conp properties
title
MgO_Active_Learning
end
cell
4.212 4.212 4.212 90 90 90
frac
Mg core 0.0 0.0 0.0
O core 0.5 0.5 0.5
species
Mg core 2.0
O core -2.0
buck
Mg core O core {A:.4f} {rho:.4f} 0.0 0.0 10.0
O core O core 22764.0 0.149 27.88 0.0 10.0
coulomb
start
"""

    def run(self, A, rho, job_id):
        if not self.gulp_exec: raise RuntimeError("GULP executable not found")
        
        run_dir = os.path.join(DEBUG_DIR, job_id)
        os.makedirs(run_dir, exist_ok=True)
        gin_path = os.path.join(run_dir, "input.gin")
        
        with open(gin_path, "w") as f: f.write(self.template.format(A=A, rho=rho))

        try:
            res = subprocess.run([self.gulp_exec], stdin=open(gin_path, "r"), capture_output=True, text=True, timeout=45)
            
            if "error" in res.stdout.lower() or "STOP" in res.stdout:
                raise RuntimeError(f"GULP Internal Error:\n{res.stdout[-500:]}")

            for line in res.stdout.splitlines():
                if "Total lattice energy" in line:
                    return float(line.split()[4]), 100.0
            
            raise RuntimeError("Energy not found in output")
        except Exception as e:
            logger.warning(f"Sim Failed: {e}")
            raise

# ==========================================
# 2. DIAGNOSTIC PROBE
# ==========================================
class AutoEmulateProbe:
    def __init__(self):
        self.buf = io.StringIO()
        self.h = logging.StreamHandler(self.buf)
        self.l = logging.getLogger("autoemulate")
    def __enter__(self):
        self.l.setLevel(logging.DEBUG); self.l.addHandler(self.h)
        return self
    def __exit__(self, *args): self.l.removeHandler(self.h)
    def logs(self): return self.buf.getvalue()

# ==========================================
# 3. AGENT BRAIN
# ==========================================
class AgentBrain:
    def __init__(self, seed, bounds):
        self.seed = seed
        self.bounds_np = np.array(bounds)
        if HAS_ML:
            self.bounds = torch.tensor(bounds, dtype=torch.float32)
            torch.manual_seed(seed)
            np.random.seed(seed)

    def suggest(self, history, params):
        gen = params.get("gen_counter", 0)
        warm_size = params.get("warm_start_size", 100)
        batch = params.get("batch_size", 10)

        if gen == 0:
            return self._lhs(warm_size), "Warm_Start_LHS", "Genesis"

        X, y = [], []
        seen = set()
        
        for h in history:
            if h.get("status") in ["OK", "Completed"] and h.get("energy") is not None:
                if not np.isfinite(h["energy"]): continue
                k = tuple(np.round(h["candidate"], 5))
                if k not in seen:
                    X.append(h["candidate"]); y.append(h["energy"]); seen.add(k)
        
        self._snapshot(X, y)
        valid = len(X)

        if not HAS_ML or valid < 5:
            return self._lhs(batch), f"Fallback_Random(Data={valid})", "LowData"

        ae_logs = "No logs."
        try:
            with AutoEmulateProbe() as probe:
                try:
                    X_t = torch.tensor(X, dtype=torch.float32) + torch.randn(valid, 2) * 1e-5
                    y_t = torch.tensor(y, dtype=torch.float32)

                    subset = ["GaussianProcess", "RandomForest"]
                    if valid > 50: subset.append("RadialBasisFunctions")
                    
                    # --- COMPATIBILITY FIX ---
                    try:
                        # Try v1.0+ API
                        ae = AutoEmulate(X_t.numpy(), y_t.numpy().ravel(), model_subset=subset)
                    except TypeError:
                        # Fallback to v0.x API
                        ae = AutoEmulate(X_t.numpy(), y_t.numpy().ravel())
                        # In v0.x, model_subset goes into setup()
                    
                    # Setup call (Try both ways just in case)
                    try:
                        ae.setup(X_t.numpy(), y_t.numpy().ravel(), fold=min(5, valid), model_subset=subset)
                    except TypeError:
                        # v1.0 already has subset, or v0.x strict params
                        ae.setup(X_t.numpy(), y_t.numpy().ravel(), fold=min(5, valid))

                    ae.compare()
                    model = ae.refit_best()
                    name = getattr(model, "best_model_name", "Model")

                    pool = self._lhs(5000)
                    mu, sig = self._pred(model, pool)
                    lcb = mu - 1.96 * sig
                    
                    inds = np.argsort(lcb)[:batch]
                    cands = np.array(pool)[inds].tolist()
                    
                    self._plot(model, X_t, gen, name)
                    return cands, f"AL_{name}", f"Success_{name}"

                except Exception as inner:
                    ae_logs = probe.logs()
                    raise inner

        except Exception as e:
            tb = traceback.format_exc()
            error_report = f"ERROR: {str(e)}\n\nTRACE:\n{tb}\n\nINTERNAL LOGS:\n{ae_logs}"
            sys.stderr.write(f"\n[CRASH GEN {gen}]\n{error_report}\n")
            return self._lhs(batch), "Fallback_Crash", error_report

    def _lhs(self, n):
        if HAS_ML:
            s = qmc.LatinHypercube(d=2, seed=self.seed).random(n)
            return qmc.scale(s, self.bounds_np[:,0], self.bounds_np[:,1]).tolist()
        return (self.bounds_np[:,0] + np.random.rand(n, 2) * (self.bounds_np[:,1]-self.bounds_np[:,0])).tolist()

    def _pred(self, m, p):
        p_t = np.array(p)
        cands = [m, getattr(m, "best_model", None), getattr(m, "model", None)]
        for c in cands:
            if c:
                try: return c.predict(p_t, return_std=True)
                except:
                    try: 
                        mu = c.predict(p_t)
                        return mu, np.ones(len(mu))*np.std(mu)
                    except: continue
        return np.zeros(len(p)), np.ones(len(p))

    def _snapshot(self, X, y):
        try: pd.DataFrame({"A":[x[0] for x in X], "rho":[x[1] for x in X], "energy": y}).to_csv(SNAPSHOT_CSV, index=False)
        except: pass

    def _plot(self, m, X, g, n):
        try:
            plt.figure(figsize=(10,8))
            xi = np.linspace(self.bounds_np[0,0], self.bounds_np[0,1], 50)
            yi = np.linspace(self.bounds_np[1,0], self.bounds_np[1,1], 50)
            Xi, Yi = np.meshgrid(xi, yi)
            G = np.column_stack([Xi.ravel(), Yi.ravel()])
            mu, _ = self._pred(m, G)
            Zi = mu.reshape(Xi.shape)
            plt.contourf(Xi, Yi, Zi, levels=25, cmap='viridis')
            plt.colorbar()
            plt.scatter(X[:,0], X[:,1], c='w', edgecolors='k', s=20)
            plt.scatter(CATLOW_TRUTH[0], CATLOW_TRUTH[1], c='r', marker='*', s=200)
            plt.title(f"Gen {g} ({n})")
            plt.savefig(f"{PLOT_DIR}/gen_{g}.png"); plt.close()
        except: pass

# ==========================================
# 4. MAIN INTEROP
# ==========================================
def main():
    try:
        raw = sys.stdin.read()
        if not raw: return
        req = json.loads(raw)
        
        task = req.get("task")
        payload = req.get("payload", {})
        config = req.get("config", {}) 
        resp = {"status": "SUCCESS"}
        
        if task == "CALCULATE":
            sim = GulpWorker()
            c = payload.get("candidate")
            e, t = sim.run(c[0], c[1], payload.get("job_id", "0"))
            resp["data"] = {"energy": e, "t_total_ms": t}
            
        elif task == "SUGGEST":
            hist = payload.get("history", [])
            if not hist:
                X, y = payload.get("X_known", []), payload.get("y_known", [])
                hist = [{"candidate": x, "energy": v, "status": "OK"} for x, v in zip(X, y)]

            seed = config.get("random_seed", 77)
            bounds = payload.get("bounds", BOUNDS_DEF)
            
            brain = AgentBrain(seed, bounds)
            c, r, meta = brain.suggest(hist, payload)
            
            resp["data"] = {"raw_candidates": c, "reasoning": r, "model_metadata": meta}
            
        print(json.dumps(resp))
    except Exception as e:
        print(json.dumps({"status": "FAILURE", "error": f"{e}\n{traceback.format_exc()}"}))

if __name__ == "__main__":
    main()