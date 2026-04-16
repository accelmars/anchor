# Exit Codes

| Command | Exit 0 | Exit 1 | Exit 2 |
|---------|--------|--------|--------|
| `mind init` | Initialized successfully, or aborted by user | Path not found, not writable, no candidate, or I/O error | — |
| `mind root` | Workspace root found | No workspace found (run `mind init`) | System error |
| `mind file mv` | Move and rewrite complete | Conflicting flags | Src not found, dst exists, lock error, or system error |
| `mind file validate` | No broken references | Broken references found | System error |
| `mind file refs` | Always (zero references is not an error) | — | — |

## Notes

- `mind init` does not distinguish between user-facing errors (bad path) and I/O errors at the exit-code level — all non-abort errors exit 1.
- `mind file refs` always exits 0. A `count: 0` result means the file exists but has no inbound references; it is not a failure.
- Exit 2 indicates an unexpected system-level failure (permissions, I/O, corrupted workspace state).
