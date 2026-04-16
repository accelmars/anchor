# mind-engine

Reference-safe file operations for Markdown workspaces. Move files and directories without breaking any `[text](link.md)` or `[[wiki]]` reference — anywhere in your workspace.

## Install

```bash
cargo install --git https://github.com/accelmars/mind-engine
```

Requires Rust 1.70+. The binary is named `mind`.

## Quick start

```bash
# Initialize your workspace (once per machine)
mind init

# Check for broken references
mind file validate
```

## Commands

### `mind root`

Print the workspace root path.

```bash
mind root
# /Users/you/projects
```

Use in scripts: `$(mind root)/path/to/file`

---

### `mind init`

Initialize a workspace. Detects the workspace root (a directory containing git repos), places `.mind-root` there, and creates `.mind/config.json`. Also writes a default `.mindignore`.

```bash
mind init

# Detecting workspace root...
#   Candidate: /Users/you/projects/  (contains 5 git repos)
#
# Workspace root [/Users/you/projects/]: _
```

Run once per machine. `.mind-root` is not committed to git.

---

### `mind file mv <src> <dst>`

Reference-safe move of a file or directory. Rewrites every `[text](path.md)` and `[[wiki]]` reference to the moved item across the entire workspace — atomically.

```bash
mind file mv people/old-name.md people/new-name.md
mind file mv projects/active/my-project projects/archive/my-project
```

Output:
```
Scanning workspace... 1,247 files
Planning rewrites... 12 references to update

  REWRITE  docs/CLAUDE.md:47
  REWRITE  projects/other/STATUS.md:83
  ... and 10 more

Validating... ✓
Committing...  ✓

Done. Moved: projects/active/my-project → projects/archive/my-project
12 references updated across 10 files.
```

If validation fails after rewriting, the operation rolls back completely — workspace unchanged.

---

### `mind file validate`

Scan all `.md` files in the workspace and report broken references.

```bash
mind file validate

# ✓ 1,247 files scanned. No broken references.
```

When broken references are found:
```
BROKEN REFERENCES (2):

  docs/guide.md:34
    → ../archive/old-project/  (not found)

  people/alice/MIND.md:8
    → ../../archive/removed/  (not found)

2 broken references in 1,247 files.
```

Exit 0 = clean. Exit 1 = broken references found.

---

### `mind file refs <file>`

List all files in the workspace that reference a given file. Run this before moving a frequently-referenced file to understand the impact.

```bash
mind file refs projects/my-project/STATUS.md

# References to: projects/my-project/STATUS.md
#
#   docs/CLAUDE.md:47
#   people/alice/MIND.md:8
#
# 2 files reference this file.
```

Exit 0 always (zero refs is not an error).

---

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Logical failure (broken refs found, file not found, not initialized) |
| 2 | System error (permissions, I/O failure, workspace corrupted) |

`mind file refs` always exits 0 (zero references is a valid result, not a failure).

---

## Documented limitations

| Constraint | Description |
|-----------|-------------|
| `.md` only | mind-engine only processes Markdown files. Other file types not tracked. |
| Explicit links only | Only `[text](path.md)` and `[[wiki]]` forms. Plain text that looks like a path is not detected. |
| `mind init` required per machine | `.mind-root` is not in any git repo. Each machine requires `mind init` once. |
| Same filesystem | `rename(2)` atomicity requires `.mind/tmp/` on same filesystem as workspace. Different mount = non-atomic. |
| Torn write recovery: manual | If killed mid-COMMIT, user must inspect `manifest.json` and clean up `.mind/tmp/` manually. |
| Ambiguous wiki stems | If two `.md` files share the same stem, `mind file mv` aborts. User must resolve ambiguity first. |

---

## Excluding files and directories

Place a `.mindignore` file at your workspace root (same directory as `.mind-root`). Uses the same pattern syntax as `.gitignore`.

```
# .mindignore
node_modules/
target/
build/
```

`mind init` writes a default `.mindignore` with `node_modules/` and `target/`. Per-directory `.mindignore` files are not supported — only the root-level file is read.

---

## Existing `.mind/` directory

If your workspace already contains a `.mind/` directory from another tool, mind-engine will create its configuration inside it (`config.json`) without disturbing other files. mind-engine silently ignores any file in `.mind/` that it did not create.

If there is a conflict, check what's already there with `ls .mind/` before running `mind init`.

---

## License

Apache 2.0 — see [LICENSE](LICENSE).
