# Installation

You have two normal ways to run UnifiedLab:

1) **From source** (best while you’re iterating)
2) **As an installed binary** (`cargo install --path .`) once you’re happy

---

## From source (recommended while developing)

```bash
git clone https://github.com/<YOUR_GH>/unifiedlab
cd unifiedlab

cargo build
./target/debug/unifiedlab --help
```

---

## Install the binary locally

From the repo root:

```bash
cargo install --path . --locked
unifiedlab --help
```

---

## Optional: Python shim

UnifiedLab can call out to a Python “shim” for ML agents / bespoke logic (e.g. AutoEmulate style loops).
If you use that pathway, you’ll want:

- Python 3.10+
- A `scripts/active_learning_shim.py` that implements the request/response contract

See: [Python shim contract](python-shim.md)

---

## A quick note on Rust + terminal UI

The TUI is meant to be **useful**, not flashy. If you’re SSH’d into a cluster login node, it should still work.
If your terminal is weird (fonts, unicode, colours), see [Troubleshooting](troubleshooting.md).
