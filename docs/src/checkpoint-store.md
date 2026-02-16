# Checkpoint store (SQLite)

UnifiedLab keeps a SQLite database as its “current truth”.
The TUI reads from it, and the coordinator updates it incrementally.

The design is a hybrid:
- fast fields (status, timestamps, IDs) are columns
- structured blobs (configs, structures) are JSON

This avoids rewriting giant “world snapshots” every tick.

---

## What’s in the DB

There are three core tables:

- `meta`  
  Key/value store for global metadata (schema version, etc.)

- `workers`  
  One row per worker with cores/tasks and a last-seen timestamp

- `jobs`  
  One row per job including:
  - status
  - timestamps
  - JSON blobs (config + structure)

---

## Why UPSERT matters

In a live system, you want:
- small writes
- idempotent updates
- no full-table rewrites

So updates are done as incremental upserts.
That’s what keeps the TUI responsive and prevents the coordinator from becoming an I/O bomb.

---

## Inspecting the DB manually

Sometimes you just want to see the raw truth:

```bash
sqlite3 ./scratch/checkpoint.db
.tables
SELECT status, count(*) FROM jobs GROUP BY status;
```

If you plan to build external dashboards, this DB is the cleanest integration point.
