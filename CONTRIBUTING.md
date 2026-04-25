# Contributing to anchor

## Prerequisites

- Rust stable toolchain — [rustup.rs](https://rustup.rs)

## Build

```bash
cargo build
```

## Test

```bash
cargo test
```

## Lint

```bash
cargo clippy -- -D warnings
cargo fmt --check
```

## Submitting a PR

1. Branch from `main`:
   ```bash
   git checkout main && git pull --rebase origin main
   git checkout -b feat/your-slug
   ```
   Branch prefixes: `feat/`, `fix/`, `docs/`, `chore/`, `refactor/`
2. Make your changes
3. Run the full check suite:
   ```bash
   cargo fmt && cargo clippy -- -D warnings && cargo test && cargo build --release
   ```
4. Open a pull request — CI must pass before merge
5. Squash merge only

For install instructions, quick start, and command reference, see [README.md](README.md).

## Commit style

```
feat: add --format json to anchor file refs
fix: emit "No references found." on zero-result query
docs: add CONTRIBUTING.md
chore: replace test fixture paths with generics
refactor: extract OutputFormat enum
```

## Contributions welcome

- Bug fixes — open an issue first for anything non-trivial
- Documentation improvements
- Performance improvements with benchmarks
- New CLI flags that fit the existing command surface
- Workspace format extensions (discuss in an issue first)

Not accepted without prior discussion: new subcommands, changes to JSON output schema (breaking), changes to exit code contract.

## What NOT to commit

- Secrets, API keys, or credentials of any kind
- Hardcoded file system paths (use relative paths or workspace-relative paths)
- Internal development IDs — commit titles are for external readers. Do not use IDs like `MF-001`, `MX-008`, or similar internal tracking labels in commit messages or PR titles.
- State labels (`DONE`, `READY`, `FAILED`) — these belong in issue trackers, not commit history

## License

Apache 2.0 — contributions are accepted under the same license.
