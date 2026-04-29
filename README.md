# AccelMars Anchor
---

## What Anchor Is

Anchor is a workspace-aware file operations tool. It does what `mv` does — moves
files and directories — but also rewrites cross-references in markdown files so links
don't break. It adds a plan layer so you can preview, validate, and apply changes as
a batch.

**The core loop:**

```
write plan → validate → diff (preview) → apply
```

A plan is a `.toml` file describing a list of operations. Anchor executes them in
order and rewrites any markdown `[text](path)` links that pointed to moved paths.

---

## Install

```sh
cargo install --git https://github.com/accelmars/anchor
```

Requires Rust 1.70+. The binary is named `anchor`.

---

## First Time Setup

Run once in the root of your project:

```sh
anchor init
```

This creates `.accelmars/anchor/config.json` alongside two config files:
- `.accelmars/anchor/ignore` — paths to exclude from ref-scanning (gitignore syntax)
- `.accelmars/anchor/acked` — acknowledged broken refs you've deferred fixing

`.accelmars/anchor/` is not committed to git — run `anchor init` once per machine.

---

## All Commands

| Command | What it does |
|---------|-------------|
| `anchor init` | Initialize workspace |
| `anchor root` | Print workspace root path |
| `anchor plan list` | Show available plan templates |
| `anchor plan new [--output file.toml]` | Interactive wizard → generates a plan file |
| `anchor plan validate <plan.toml>` | Check plan is valid before applying |
| `anchor diff <plan.toml>` | Preview what apply will do (read-only) |
| `anchor apply <plan.toml>` | Execute the plan and rewrite refs |
| `anchor file mv <src> <dst>` | Move one file/dir immediately (no plan) |
| `anchor file validate` | Scan workspace for broken markdown refs |
| `anchor file refs <file>` | Find all files that link to a given file |
| `anchor frontmatter audit\|migrate\|normalize\|add-required\|check-schema` | Frontmatter management |

---

## Direct Move

For single operations, skip the plan entirely:

```sh
anchor file mv old-path new-path
```

Moves the file or directory and rewrites all inbound markdown refs in one step.

---

## When NOT to use anchor

anchor is purpose-built for Markdown workspaces with cross-file links. Wrong tool for:

- **Moving non-Markdown files.** Code, images, and binaries are not tracked. Use `git mv`.
- **Global search-and-replace.** anchor rewrites references, not arbitrary strings. Use `sed`.
- **Repos with no cross-file Markdown links.** anchor adds overhead with no benefit.
- **Replacing `git mv` on source code.** Source code moves belong in your normal git workflow.

---

## Telemetry

anchor collects no telemetry. No data leaves your machine.

---

## License

Apache 2.0 — see [LICENSE](LICENSE).

---

## Go Deeper

| Question | Doc |
|----------|-----|
| How do I use the plan workflow? | [docs/PLAN-WORKFLOW.md](docs/PLAN-WORKFLOW.md) |
| What does anchor rewrite (and what doesn't it)? | [docs/REWRITES.md](docs/REWRITES.md) |
| How do I check for broken refs / find what links to a file? | [docs/REFERENCE-HEALTH.md](docs/REFERENCE-HEALTH.md) |
| What are anchor's limitations and constraints? | [docs/LIMITATIONS.md](docs/LIMITATIONS.md) |
| How do I exclude files from scanning? | [docs/IGNORE.md](docs/IGNORE.md) |
| What does a complete anchor session look like? | [docs/TYPICAL-SESSION.md](docs/TYPICAL-SESSION.md) |
| Full flag docs and JSON schemas for every command | [docs/COMMAND-REFERENCE.md](docs/COMMAND-REFERENCE.md) |
| Exit codes | [docs/EXIT-CODES.md](docs/EXIT-CODES.md) |
