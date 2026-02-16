# Workflow blueprint (Draw.io)

UnifiedLab lets you describe a workflow visually, because honestly: it’s easier to reason about a graph when you can *see* it.

## The idea

- You create a `.drawio` diagram with nodes and edges.
- UnifiedLab imports it into an internal graph.
- Each node becomes a job.
- Each edge becomes a dependency / data flow relation.

Then `deploy` turns that into a submission payload and drops it into the inbox.

---

## Node types (what a box *means*)

At the moment, UnifiedLab recognises a handful of node categories:

- **Compute**  
  A “do the expensive thing” job (e.g. DFT / MD / GULP / whatever you wrap).

- **Generator**  
  A “propose new candidates” node. Think: evolutionary mutation, Bayesian optimisation, random sampling, etc.  
  Generator nodes accept parameter overrides via `deploy --params '{...}'`.

- **Switch**  
  Conditional logic: choose what happens next based on a result (energy, bandgap, or an external script).

- **Aggregator**  
  Collect and summarise results from upstream.

- **Verifier**  
  Sanity checking / tolerance checks.

- **Sentinel**  
  A guard node. Useful for “stop the world if X”.

You don’t need to memorise this. The point is: the graph is readable.

---

## Edges (what an arrow *means*)

Edges can represent:
- hard dependency
- soft dependency
- parameter mapping (data flow)

If you’re not sure what you need, start with hard dependencies everywhere.
You can make it clever later.

---

## How I recommend you start (simple and robust)

1) Draw a straight line of 3 nodes: **Generator → Compute → Aggregator**
2) Deploy with a tiny `gen_limit` override
3) Watch it in the TUI

Once that’s stable:
- add branches (switches)
- add more compute nodes
- add verification

---

## Debugging your blueprint

When something doesn’t behave:
- Start with a *small* blueprint (3–5 nodes).
- Check that `deploy` succeeds and writes an inbox payload.
- Watch the coordinator logs and TUI state to confirm jobs are created.

If you want help making a “gold standard” example blueprint for your use case (MOFs, surface science, active learning, etc.), I can sketch a canonical graph layout and the parameters you’d typically wire through.
