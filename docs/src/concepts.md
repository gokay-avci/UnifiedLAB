# Concepts (plain English)

If you take nothing else from this page, take this:

> UnifiedLab is a graph scheduler with two memories:  
> an append-only *event history* and a queryable *checkpoint state*.

---

## Jobs, nodes, and flows

A **workflow** is a directed graph. Each **node** becomes a **job**.

A job is basically:
- *what to run* (config)
- *what it needs* (resources)
- *what it depends on* (other jobs)

The graph gives you “what must happen before what”.

---

## Coordinator vs workers

Think of this like a small lab:

- The **coordinator** is the lab manager.
  - keeps the whiteboard (the checkpoint)
  - hands out tasks
  - refuses nonsense and logs everything

- The **workers** are the researchers.
  - they ask for work
  - run it
  - report results back

In MPI terms:
- rank 0 = coordinator + worker (so the manager can also do some work)
- ranks 1..N = workers

---

## Events vs checkpoints

This is the heart of the design.

### Events (append-only)

Events are written to logs.
They are:
- easy to append
- easy to audit
- recoverable even after partial writes (the reader scans for a magic header)

Events are “what happened”.

### Checkpoints (SQLite state)

The checkpoint DB is:
- fast to query
- easy to summarise (perfect for the TUI)
- updated incrementally (UPSERTs instead of “rewrite the universe”)

Checkpoints are “what we currently believe to be true”.

---

## Why both?

Because clusters crash, filesystems hiccup, and humans press Ctrl+C.

- If you only store state: you lose the story (and debugging becomes archaeology).
- If you only store events: you can’t easily ask “what is currently running?”.

UnifiedLab keeps both, and tries to make recovery boring.

---

## Where dependencies live

Dependencies are part of the workflow graph:
- **hard dependencies**: must finish before a job runs
- **soft dependencies**: influence scheduling but aren’t an absolute gate
- **data flow edges**: optionally map parameters between nodes

You can learn the user-facing side in [Workflow blueprint (Draw.io)](workflow-dsl.md).
