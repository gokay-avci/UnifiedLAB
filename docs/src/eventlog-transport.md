# Event log & transport

UnifiedLab uses an append-only event log for durability and debuggability.

It’s intentionally defensive:
- every record has a magic header (`ULAB`)
- every record has a CRC32 checksum
- the reader can scan byte-by-byte to recover after partial writes

This is how you make “log files on weird filesystems” behave well enough for orchestration.

---

## What the event log stores

Each record is:
- a small binary envelope (kind, timestamps, ids, etc.)
- a JSON payload (flexible schema without recompiling the world)

This gives you:
- speed and size sanity from the envelope
- flexibility from JSON

---

## Why not “just JSON lines”?

Because corruption happens.
If a write is torn in half, JSON lines becomes “good luck”.
The magic header + CRC approach lets you resynchronise and keep going.

---

## Size limits

Records above 128MB are rejected to avoid accidental out-of-memory situations.
If you want to attach huge data, store it separately and reference it by path/hash.

---

## Where to look when debugging

- `root/events.log` — the coordinator’s global log
- `root/inbox/*.log` — incoming submissions / worker messages

If you see an inbox log appear after `deploy`, you know submission worked.
If the coordinator doesn’t react, the problem is in coordination/scheduling, not deployment.
