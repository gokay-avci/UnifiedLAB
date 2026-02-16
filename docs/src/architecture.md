# Under the bonnet

This page explains the guts of UnifiedLab, but still in human language.

If you’re the sort of person who reads code for fun, you’ll enjoy it.
If you’re not, treat this as a map.

---

## The pipeline in one picture

```text
            (1) deploy
              |
              v
     root/inbox/worker_architect_*.log
              |
              v
   +---------------------+
   | Coordinator (rank0) |
   |  - consumes inbox   |
   |  - writes events    |
   |  - updates DB       |
   +---------------------+
        |           |
  (2) grants     (3) logs state
        |           |
        v           v
   Workers       checkpoint.db
 (rank 1..N)      events.log
```

---

## Why the design looks like this

HPC is not Kubernetes. Filesystems do odd things. Jobs die. Nodes get pre-empted.

So UnifiedLab chooses:
- **append-only logs** for “facts”
- **SQLite** for “current state”
- **defensive parsing** everywhere (magic headers + CRC for events)

The goal is: *if something goes wrong, you can recover or at least explain the failure.*

---

## Key components

### 1) Transport

Transport is the “nervous system”.
In the debug edition you have a file-based transport that moves messages through event logs.
It’s boring on purpose.

See: [Event log & transport](eventlog-transport.md)

### 2) Marketplace scheduler

The coordinator is essentially a scheduler:
- workers ask for work
- coordinator grants tasks when dependencies and resources allow

See: [Marketplace scheduler](marketplace.md)

### 3) Checkpoint store

The checkpoint DB is a hybrid model:
- hot fields stored as columns (status, timestamps)
- complex data stored as JSON blobs

This keeps queries fast without forcing you to pre-normalise everything.

See: [Checkpoint store (SQLite)](checkpoint-store.md)

### 4) Resource detection

UnifiedLab detects:
- cluster type (Slurm/PBS/LSF/Local)
- rank/world-size
- cores and memory

This matters for scheduling and for wrapping sub-commands into job steps.

See: [Resource & topology detection](resources-topology.md)
