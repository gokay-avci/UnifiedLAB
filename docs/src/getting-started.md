# Getting started

This page is intentionally practical. The fastest way to understand UnifiedLab is to run it once and watch the state move.

---

## Folder layout (what appears under `--root`)

When you run UnifiedLab with `--root ./scratch`, you should expect something like:

```text
scratch/
  checkpoint.db      # durable state (SQLite)
  events.log         # global append-only event log (coordinator)
  inbox/             # job submissions and worker messages (append-only logs)
```

- **`deploy`** writes a job submission event into `inbox/…`
- **`start`** (rank 0) acts as coordinator and consumes `inbox/…`
- **workers** request work, run tasks, and report completion back as events

---

## Quickstart: laptop mode

Laptop mode is for development and sanity-checking. By default, UnifiedLab refuses to run locally unless you explicitly say you mean it.

### 1) Start the service

```bash
unifiedlab start --root ./scratch --force-local
```

What happens:
- It detects a **Local** environment and starts in single-process mode.
- Rank 0 becomes **coordinator + worker**.

### 2) Deploy a blueprint

In a second terminal:

```bash
unifiedlab deploy --root ./scratch --file ./experiments/experiment.drawio --params '{"gen_limit": 10}'
```

This does *not* directly run jobs.
It drops a **submission payload** into the inbox, and the coordinator picks it up.

### 3) Watch the world

```bash
unifiedlab tui --checkpoint ./scratch/checkpoint.db
```

---

## Quickstart: cluster mode

Cluster mode means “one coordinator, many workers”.

### Slurm (typical)

Inside an allocation, you’d launch something like:

```bash
srun --ntasks=8 unifiedlab start --root $SCRATCH/unifiedlab
```

UnifiedLab detects rank/world-size from standard variables (Slurm or MPI), so:
- rank 0 = coordinator + worker
- ranks 1..N = workers

Then, from anywhere that can write to the same root (often the login node):

```bash
unifiedlab deploy --root $SCRATCH/unifiedlab --file experiment.drawio
```

> If you’re not sure whether your cluster exposes the shared filesystem the way you think it does, start with a tiny allocation and watch the inbox + checkpoint behaviour.

---

## Safety switch: `--force-local`

UnifiedLab refuses to run on a workstation by default. That’s on purpose.

If you’re on a laptop and you *want* it, say so:

```bash
unifiedlab start --force-local
```

---

## Next step

- Learn the mental model: [Concepts (plain English)](concepts.md)
- Learn how to describe a workflow: [Workflow blueprint (Draw.io)](workflow-dsl.md)
