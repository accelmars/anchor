# Limitations

Known constraints and edge cases in anchor's current implementation.

---

## Operational limitations

**No rollback on partial failure**

If op 5 of 10 fails, ops 1–4 have already been applied and cannot be undone
automatically. Run `anchor file validate` to identify the state of the workspace,
then apply a corrective plan.

**Explicit `--path` bypasses parent workspace detection**

`anchor init --yes` (no `--path`) defaults to CWD safely. But if you pass
`--path ./subdir` explicitly, anchor does not check whether a parent directory
already has `.accelmars/` — it creates a nested workspace without warning. Check
for a parent workspace first:

```sh
ls ../.accelmars 2>/dev/null && echo "parent workspace exists — skip init"
```

**Wizard is a scaffold, not a complete authoring tool**

Designed for simple plans (up to ~5 items). For larger or complex plans: generate a
starting file with the wizard, edit it manually, then run `anchor plan validate`
before applying.

**Non-.md file paths not rewritten**

Paths in `config.json`, `settings.yaml`, `.ts`, `.py`, and other non-markdown files
are not rewritten. Anchor warns on stderr with a count after each move — treat it
as a manual cleanup checklist.

---

## Hard constraints

| Constraint | Description |
|------------|-------------|
| `.md` only | anchor only processes Markdown files. Other file types not tracked. |
| Explicit links only | Only `[text](path.md)` and associated forms. Plain prose paths are not detected. |
| `anchor init` required per machine | `.accelmars/anchor/` is not committed to git. Each machine requires `anchor init` once. |
| Same filesystem | `rename(2)` atomicity requires `.accelmars/anchor/` on the same filesystem as the workspace. Different mount = non-atomic. |
| Torn write recovery: manual | If killed mid-commit, inspect `manifest.json` and clean up `.accelmars/anchor/tmp/` manually. |
| Ambiguous wiki stems | If two `.md` files share the same stem, `anchor file mv` aborts. Resolve the ambiguity first. |
