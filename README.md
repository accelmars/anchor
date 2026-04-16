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
# [1/2] Workspace root [/Users/you/projects/]: _
```

Run once per machine. `.mind-root` is not committed to git.

#### Flags

| Flag | Description |
|------|-------------|
| `--yes` | Accept the detected workspace root without prompting. Fully non-interactive — no prompts emitted, only the final success summary. |
| `--path <dir>` | Use `<dir>` as the workspace root, skipping auto-detection. Validates that the path exists and is writable. |

`--yes` and `--path` can be combined:

```bash
# Accept detected root non-interactively (CI, scripts)
mind init --yes

# Specify path explicitly (still prompts for confirmation)
mind init --path /Users/you/projects

# Fully non-interactive: specify path, skip all prompts
mind init --yes --path /Users/you/projects
```

---

### `mind file mv <src> <dst>`

Reference-safe move of a file or directory. Rewrites every `[text](path.md)` and `[[wiki]]` reference to the moved item across the entire workspace — atomically.

```bash
mind file mv people/old-name.md people/new-name.md
mind file mv projects/active/my-project projects/archive/my-project
```

Default (no flags): exits 0, no output on success.

With `--verbose`:
```
Moved. Rewrote 12 references in 10 files.
```

With `--format json`:
```json
{"moved":true,"refs_rewritten":12,"files_touched":10,"src":"projects/active/my-project","dst":"projects/archive/my-project"}
```

If validation fails after rewriting, the operation rolls back completely — workspace unchanged.

> **Atomicity:** The final `rename()` step is atomic on same-filesystem moves. Cross-filesystem moves (different mount points) are not atomic. Cross-filesystem atomicity is a Phase 2 concern.

#### Flags

| Flag | Description |
|------|-------------|
| `--verbose` | Print a human-readable confirmation on success. Mutually exclusive with `--format`. |
| `--format json` | Output JSON result on success. For AI agents and scripts. Mutually exclusive with `--verbose`. |

#### `--format json` schema

| Field | Type | Description |
|-------|------|-------------|
| `moved` | bool | Always `true` on success |
| `refs_rewritten` | number | Number of reference rewrites applied |
| `files_touched` | number | Number of distinct files whose content was updated |
| `src` | string | Workspace-relative source path |
| `dst` | string | Workspace-relative destination path |

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

#### `--format json`

Output results as JSON for programmatic use (AI agents, scripts):

```bash
mind file validate --format json
```

Clean workspace:
```json
{"clean":true,"files_scanned":1247,"broken":[],"acknowledged":0}
```

With broken references:
```json
{
  "clean": false,
  "files_scanned": 1247,
  "broken": [
    {"file": "docs/guide.md", "line": 34, "ref": "../archive/old-project/"},
    {"file": "people/alice/MIND.md", "line": 8, "ref": "../../archive/removed/"}
  ],
  "acknowledged": 3
}
```

| Field | Type | Description |
|-------|------|-------------|
| `clean` | bool | `true` if no unresolved broken references |
| `files_scanned` | number | Total `.md` files scanned |
| `broken` | array | Unresolved broken refs: `file` (workspace-relative), `line` (1-based), `ref` (raw target string) |
| `acknowledged` | number | Broken refs suppressed by `.mindacked` patterns |

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

Zero results:
```bash
mind file refs projects/my-project/STATUS.md
# No references found.
```

Exit 0 always (zero refs is not an error).

#### `--format json`

Output results as JSON for programmatic use (AI agents, scripts):

```bash
mind file refs projects/my-project/STATUS.md --format json
```

```json
{
  "refs": [
    {"file": "docs/CLAUDE.md", "line": 47},
    {"file": "people/alice/MIND.md", "line": 8}
  ],
  "query_path": "projects/my-project/STATUS.md",
  "count": 2
}
```

Zero results with `--format json`:
```json
{"refs": [], "query_path": "projects/my-project/STATUS.md", "count": 0}
```

| Field | Type | Description |
|-------|------|-------------|
| `refs` | array | Each entry: `file` (workspace-relative path), `line` (1-based) |
| `query_path` | string | Normalized workspace-relative path that was queried |
| `count` | number | Total number of reference hits |

> **Note for AI agents:** `count: 0` means the file exists but has no inbound references. If you receive `count: 0` for a path you expect to be referenced, verify the path is correct using `mind file validate`.

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
