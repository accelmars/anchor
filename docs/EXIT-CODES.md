# Exit Codes

| Command | Exit 0 | Exit 1 | Exit 2 |
|---------|--------|--------|--------|
| `anchor init` | Initialized successfully, or aborted by user | Path not found, not writable, no candidate, or I/O error | — |
| `anchor root` | Workspace root found | No workspace found (run `anchor init`) | System error |
| `anchor file mv` | Move and rewrite complete | Conflicting flags | Src not found, dst exists, lock error, or system error |
| `anchor file validate` | No broken references | Broken references found | System error |
| `anchor file refs` | Always (zero references is not an error) | — | — |

| `anchor apply` | All operations completed | Op/preflight failure | Workspace/infra error |
| `anchor diff` | Preview shown (0 or more ops) | Plan parse error or no workspace found | Scan/I/O error |
| `anchor plan validate` | Plan is valid | Validation failures (src/dst errors) | Plan parse error |
| `anchor plan new` | Plan file written | Invalid input or write failure | — |
| `anchor plan list` | Templates listed (always) | — | — |

## Notes

- `anchor init` does not distinguish between user-facing errors (bad path) and I/O errors at the exit-code level — all non-abort errors exit 1.
- `anchor file refs` always exits 0. A `count: 0` result means the file exists but has no inbound references; it is not a failure.
- Exit 2 indicates an unexpected system-level failure (permissions, I/O, corrupted workspace state).
- `anchor diff` exits 1 for no-workspace (user config error), not 2.
