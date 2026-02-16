# UnifiedLab docs (mdBook)

This folder is a self-contained mdBook project.

## Local preview

Install mdBook once:

```bash
cargo install mdbook --locked
```

Then serve the docs:

```bash
mdbook serve docs
```

Open the URL printed in your terminal (usually http://localhost:3000).

## Editing

All content lives in:

- `docs/src/` (Markdown)
- `docs/src/SUMMARY.md` controls the sidebar navigation.

## Publishing

If you later want to publish to GitHub Pages, you can build:

```bash
mdbook build docs
```

and deploy `docs/book/` as static files.
