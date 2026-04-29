# Plan-Based Workflow

Use this when you have multiple operations to perform together. A plan gives you a
chance to validate and preview before committing any changes.

**The loop:**

```
write plan → validate → diff (preview) → apply
```

---

## Step 1 — Write the plan

**Option A: Interactive wizard**

```sh
anchor plan new --output my-plan.toml
```

The wizard lists templates and asks questions interactively. Select a template by
number, answer the prompts, and it writes a `.toml` file ready for apply.

Available templates:

| # | Template | Use when |
|---|----------|----------|
| 1 | Batch Move | You have an explicit list of src→dst pairs |
| 2 | Categorize | You want to group flat items under a new parent folder |
| 3 | Archive | You want to move items into an archive location |
| 4 | Rename | You want to rename items |
| 5 | Scaffold | You want to create a directory structure |

**Tip — wizard as scaffold:** For complex plans (10+ moves), use the wizard to
generate a starting file, then open it and edit directly. Run `anchor plan validate`
to confirm your edits before applying.

**Option B: Hand-author the TOML**

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

Available op types: `move`, `create_dir`.

**Important:** If move destinations are inside a new parent directory that doesn't
exist yet, add a `create_dir` op for it before the moves. The wizard detects this
and prompts to add `create_dir` ops automatically.

---

## Step 2 — Validate

```sh
anchor plan validate my-plan.toml
```

Checks that every `src` exists and every `dst` does not. Exits 0 if valid, 1 if
errors. Fix any errors before proceeding.

`anchor plan validate` emits a `note:` when a destination parent does not exist.
This exits 0 — missing parents are auto-created by apply.

---

## Step 3 — Preview

```sh
anchor diff my-plan.toml
```

Shows each operation and how many refs will be rewritten, without touching any files.
Safe to run repeatedly.

```sh
anchor diff --verbose my-plan.toml
```

Lists each file and each ref that will be rewritten.

---

## Step 4 — Apply

```sh
anchor apply my-plan.toml
```

Executes all ops in order. For each move, rewrites markdown cross-references. Prints
progress as it runs.

If post-rewrite validation fails, the entire plan rolls back — workspace unchanged.

---

## Rollback diagnostics

When apply rolls back, it prints per-ref diagnostics so you know exactly what failed:

```
BROKEN REFERENCES AFTER REWRITE (1):
  docs/guide.md:14  → old-folder/guide.md → new-folder/guide.md  (target not found)
    similar: new-folder/GUIDE.md
```

Each entry shows the file, line, the rewrite that was attempted, and the closest
match if one exists. Fix the plan or the source files, then re-apply.
