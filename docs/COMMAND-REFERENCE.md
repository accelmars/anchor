# Command Reference

Full flag documentation and output schemas for every anchor command.

For the plan workflow (write → validate → diff → apply), see [PLAN-WORKFLOW.md](PLAN-WORKFLOW.md).
For exit codes, see [EXIT-CODES.md](EXIT-CODES.md).

---

## `anchor root`

Print the workspace root path.

```bash
anchor root
# /Users/you/projects
```

Use in scripts: `$(anchor root)/path/to/file`

---

## `anchor init`

Initialize a workspace. Detects the workspace root (a directory containing git
repos), creates `.accelmars/anchor/config.json` there. Also writes default
`.accelmars/anchor/ignore` and `.accelmars/anchor/acked` files.

```bash
anchor init

# Detecting workspace root...
#   Candidate: /Users/you/projects/  (contains 5 git repos)
#
# [1/2] Workspace root [/Users/you/projects/]: _
```

Run once per machine. `.accelmars/anchor/` is not committed to git.

### Flags

| Flag | Description |
|------|-------------|
| `--yes` | Accept the detected workspace root without prompting. Fully non-interactive. |
| `--path <dir>` | Use `<dir>` as the workspace root, skipping auto-detection. |

`--yes` and `--path` can be combined:

```bash
anchor init --yes                          # non-interactive, detected root
anchor init --path /Users/you/projects     # explicit path, prompts for confirmation
anchor init --yes --path /Users/you/projects  # fully non-interactive
```

---

## `anchor file mv <src> <dst>`

Reference-safe move of a file or directory. Rewrites every markdown reference to
the moved item across the entire workspace — atomically.

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

If post-rewrite validation fails, the operation rolls back completely — workspace
unchanged.

> **Atomicity:** The final `rename()` step is atomic on same-filesystem moves.
> Cross-filesystem moves are not atomic.

### Flags

| Flag | Description |
|------|-------------|
| `--verbose` | Print a human-readable confirmation on success. Mutually exclusive with `--format`. |
| `--format json` | Output JSON result on success. For AI agents and scripts. Mutually exclusive with `--verbose`. |

### `--format json` schema

| Field | Type | Description |
|-------|------|-------------|
| `moved` | bool | Always `true` on success |
| `refs_rewritten` | number | Number of reference rewrites applied |
| `files_touched` | number | Number of distinct files whose content was updated |
| `src` | string | Workspace-relative source path |
| `dst` | string | Workspace-relative destination path |

---

## `anchor file validate`

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

`anchor validate` is an alias for `anchor file validate`.

Exit 0 = clean. Exit 1 = broken references found.

### `--format json`

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
| `broken` | array | Unresolved broken refs: `file`, `line` (1-based), `ref` (raw target string) |
| `acknowledged` | number | Broken refs suppressed by `.accelmars/anchor/acked` |

---

## `anchor file refs <file>`

List all files in the workspace that reference a given file.

```bash
anchor file refs projects/my-project/STATUS.md

# References to: projects/my-project/STATUS.md
#
#   docs/CLAUDE.md:47
#   people/alice/MIND.md:8
#
# 2 files reference this file.
```

Exit 0 always — zero results is not an error.

### `--format json`

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

Zero results:
```json
{"refs": [], "query_path": "projects/my-project/STATUS.md", "count": 0}
```

| Field | Type | Description |
|-------|------|-------------|
| `refs` | array | Each entry: `file` (workspace-relative), `line` (1-based) |
| `query_path` | string | Normalized workspace-relative path that was queried |
| `count` | number | Total number of reference hits |

> **Note for AI agents:** `count: 0` means the file exists but has no inbound
> references. If you receive `count: 0` for a path you expect to be referenced,
> verify the path is correct using `anchor file validate`.

---

## `anchor text find <pattern>`

Read-only enumeration of literal-or-regex occurrences across markdown files,
with surrounding context lines per match. Companion to the `text_rename` plan
op: use `text find` to inspect candidate sites before authoring a rename.

```bash
anchor text find "apps-monorepo"

# forge/projects/atlas-v01/_PROJECT.md:42
#   > 40: Atlas is the customer-facing app
#   > 41: built as part of the
#   > 42: apps-monorepo
#   > 43: alongside the ops admin
#   > 44: app.
#
# Found 1 occurrence(s) in 1 file(s).
```

### Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--regex` | off | Treat pattern as regex instead of literal |
| `--include <GLOB>` | (any) | Restrict search to files matching this glob (repeatable) |
| `--exclude <GLOB>` | none | Skip files matching this glob (repeatable) |
| `--file-type <EXT>` | `md` | File extensions to search (repeatable) |
| `--context <N>` | `2` | Lines of context before and after each match |
| `--format text\|json` | `text` | Output format |
| `--include-code-blocks` | off | Include matches inside fenced code blocks |
| `--exclude-frontmatter` | off | Skip matches inside YAML frontmatter |

### Exit codes

| Code | Meaning |
|------|---------|
| `0` | One or more occurrences found |
| `1` | No occurrences found |
| `2` | Error (workspace not initialized, invalid regex, ...) |

### `--format json`

Stable schema for machine consumers (anchor-engine `--review`, AI agents):

```json
{
  "pattern": "apps-monorepo",
  "regex": false,
  "results": [
    {
      "file": "forge/projects/atlas-v01/_PROJECT.md",
      "line": 42,
      "column": 5,
      "match_text": "apps-monorepo",
      "context_before": ["...line 40", "...line 41"],
      "context_after": ["...line 43", "...line 44"],
      "in_code_block": false,
      "in_frontmatter": false,
      "compound_match": false
    }
  ],
  "total_occurrences": 1,
  "total_files": 1
}
```

| Field | Type | Description |
|-------|------|-------------|
| `pattern` | string | Original search pattern |
| `regex` | bool | Whether `--regex` was used |
| `results[].file` | string | Workspace-relative path |
| `results[].line` | number | 1-based line number |
| `results[].column` | number | 1-based column of match start |
| `results[].match_text` | string | The matched substring |
| `results[].context_before` | array<string> | Up to `--context` lines preceding the match |
| `results[].context_after` | array<string> | Up to `--context` lines following the match |
| `results[].in_code_block` | bool | Match lies inside a fenced code block |
| `results[].in_frontmatter` | bool | Match lies inside YAML frontmatter |
| `results[].compound_match` | bool | Literal match is adjacent to identifier characters (likely part of a larger name); always false for regex |

> **Note for AI agents:** `compound_match: true` flags sites where a literal
> substitution might catch more than intended (e.g., `apps-monorepo` matching
> inside `apps-monorepo-deploy`). Inspect those sites before applying a
> `text_rename` plan that would rewrite them.

---

## `anchor frontmatter` subcommands

Frontmatter management for `.md` files against a JSON Schema authority.

### `anchor frontmatter audit <dir>`

Report schema compliance drift: missing required fields, invalid enum values,
wrong schema version.

```bash
anchor frontmatter audit ./docs
anchor frontmatter audit ./docs --format json
```

### `anchor frontmatter migrate --to <version> <dir>`

Apply schema_version transitions. Dry-run by default; use `--apply` to write.

```bash
anchor frontmatter migrate --to 1 ./docs          # dry-run
anchor frontmatter migrate --to 1 --apply ./docs  # write changes
```

### `anchor frontmatter normalize [--reorder] <dir>`

Resolve status synonyms and optionally reorder keys to canonical order.
Dry-run by default; use `--apply` to write.

```bash
anchor frontmatter normalize --reorder ./docs
anchor frontmatter normalize --reorder --apply ./docs
```

### `anchor frontmatter add-required <dir>`

Add missing required fields with default values. Dry-run by default.

```bash
anchor frontmatter add-required ./docs
anchor frontmatter add-required --apply ./docs
```

### `anchor frontmatter check-schema <spec> <schema>`

CI guard: exits 1 if `FRONTMATTER.md` and `FRONTMATTER.schema.json` diverge.

```bash
anchor frontmatter check-schema FRONTMATTER.md frontmatter.schema.json
echo "Exit: $?"  # 0 = in sync, 1 = diverged
```

---

## Plan commands

See [PLAN-WORKFLOW.md](PLAN-WORKFLOW.md) for the full workflow.

| Command | What it does |
|---------|-------------|
| `anchor plan list` | Show available plan templates |
| `anchor plan new [--output file.toml]` | Interactive wizard → generates a plan file |
| `anchor plan validate <plan.toml>` | Check plan is valid (src exists, dst free) |
| `anchor diff <plan.toml>` | Preview what apply will do (read-only) |
| `anchor diff --verbose <plan.toml>` | Preview with per-file, per-ref detail |
| `anchor apply <plan.toml>` | Execute the plan and rewrite refs |

### Plan op schema

`create_dir`:

```toml
[[ops]]
type = "create_dir"
path = "new-parent"
```

`move`:

```toml
[[ops]]
type = "move"
src = "old-location"
dst = "new-location"
```

`text_rename`:

```toml
[[ops]]
type = "text_rename"
from = "old term"
to = "new term"
include_paths = ["docs/**"]
exclude_paths = ["docs/archive/**"]
file_types = ["md"]
literal = true
match_in_code_blocks = false
match_in_frontmatter = true
```

Defaults: `include_paths = []` means all matching file types,
`exclude_paths = []`, `file_types = ["md"]`, `literal = true`,
`match_in_code_blocks = false`, and `match_in_frontmatter = true`.

When `literal = false`, `from` is treated as a regex. `text_rename` is content
substitution only; it is separate from move/link rewriting.
