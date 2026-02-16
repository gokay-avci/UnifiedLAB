# Marketplace scheduler

The “marketplace” is the coordinator’s scheduling brain.

Workers don’t get assigned tasks by magic; they ask:

1) **Work request** (worker → coordinator)  
2) **Work grant** (coordinator → worker)  
3) **Job complete** (worker → coordinator)

This pattern is simple, robust, and scales better than a coordinator trying to push tasks blindly.

---

## What the coordinator does

On every tick, the coordinator:
- reads incoming events (inbox + broadcasts)
- updates checkpoint state
- decides which jobs are runnable (dependencies satisfied)
- grants work to workers that have capacity

---

## Why the worker asks first

Because it’s the easiest way to avoid lying.

Workers know:
- how many cores they have
- how many tasks they can run
- what they’re currently doing

The coordinator knows:
- the global workflow graph and dependencies

Putting those together via “request/grant” keeps the system stable.

---

## Fairness and future improvements

Right now, the marketplace aims for “good enough” scheduling:
- don’t violate dependencies
- don’t exceed resources
- keep workers busy

The design leaves room for richer policies:
- tag-aware scheduling (GPUs, high-mem)
- priorities per workflow branch
- anti-affinity / locality constraints
