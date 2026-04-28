# Changelog

All notable changes to anchor are documented here.
Format: [Keep a Changelog](https://keepachangelog.com/)

## [Unreleased]

## [0.5.0] — 2026-04-28

### Added
- Pre-move source validation gate: `anchor file mv` now validates all relative references
  in source files before rewriting. Broken source refs abort with `BROKEN REFERENCES IN SOURCE`
  showing the original file, line, and path — not the post-rewrite adjusted path.

### Fixed
- Gap 1: Partial-path backtick refs (e.g. `` `projects/os-council/...` ``) now matched and rewritten
  during moves, not just full workspace-relative paths.
- Gap 2: Backtick refs prefixed with `` `$(anchor root)/` `` now matched and rewritten.
- Gap 3: Relative backtick paths (e.g. `` `../os/...` ``) now resolved relative to the
  containing file before matching — consistent with Form 1 markdown link behavior.
- Gap 4: `anchor file validate` no longer reports false positives on valid relative backtick paths.
- Gap 5: Files inside a moved directory that contain `` `$(anchor root)/...old-path...` ``
  self-references are now updated.

## [0.4.0] — 2026-04-27

### Added
- **Backtick path rewriting** — `` `path/to/file` `` inline-code spans in `.md` files are tracked and rewritten on move (AR-001)
- **HTML href rewriting** — `<a href="path">` links tracked and rewritten on move; link text that mirrors the path string updates in sync (AR-004)
- **`anchor validate` alias** — top-level `anchor validate` runs the same check as `anchor file validate` (AR-005)
- **`anchor diff --verbose`** — new flag prints per-file, per-ref lines showing exactly what will be rewritten (AR-007)
- **EXIT-CODES.md** — new reference doc covering exit codes for all 5 commands (AR-007)
- **Wizard scaffold guidance** — `anchor plan new` prints an intro explaining the wizard is a scaffold, with `Tip:` and `Validate:` in the post-write hint block (AR-007a)
- **`anchor plan new --template`** — selects a plan template directly without launching the wizard (AR-012)

### Fixed
- **Multi-level relative refs** — `../../deep/path` style references now resolve correctly across sequential directory moves (AR-002)
- **Workspace init parent detection** — `anchor init` in a directory inside an existing workspace warns and aborts by default; `--path` flag proceeds with explicit warning (AR-003)
- **`anchor init --yes` CWD default** — defaults to current directory when no workspace candidate is detected, rather than failing (AR-003)
- **Non-.md file rewriting** — JSON/YAML/TOML/TS/JS/PY files containing the old path are rewritten in-place on `anchor file mv` and `anchor apply`; stderr reports count of files updated (AR-010)
- **Zero-ref plain-text warning** — moving a file with 0 detected refs warns when plain-text `.md` mentions of the old path exist (AR-005)
- **batch-move `create_dir` prompt** — the batch-move wizard asks to add `create_dir` ops for missing destination parents (AR-006)
- **`anchor plan validate` dst-parent note** — validates report a `note:` when a destination parent directory does not exist (exit 0) (AR-006)
- **Re-apply detection** — applying an already-applied plan emits a "may have already been applied" hint instead of silently doing nothing (AR-007)
- **Exit code corrections** — `anchor diff` without a workspace now exits 1 (user config error) instead of 2 (AR-007)
- **`anchor init --path` parent note** — `anchor init --path <child>` warns about an existing parent workspace but proceeds (exit 0) (AR-011)

## [0.3.0] — 2026-04-26

### Added
- `anchor plan validate <plan.toml>` — pre-flight validation (AP-001)
- `anchor serve [--port N]` — HTTP server with `/health` and `/file/validate` endpoints (AP-004)
- `pub fn routes()` + `pub fn build_state()` — platform composition interface (AP-005)
- `anchor recover` — torn-write recovery for stale tmp directories (AP-007)
- YAML frontmatter reference detection — `anchor file validate` now detects broken Path Anchors (AP-002)
- TOML config reference detection — `.toml` files scanned for path references (AP-003)

### Fixed
- Cross-filesystem moves now complete via copy+delete fallback instead of failing with EXDEV (AP-006)

## [0.2.0] — 2026-04-25

### Changed

- Binary renamed from `mind` to `anchor`; crate renamed from `accelmars-mind` to `accelmars-anchor`
- Repository renamed from `accelmars/mind-engine` to `accelmars/anchor`
- Acked file path: `.mindacked` → `.accelmars/anchor/acked`
- Repo made public under Apache 2.0

## [0.1.1] — 2026-04-17

### Fixed

- `anchor file mv` now removes empty `.mind/tmp/` directory after a successful operation

### Added

- `--version` flag to print the installed version

## [0.1.0] — 2026-04-16

### Added

- `anchor init` — initialize workspace with guided wizard; `--yes` and `--path` flags for non-interactive use
- `anchor root` — print workspace root (machine-readable, stable output format)
- `anchor file mv <src> <dst>` — reference-safe move with atomic transaction; `--verbose` and `--format json` output modes
- `anchor file validate` — scan workspace for broken references; `--format json` output mode
- `anchor file refs <file>` — list files referencing a given file; `--format json` output mode; zero-result disambiguation
- `.accelmars/anchor/ignore` — gitignore-compatible exclusion patterns for scanner (default: `node_modules/`, `target/`)
- `.accelmars/anchor/acked` — gitignore-compatible acknowledgement patterns to suppress known broken references from validate output
