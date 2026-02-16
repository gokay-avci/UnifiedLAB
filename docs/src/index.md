# UnifiedLab

UnifiedLab is my attempt to make **HPC “active learning” workflows feel boring** — in the good way.

You draw a workflow as a **graph** (in Draw.io), deploy it, and then a coordinator hands out work to workers across an allocation.
Under the hood it’s a very pragmatic design:

- **Events** are append-only (cheap, auditable, recoverable).
- **State** is a SQLite checkpoint (fast to query, perfect for a TUI).
- **Work** is “rented out” to workers (so the coordinator stays sane).

If you’ve ever stitched together *Slurm scripts + Python glue + CSVs + hope*… this is for you.

---

## What you can do today

- Run a node service locally (for development) with `--force-local`.
- Deploy a Draw.io blueprint into an inbox (it becomes a job submission event).
- Watch what’s happening using the built-in **TUI dashboard**.

> **Design principle:** UnifiedLab tries hard to fail loudly and explain itself.  
> I’d rather throw a sharp error than silently do the wrong thing on a cluster.

---

## The three commands you’ll actually use

```bash
# 1) Start the service (coordinator + worker on rank 0, workers on other ranks)
unifiedlab start --root ./scratch --force-local

# 2) Deploy a blueprint (drops a submission event into the inbox)
unifiedlab deploy --root ./scratch --file ./experiments/experiment.drawio --params '{"gen_limit": 50}'

# 3) Monitor
unifiedlab tui --checkpoint ./scratch/checkpoint.db
```

---

## Where to go next

- **Start here:** [Getting started](getting-started.md)
- Want the “mental model” first? [Concepts (plain English)](concepts.md)
- Want to plug in your own ML/physics? [Python shim contract](python-shim.md)
