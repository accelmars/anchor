# Excluding Files

Place an ignore file at `.accelmars/anchor/ignore` in your workspace root. Uses the
same pattern syntax as `.gitignore`.

```
# .accelmars/anchor/ignore
node_modules/
target/
build/
```

`anchor init` writes a default ignore file with `node_modules/` and `target/`.

---

## Effect

Files and directories matching ignore patterns are excluded from:

- Reference scanning (`anchor file validate`, `anchor diff`, `anchor apply`)
- Inbound ref detection (`anchor file refs`)

They are **not** excluded from being moved — `anchor file mv` and `anchor apply`
will still move matched source paths. The exclusion applies only to scanning.

---

## Acknowledged broken refs

For broken refs you know about but are not ready to fix, place patterns in
`.accelmars/anchor/acked`. Acknowledged refs are suppressed from
`anchor file validate` output and do not affect the exit code.

See [REFERENCE-HEALTH.md](REFERENCE-HEALTH.md) for details on the validate command.
