# TUI dashboard

The TUI exists for one reason: **you should be able to answer “what is happening?” in 10 seconds.**

It reads from the checkpoint database and shows:
- workers (last seen, resources)
- jobs (queued/running/done/failed)
- recent events/log output

Run it like this:

```bash
unifiedlab tui --checkpoint ./scratch/checkpoint.db
```

---

## What to look at first

When you’re debugging, I typically scan in this order:

1) **Worker heartbeat**  
   Are workers alive? Are they reporting in?

2) **Job status distribution**  
   Are jobs stuck in “queued”? Are failures spiking?

3) **Recent events**  
   Did the deploy payload land? Are work requests/grants flowing?

---

## A note on expectations

The TUI is intentionally conservative:
- it’s not meant to be a perfect “Kubernetes dashboard”
- it’s meant to be a *sane* window into a workflow graph running on an HPC allocation

If you need richer visualisation later:
- the checkpoint DB is the clean integration point
- you can build a web dashboard on top without changing core orchestration
