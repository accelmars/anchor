# Changelog

All notable changes to anchor are documented here.
Format: [Keep a Changelog](https://keepachangelog.com/)

## [Unreleased]

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
