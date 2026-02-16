# Development notes

This project is built to evolve.

Here are the house rules I recommend (and try to follow):

---

## My priorities

1) Correctness and recoverability (especially on HPC filesystems)
2) Explainable behaviour (logs + deterministic decisions where possible)
3) Performance *after* the above (but still: we’re in Rust)

---

## How to iterate safely

- Add a tiny blueprint test (3 nodes)
- Run in laptop mode with `--force-local`
- Validate state changes in the TUI and via SQL
- Only then scale out to cluster allocations

---

## Code structure (mental map)

- `core` — foundational types (jobs, structures, configs)
- `workflow` — graph model and blueprint import
- `marketplace` — coordinator scheduling logic
- `checkpoint` — SQLite state store
- `eventlog` — append-only event store
- `transport` — message plumbing
- `resources` — cluster/topology detection
- `tui` — monitoring UI
- `interop` / `codes` — Python shim contract + external driver logic

---

## Testing ideas

If you want to harden this project, the best tests are:
- event log corruption recovery
- checkpoint upsert correctness
- dependency scheduling correctness
- deterministic “toy workflows” end-to-end

If you want, I can scaffold a minimal `tests/` suite that exercises these pieces.
