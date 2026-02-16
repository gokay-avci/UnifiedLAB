# CLI reference

UnifiedLab exposes three subcommands.

> Tip: if you ever wonder “what does this do?”, run `--help`.  
> It’s meant to be readable.

---

## `unifiedlab start`

Start the node service. In cluster mode, this is launched once per rank.

```bash
unifiedlab start --root ./scratch --force-local
```

### Options

- `--root <PATH>`  
  Where to keep state (`checkpoint.db`) and logs (`events.log`, `inbox/…`).

- `--force-local`  
  Required if you’re running on a workstation/laptop. This is a safety gate.

- `--limit-cores <N>`  
  Override detected core count (useful for testing packing/scheduling behaviour).

- `--tags <TAG>...`  
  Manually tag this node (e.g. `gpu`, `highmem`). Tags are a hook for smarter scheduling.

---

## `unifiedlab deploy`

Deploy a Draw.io blueprint to the cluster (really: to the inbox).

```bash
unifiedlab deploy --file experiment.drawio --root ./scratch
```

### Options

- `--file <PATH>`  
  Path to the `.drawio` XML file.

- `--root <PATH>`  
  Same root used by the coordinator/workers.

- `--params <JSON>`  
  A JSON object merged into *Generator* nodes’ parameter maps.

Example:

```bash
unifiedlab deploy --file experiment.drawio --root ./scratch \
  --params '{"gen_limit": 10, "bloom_strictness": 0.8}'
```

---

## `unifiedlab tui`

Launch the monitoring dashboard.

```bash
unifiedlab tui --checkpoint ./scratch/checkpoint.db
```

### Options

- `--checkpoint <PATH>`  
  Path to the SQLite checkpoint database.
