# AccelMarsAnchor

Reference-safe file operations for Markdown workspaces. Move files and directories without breaking any `[text](link.md)` or `[[wiki]]` reference — anywhere in your workspace.

## Install

```bash
cargo install --git https://github.com/accelmars/anchor
```

Requires Rust 1.70+. The binary is named `anchor`.

## Quick start

```bash
# Initialize your workspace (once per machine)
anchor init

# Check for broken references
anchor file validate
```

## Commands

### `anchor root`

Print the workspace root path.

```bash
anchor root
# /Users/you/projects
```

Use in scripts: `$(anchor root)/path/to/file`

---

### `anchor init`

Initialize a workspace. Detects the workspace root (a directory containing git repos), creates `.accelmars/anchor/config.json` there. Also writes a default `.accelmars/anchor/ignore`.

```bash
anchor init

# Detecting workspace root...
#   Candidate: /Users/you/projects/  (contains 5 git repos)
#
# [1/2] Workspace root [/Users/you/projects/]: _
```

Run once per machine. `.accelmars/anchor/` is not committed to git.

#### Flags

| Flag | Description |
|------|-------------|
| `--yes` | Accept the detected workspace root without prompting. Fully non-interactive — no prompts emitted, only the final success summary. |
| `--path <dir>` | Use `<dir>` as the workspace root, skipping auto-detection. Validates that the path exists and is writable. |

`--yes` and `--path` can be combined:

```bash
# Accept detected root non-interactively (CI, scripts)
anchor init --yes

# Specify path explicitly (still prompts for confirmation)
anchor init --path /Users/you/projects

# Fully non-interactive: specify path, skip all prompts
anchor init --yes --path /Users/you/projects
```

---

### `anchor file mv <src> <dst>`

Reference-safe move of a file or directory. Rewrites every `[text](path.md)` and `[[wiki]]` reference to the moved item across the entire workspace — atomically.

```bash
anchor file mv people/old-name.md people/new-name.md
anchor file mv projects/active/my-project projects/archive/my-project
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

### `anchor file validate`

Scan all `.md` files in the workspace and report broken references.

```bash
anchor file validate

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
anchor file validate --format json
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
| `acknowledged` | number | Broken refs suppressed by `.accelmars/anchor/acked` patterns |

---

### `anchor file refs <file>`

List all files in the workspace that reference a given file. Run this before moving a frequently-referenced file to understand the impact.

```bash
anchor file refs projects/my-project/STATUS.md

# References to: projects/my-project/STATUS.md
#
#   docs/CLAUDE.md:47
#   people/alice/MIND.md:8
#
# 2 files reference this file.
```

Zero results:
```bash
anchor file refs projects/my-project/STATUS.md
# No references found.
```

Exit 0 always (zero refs is not an error).

#### `--format json`

Output results as JSON for programmatic use (AI agents, scripts):

```bash
anchor file refs projects/my-project/STATUS.md --format json
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

> **Note for AI agents:** `count: 0` means the file exists but has no inbound references. If you receive `count: 0` for a path you expect to be referenced, verify the path is correct using `anchor file validate`.

---

## Architecture

anchor works at the **workspace** level — a directory that contains one or more git repositories (not a single repo). When you run `anchor init`, it detects or accepts a workspace root and writes configuration there.

Inside that workspace, anchor tracks two kinds of Markdown references:

- `[text](path.md)` — standard Markdown links to other `.md` files
- `[[wiki]]` — wiki-style links by filename stem

When you move a file with `anchor file mv`, anchor rewrites every inbound reference to that file across the entire workspace before committing the filesystem rename. The rename and reference rewrites are applied as a single atomic transaction — if anything fails, the workspace rolls back to its original state unchanged.

anchor does **not** track:

- Non-Markdown files (code, images, binaries, data files)
- Plain text that looks like a path but is not a Markdown link
- References in files excluded by `.accelmars/anchor/ignore`

---