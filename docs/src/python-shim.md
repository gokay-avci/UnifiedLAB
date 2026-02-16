# Python shim contract

UnifiedLab can delegate “smart bits” to a Python shim.

This is intentionally blunt:
- Rust handles orchestration, durability, scheduling, and safety.
- Python handles “whatever weird thing we’re experimenting with this week”.

The shim contract is designed to be stable even if your internal ML code changes.

---

## What the shim is for

Typical uses:
- “Suggest next candidates” logic (active learning / evolutionary search)
- Calling external tools that are easiest to control from Python
- Prototyping before porting into Rust (if ever)

---

## The two workloads

UnifiedLab distinguishes between:

- `standard_process`  
  Traditional physics codes (GULP/VASP/LAMMPS etc.) launched as subprocesses.

- `auto_emulate`  
  “Agent mode” where the shim proposes and evaluates candidates in a loop.

---

## Message shape (high level)

Rust sends a JSON request to the shim and expects a JSON response back.

A request tells the shim:
- what workload type it is
- what task to do (`CALCULATE` vs `SUGGEST`)
- any parameters / context it needs

A response returns:
- either a calculation result
- or a suggestion payload (new candidates)

If you’re implementing the shim, the best strategy is:

1) print *nothing* to stdout except the JSON response  
2) log to stderr or a file  
3) validate inputs and fail loudly if something is missing

---

## Where the shim lives

By convention:

```text
scripts/active_learning_shim.py
```

UnifiedLab calls this as a subprocess.

If you want to support multiple shims (e.g. per project), you can keep separate scripts and choose via config/params.
The key is: keep the contract stable.

---

## Practical advice (what I do)

- Start with a shim that returns a **fixed** response (so you can test plumbing).
- Once the plumbing is solid, add real logic.
- Use the checkpoint DB as your source of truth for what jobs ran and what happened.

If you want, I can generate a minimal shim template that:
- supports `CALCULATE` and `SUGGEST`
- validates payloads
- produces deterministic output for testing
