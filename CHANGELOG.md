# Changelog

All notable changes to anchor are documented here.
Format: [Keep a Changelog](https://keepachangelog.com/)

## [Unreleased]

## [0.2.1] — 2026-04-25

### Added

- `anchor diff <plan.toml>` — preview plan operations (read-only; no workspace changes)
- `anchor apply <plan.toml>` — execute a plan atomically via PLAN→APPLY→VALIDATE→COMMIT engine
- `anchor plan new` — interactive wizard: select a template, fill parameters, produce a `.plan.toml` file
- `anchor plan list` — list available plan templates (built-in, workspace, user) with three-section output
- `anchor init --path <dir>` — `--path` flag for non-interactive workspace creation outside git root
- suggest_similar utility (`src/core/suggest.rs`) — Levenshtein + basename "Did you mean?" on all path failures
- anchor-engine crate scaffold — AI translation adapter (Gateway integration, retry, circuit-breaker), schema library (5 plan schemas + TEMPLATE_PATTERNS)
- 5 Pareto plan templates embedded as TOML (`batch-move`, `categorize`, `archive`, `rename`, `scaffold`)
- Cross-binary integration test suite (`tests/anchor_engine_integration.rs`)

### Changed

- `anchor file mv`: returns error with similar-path suggestions when source not found (instead of process::exit)
- `anchor file validate`: shows inline `  similar: {top-match}` hint per broken reference
- `anchor file refs`: exits 2 with similar-path suggestions when target path not in workspace

### Fixed

- `anchor file mv`: directory copies no longer hang on symlinks to ancestor directories — `entry.file_type()` replaces `entry.is_dir()` so symlinks are skipped (FS-004)

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
