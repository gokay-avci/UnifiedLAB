# How to use UnifiedLab

UnifiedLab has a very deliberate “shape”:

1) **You start a service** (`start`) inside an allocation (or locally for dev).
2) **You deploy a blueprint** (`deploy`) which becomes a submission event.
3) **The coordinator schedules work**, and workers execute tasks.
4) **Everything important ends up in two places:**
   - `checkpoint.db` (queryable truth)
   - `events.log` / `inbox/*.log` (append-only history)

If you keep that in your head, the rest of the system becomes predictable.

---

## Typical development loop (my favourite)

```bash
# terminal A
unifiedlab start --root ./scratch --force-local

# terminal B
unifiedlab deploy --root ./scratch --file ./experiments/experiment.drawio --params '{"gen_limit": 5}'

# terminal C (optional)
unifiedlab tui --checkpoint ./scratch/checkpoint.db
```

When something looks odd, I go in this order:
1) TUI view (state summary)
2) `events.log` (history)
3) `inbox/*.log` (who said what)

---

## What `deploy` really does

This is important:

> `deploy` is an **architect**, not an executor.

It parses your blueprint, turns nodes into jobs, adds dependency edges, and then writes a single submission payload into the inbox.
After that, the coordinator takes over.

That means:
- You can deploy from a different machine, as long as it can write to the same root.
- You can “stage” workflows by generating inbox payloads without running compute.

---

## What `start` really does

`start` is a node service:
- Rank 0: coordinator + worker
- Other ranks: workers only

Workers periodically ask the coordinator for work, and the coordinator grants tasks according to:
- dependencies (hard + soft)
- resources requested
- fairness / availability

See the deeper architecture tour here: [Under the bonnet](architecture.md).
