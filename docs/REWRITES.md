# What Anchor Rewrites

Understanding the scope of ref-rewriting is critical to knowing when a move is
truly clean.

---

## Rewritten ✓

| Format | Example |
|--------|---------|
| Standard markdown link | `[text](path/to/file.md)` |
| Fragment link | `[text](path/to/file.md#section)` |
| Title-attribute link | `[text](path/to/file.md "Title")` |
| Backtick refs in .md files | `` `path/to/file.md` `` |
| HTML anchor tags | `<a href="path/to/file.md">` |
| Link text mirroring the path | `[path/to/file.md](path/to/file.md)` — text updated in sync |

---

## NOT rewritten ✗

| Format | Example |
|--------|---------|
| Plain prose | `see path/to/file.md for details` |
| Markdown table cells (non-link) | `\| path/to/file.md \|` |
| Fenced code blocks | ` ```\npath/to/file.md\n``` ` |
| Non-.md files | `config.json`, `settings.yaml`, `.ts`, `.py` |

---

## Post-apply warnings

**Plain text in `.md` files** — After apply, anchor reports partial-path plain-text
occurrences that were not rewritten (e.g. bare `os-council` after moving
`councils/os-council`). The move completes successfully; the report is informational.
Check navigation documents (`CLAUDE.md`, `README.md`) for bare path references.

**Non-.md files** — Anchor emits a `stderr` warning listing how many non-markdown
files contain the old path, and suggests `anchor file refs <old-path>` to inspect
them. You must fix those files manually.

---

## Context scoping

Each move op bounds its rewrite scope to the deepest git-repo ancestor of the source
path. A common-noun folder rename (e.g. `workflows/`) inside one repo does not
rewrite unrelated occurrences in sibling repos. Files outside the scope that hold a
fully-qualified workspace-relative path to the moved location are still rewritten
(inward-ref rule).
