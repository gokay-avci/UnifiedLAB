# Roadmap

This is intentionally aspirational. The core is already useful; the rest is about making it delightful.

---

## Near-term (quality of life)

- A “gold standard” example blueprint repository
- A minimal Python shim template with validation
- Better error messages for blueprint import failures
- Tag-aware scheduling (GPU/highmem routing)

---

## Mid-term (real-world HPC pain)

- Slurm job-step awareness for multi-job allocations
- Better locality controls (don’t scatter when you don’t need to)
- Smarter backpressure (avoid flooding the filesystem)
- Optional compression for event payloads

---

## Longer-term (fun stuff)

- Export workflow state to a web dashboard
- Pluggable schedulers (swap marketplace policy)
- Multi-fidelity active learning patterns as first-class nodes
