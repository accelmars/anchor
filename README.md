# AccelMarsAnchor
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

## First Time Setup

Before anchor commands work, you need a workspace marker. Run once in the root of
your project:

```sh
anchor init --path .
```

This creates `.accelmars/anchor/config.json` alongside two config files:
- `.accelmars/anchor/ignore` — paths to exclude from ref-scanning (gitignore syntax)
- `.accelmars/anchor/acked` — acknowledged broken refs you've deferred fixing

**Note — post-v0.3.0: `anchor init --yes` without `--path` now defaults to CWD with a notice instead of erroring.**

**Remaining gotcha — explicit `--path` bypasses parent detection.**  
If you run `anchor init --yes --path ./subdir` and a parent directory already has
`.accelmars/`, anchor still creates a nested workspace without warning. Check for a
parent workspace first:
```sh
ls ../.accelmars 2>/dev/null && echo "parent workspace exists — skip init"
```

---

## All Commands (Quick Reference)

| Command | What it does |
|---------|-------------|
| `anchor init --path <dir>` | Initialize workspace at path |
| `anchor root` | Print workspace root path. Useful in scripts. |
| `anchor plan list` | Show available plan templates |
| `anchor plan new [--output file.toml]` | Interactive wizard → generates a plan file |
| `anchor plan validate <plan.toml>` | Check plan is valid before applying |
| `anchor diff <plan.toml>` | Preview what apply will do (read-only) |
| `anchor apply <plan.toml>` | Execute the plan and rewrite refs |
| `anchor file mv <src> <dst>` | Move one file/dir immediately (no plan) |
| `anchor file validate` | Scan workspace for broken markdown refs |
| `anchor file refs <file>` | Find all files that link to a given file |

---

## The Plan-Based Workflow

Use this when you have multiple operations to perform together. It gives you a chance
to review before committing.

### Step 1 — Write the plan

**Option A: Interactive wizard**

```sh
anchor plan new --output my-plan.toml
```

The wizard lists templates and asks questions interactively. Select a template by
number, then answer the prompts. The wizard writes a `.toml` file ready for apply.

Available templates:

| # | Template | Use when |
|---|----------|----------|
| 1 | Batch Move | You have an explicit list of src→dst pairs |
| 2 | Categorize | You want to group flat items under a new parent folder |
| 3 | Archive | You want to move items into an archive location |
| 4 | Rename | You want to rename items |
| 5 | Scaffold | You want to create a directory structure |

**Tip — wizard as scaffold:** For complex plans (10+ moves), use the wizard to
generate a starting file, then open it and edit directly — the TOML format is
straightforward (see Option B below). Run `anchor plan validate` to confirm your
edits are correct before applying.

**Option B: Hand-author the TOML**

Plan files are straightforward TOML. Write one directly:

```toml
version = "1"
description = "What this plan does"

[[ops]]
type = "create_dir"
path = "new-parent"

[[ops]]
type = "move"
src = "old-location"
dst = "new-parent/new-name"
```

Available op types: `move`, `create_dir`. That's it for v0.3.0.

**Important:** If your move destinations are inside a new parent directory that
doesn't exist yet, add a `create_dir` op for it before the moves. The wizard now
detects when destinations require a new parent and prompts to add `create_dir` ops.

### Step 2 — Validate

```sh
anchor plan validate my-plan.toml
```

Checks that every `src` exists and every `dst` does not yet exist. Exits 0 if valid,
1 if errors. Fix any errors before proceeding.

**v0.4.0:** `anchor plan validate` now emits a `note:` when a destination parent does not exist. Validate still exits 0 — missing parents are auto-created by apply.

### Step 3 — Preview

```sh
anchor diff my-plan.toml
```

Shows each operation and how many refs will be rewritten, without touching any files.
Safe to run repeatedly.

Use `anchor diff --verbose` to list each file and ref that will be rewritten.

### Step 4 — Apply

```sh
anchor apply my-plan.toml
```

Executes all ops in order. For each move, rewrites markdown cross-references. Prints
progress as it runs.

---

## The Direct Move Workflow

For single operations, skip the plan entirely:

```sh
anchor file mv old-path new-path
```

This moves the file or directory and rewrites all inbound markdown refs in one step.
Same ref-rewriting behaviour as apply — same limitations apply.

---

## Reference Health

### Check for broken refs

```sh
anchor file validate
```

Scans all `.md` files in the workspace. Reports any `[text](path)` links where the
target doesn't exist, with file and line number.

```
BROKEN REFERENCES (2):
  my-file.md:14  → old-folder/guide.md  (not found)
    similar: new-folder/guide.md
2 broken references in 2 files.
```

The `similar:` hint is a best-guess fuzzy match — sometimes accurate, sometimes not.

Exit codes: 0 = clean, 1 = broken refs found, 2 = system error.

### Find what references a file

```sh
anchor file refs path/to/file.md
```

Lists every file in the workspace that links to the given path. Useful before moving
something to understand the blast radius.

---

## What Anchor Rewrites (and What It Doesn't)

Understanding the scope of ref-rewriting is critical to knowing when a rename is
truly clean.

### Rewritten ✓

| Format | Example |
|--------|---------|
| Standard markdown link | `[text](path/to/file.md)` |
| Fragment link | `[text](path/to/file.md#section)` |
| Title-attribute link | `[text](path/to/file.md "Title")` |
| Backtick refs in .md files | `` `path/to/file.md` `` |
| HTML anchor tags | `<a href="path/to/file.md">` |
| Link text matching path | `[path/to/file.md](path/to/file.md)` (text updated in sync) |

### NOT rewritten ✗

| Format | Example |
|--------|---------|
| Plain text (prose, tables, code blocks) | `see path/to/file.md for details` — not rewritten, but anchor warns when 0 refs are rewritten and plain-text occurrences exist |
| Markdown table cells (non-link) | `\| path/to/file.md \|` |
| Fenced code blocks | ` ```\npath/to/file.md\n``` ` |
| Non-.md files | `config.json`, `settings.yaml`, `.ts`, `.py` |

**Watch for non-.md files.** Anchor now rewrites backtick refs and HTML links within
`.md` files, so most in-markdown formats are covered. The remaining gap is
**non-.md files** (`config.json`, `settings.yaml`, `.ts`, `.py`): anchor emits a
`stderr` warning listing how many such files contain occurrences, and suggests
`anchor refs <old-path>` to inspect them — but you must fix those files manually.

**Plain text within `.md`** (unformatted prose, table cells with bare paths, fenced
code blocks) is also not rewritten. Check your navigation documents
(`CONSTELLATION.md`, `CLAUDE.md`, `README.md`) for bare path references after apply.

---

## Known Limitations

### No rollback on partial failure

If op 5 of 10 fails, ops 1–4 have been applied and cannot be undone automatically.
Run `anchor file validate` to identify the state of the workspace, then apply a
corrective plan.

### Explicit `--path` bypasses parent workspace detection

`anchor init --yes` (no `--path`) now defaults to CWD safely. But if you pass
`--path ./subdir` explicitly, anchor does not check whether a parent directory
already has `.accelmars/` — it creates a nested workspace without warning.

### Wizard is a scaffold, not a complete authoring tool

The wizard is designed for simple plans (up to ~5 items). For larger or more
complex plans: generate a starting file with the wizard, edit it manually, then
use `anchor plan validate` to check your edits before applying.

### Non-.md file paths not rewritten

Paths in `config.json`, `settings.yaml`, `.ts`, `.py`, and other non-markdown files
are not rewritten. Anchor warns you (`stderr`) with a count after each move — treat
this as a TODO list for manual cleanup.

---

## Exit Codes

| Command | Exit 0 | Exit 1 | Exit 2 |
|---------|--------|--------|--------|
| `anchor init` | Initialized | Error (bad path, no candidate) | — |
| `anchor root` | Root found | No workspace | System error |
| `anchor file mv` | Moved + refs rewritten | Flag conflict | Src missing, dst exists, I/O error |
| `anchor file validate` | No broken refs | Broken refs found | System error |
| `anchor file refs` | Always exits 0 | — | — |
| `anchor apply` | All ops complete | Preflight or op failure | Workspace/infra error |
| `anchor diff` | Preview shown | Plan parse error or no workspace found | Scan/I/O error |
| `anchor plan validate` | Plan valid | Validation errors | Parse error |
| `anchor plan new` | Plan file written | Invalid input or write failure | — |
| `anchor plan list` | Templates listed (always) | — | — |

---

## Typical Session

```sh
# 1. Check workspace is initialized
anchor root

# 2. See what references a folder before moving it
anchor file refs foundations/gateway-engine/

# 3. Write the plan (by hand for large batches)
cat > restructure.toml << 'EOF'
version = "1"
description = "Group foundations under foundations/"

[[ops]]
type = "create_dir"
path = "foundations"

[[ops]]
type = "move"
src = "foundations/gateway-engine"
dst = "foundations/gateway-engine"
EOF

# 4. Validate
anchor plan validate restructure.toml

# 5. Preview (add --verbose to see each ref that will change)
anchor diff --verbose restructure.toml

# 6. Apply
anchor apply restructure.toml
# anchor warns on stderr if any non-.md files have unhandled occurrences

# 7. Verify — must exit 0
anchor validate

# 8. Manually check plain-text refs (not in backticks/links) in navigation docs
grep -r "foundations/gateway-engine" CONSTELLATION.md CLAUDE.md
```

## When NOT to use anchor

anchor is purpose-built for Markdown workspaces with cross-file links. It is the wrong tool for:

- **Moving non-Markdown files.** Code, images, and binaries are not tracked. Use `git mv` or your shell for those.
- **Global search-and-replace.** anchor rewrites references, not arbitrary strings. Use `sed` or your editor's find-and-replace.
- **Repos with no cross-file Markdown links.** If your `.md` files don't link to each other, anchor adds overhead with no benefit.
- **Replacing `git mv` on source code.** anchor is not a general-purpose file manager. Source code moves belong in your normal git workflow.

---

## Telemetry

anchor collects no telemetry. No data leaves your machine.

---

## Documented limitations

| Constraint | Description |
|-----------|-------------|
| `.md` only | anchor only processes Markdown files. Other file types not tracked. |
| Explicit links only | Only `[text](path.md)` and `[[wiki]]` forms. Plain text that looks like a path is not detected. |
| `anchor init` required per machine | `.accelmars/anchor/` is not in any git repo. Each machine requires `anchor init` once. |
| Same filesystem | `rename(2)` atomicity requires `.accelmars/anchor/` on same filesystem as workspace. Different mount = non-atomic. |
| Torn write recovery: manual | If killed mid-COMMIT, user must inspect `manifest.json` and clean up `.accelmars/anchor/tmp/` manually. |
| Ambiguous wiki stems | If two `.md` files share the same stem, `anchor file mv` aborts. User must resolve ambiguity first. |

---

## Excluding files and directories

Place an ignore file at `.accelmars/anchor/ignore` in your workspace root. Uses the same pattern syntax as `.gitignore`.

```
# .accelmars/anchor/ignore
node_modules/
target/
build/
```

`anchor init` writes a default ignore file with `node_modules/` and `target/`.

---

## License

Apache 2.0 — see [LICENSE](LICENSE).
