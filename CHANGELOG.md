# Changelog

All notable changes to mind-engine are documented here.
Format: [Keep a Changelog](https://keepachangelog.com/)

## [Unreleased]

## [0.1.1] — 2026-04-17

### Fixed

- `mind file mv` now removes empty `.mind/tmp/` directory after a successful operation (MX-008)

### Added

- `--version` flag to print the installed version

## [0.1.0] — 2026-04-16

### Added

- `mind init` — initialize workspace with guided wizard; `--yes` and `--path` flags for non-interactive use
- `mind root` — print workspace root (machine-readable, stable output format)
- `mind file mv <src> <dst>` — reference-safe move with atomic transaction; `--verbose` and `--format json` output modes
- `mind file validate` — scan workspace for broken references; `--format json` output mode
- `mind file refs <file>` — list files referencing a given file; `--format json` output mode; zero-result disambiguation
- `.mindignore` — gitignore-compatible exclusion patterns for scanner (default: `node_modules/`, `target/`)
- `.mindacked` — gitignore-compatible acknowledgement patterns to suppress known broken references from validate output
