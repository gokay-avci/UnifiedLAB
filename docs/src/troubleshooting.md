# Troubleshooting

This is a living page. Here are the common “first-time” problems and what they usually mean.

---

## “SAFETY: Local execution detected. Use --force-local …”

You’re on a workstation/laptop and UnifiedLab is protecting you from yourself.

Fix:

```bash
unifiedlab start --force-local
```

---

## `deploy` says it worked, but nothing happens

Checklist:

1) Are you using the same `--root` for `start` and `deploy`?
2) Did an inbox log appear under `root/inbox/`?
3) Is the coordinator running (rank 0 process alive)?
4) Does the TUI show any jobs being created?

If (2) is false, deployment didn’t write to the right place.
If (2) is true but nothing changes, look at coordinator logs / event log reading.

---

## The TUI opens but shows nothing

Usually:
- you pointed it at the wrong checkpoint path
- the service hasn’t created the DB yet
- permissions / filesystem issues

Try:

```bash
ls -lah ./scratch/checkpoint.db
sqlite3 ./scratch/checkpoint.db 'SELECT count(*) FROM jobs;'
```

---

## Cluster runs are “weird”

HPC tip: always confirm rank/world-size detection.

UnifiedLab checks common variables (Slurm and MPI). If your site uses something exotic, you may need to patch detection.

If jobs inside the allocation oversubscribe nodes, pay attention to:
- `--limit-cores`
- whether your launch method exports the standard MPI env vars

---

## I changed the blueprint, but the old one seems to run

Remember: `deploy` writes a submission payload to the inbox.
If you deploy multiple times, you will have multiple payload logs.

If you want a clean slate, delete your root folder:

```bash
rm -rf ./scratch
mkdir -p ./scratch
```
