
## [Unreleased]

## [0.7.0] - 2026-05-03

### Added
- (**AENG-010**) `anchor file mv` now detects backtick path refs in prose context (arrow lines `→`/`->`, `state_log` entries, moved/renamed/previously keywords) and skips them; skipped candidates shown as `[prose?]` entries in `anchor diff --verbose` and counted `(+N prose? skipped)` in non-verbose output; `--allow-prose-rewrites` flag reverts to previous behavior (rewrite all backtick refs).
- (**AENG-012**) `anchor frontmatter add-required --batch` reads `inference-rules.toml` from the active template and auto-fills engine-class fields by folder position (`provider: <stem>` in `15-providers/`, `type: eval` + `pass_status: NOT_RUN` from constants in `31-evals/`). Fill-if-absent only — existing values are never overwritten. Template absent or unparseable: inference silently skipped.
- (**AENG-003**) `anchor apply --allow-broken=<file>:<line>` (repeatable) and `--allow-broken-from=<path>` suppress acknowledged false-positive rollbacks; acked refs persist to `.accelmars/anchor/acked` on success.
- (**AENG-004**) `anchor diff --verbose` now lists non-MD file rewrites with file path and detail (`path/to/config.toml  \`old\` → \`new\``), grouped under a `## Non-markdown rewrites (N file(s))` section after MD entries. Non-verbose output unchanged.
- (**AENG-006-scaffold**) `anchor frontmatter migrate --to 1` now scaffolds files with no frontmatter; title inferred from first `# Heading` or filename stem.
- (**AENG-006-plan**) `anchor frontmatter migrate <plan.toml>` now accepts a TOML plan file as its path argument (no `--to` needed); supports `add_field` and `set_field` ops across multiple files in a single atomic pass; dry-run by default, `--apply` to write. `--to` remains fully supported and takes precedence over plan-file detection.

### Changed
- (**AENG-001** refinement) `scope_boundaries` config replaces `.anchorscope` filesystem walk; add `"scope_boundaries": ["foundations/*"]` to `.accelmars/anchor/config.json` instead of placing marker files. `ScopeResolver` now reads boundaries from config — supports `prefix/*` glob (direct children) and literal paths. Workspaces without `scope_boundaries` key fall back to v0.6.0 Repo scope unchanged.

### Bug Fixes
- (**refs**) AENG-001 (complete) — foundation-scoped rewrites via `.anchorscope`. `ScopeResolver` now discovers `.anchorscope` marker files recursively under the workspace root and `scope_for_move()` returns `RewriteDomain::Defined(<deepest .anchorscope ancestor>)` when one contains the move source. This eliminates the cross-foundation prose corruption observed in the gateway-engine Pass 1 run (2026-05-01, v0.6.0): sibling foundations are now out of scope unless they hold a workspace-relative inward ref. Workspaces without `.anchorscope` markers fall back to v0.6.0 `Repo` scope — fully backward compatible.
- Anchor frontmatter no longer hardcodes accelmars-workspace/ defaults — schema resolution uses .accelmars/anchor/frontmatter-schema.json fallback with explicit error; test fixtures genericized; CI boundary guard added (#72) ([#72](https://github.com/accelmars/anchor/pull/72))


### Documentation
- Documentation restructure — README slimmed to essentials; QUICK-START replaced by seven focused guides (COMMAND-REFERENCE, PLAN-WORKFLOW, TYPICAL-SESSION, REWRITES, REFERENCE-HEALTH, IGNORE, LIMITATIONS); Apache 2.0 license text corrected (#74) ([#74](https://github.com/accelmars/anchor/pull/74))
- README restructure, QUICK-START.md, and CI path fix — command reference table, typical session walkthrough, per-command exit codes; frontmatter-schema-check updated for accelmars-standard rename (#73) ([#73](https://github.com/accelmars/anchor/pull/73))

## [0.6.0] - 2026-04-29

### Features
- Post-apply UX-001 surfaces partial-path plain-text remainder — bare-prose occurrences of path segments (e.g. 'os-council') reported per-file with counts after every anchor apply (#68) ([#68](https://github.com/accelmars/anchor/pull/68))
- Rollback diagnostics and `anchor frontmatter` subcommand family — failing refs named on rollback; audit, migrate, normalize, add-required, and check-schema commands; CI schema drift guard (#67) ([#67](https://github.com/accelmars/anchor/pull/67))


### Bug Fixes
- (**refs**) Context-scoped reference rewrite — common-noun folder renames (e.g. `workflows/`) no longer rewrite unrelated occurrences across sibling git repos; inward workspace-relative refs still rewritten correctly (#70) ([#70](https://github.com/accelmars/anchor/pull/70))
- (**refs**) Exclude fenced code blocks from ref scanning — FenceState/FenceMarker state machine with marker-type and length matching (#69) ([#69](https://github.com/accelmars/anchor/pull/69))


### Documentation
- Update relative path example in CHANGELOG.md (#66) ([#66](https://github.com/accelmars/anchor/pull/66))

## [0.5.0] - 2026-04-28

### Features
- Expand backtick ref coverage and add pre-move validation — partial-path, $(anchor root)/ prefix, relative-path, and internal self-ref rewrites; broken-source-ref gate; validate false-positive fix (#64) ([#64](https://github.com/accelmars/anchor/pull/64))


### Bug Fixes
- Test isolation for CWD-sensitive tests — CWD_MUTEX serializes the subdir mv test; default output path test writes to TempDir instead of process CWD (#63) ([#63](https://github.com/accelmars/anchor/pull/63))
- Workspace-root-anchored ignore patterns and plan file self-modification — node_modules/ in .accelmars/anchor/ignore now matches the workspace root; anchor apply no longer rewrites the active plan file (#62) ([#62](https://github.com/accelmars/anchor/pull/62))
- Eliminate test race condition in anchor file mv — extract run_impl with injected workspace_root and cwd; parallel tests no longer mutate global process state (#61) ([#61](https://github.com/accelmars/anchor/pull/61))

## [0.4.0] - 2026-04-27

### Features
- Non-.md file rewriting, init --path parent detection fix, and plan new --template — JSON/YAML/TS/JS/PY occurrences rewritten on move; --path warns when inside parent workspace; plan new --template selects a plan template without the wizard (#59) ([#59](https://github.com/accelmars/anchor/pull/59))
- Wizard scaffold-first UX — intro blurb and Tip:/Validate: hints guide operators to the scaffold-then-edit pattern (#58) ([#58](https://github.com/accelmars/anchor/pull/58))
- Diff --verbose, re-apply hint, and exit code corrections — `anchor diff --verbose` lists each file and ref to be rewritten; double-apply detected with a helpful hint; five commands documented in EXIT-CODES.md (#57) ([#57](https://github.com/accelmars/anchor/pull/57))
- Batch-move create_dir prompt and plan validate dst-parent note — `batch-move` wizard asks to add a `create_dir` op for missing destination parents; `anchor plan validate` emits a note when dst parent does not exist (exit 0) (#56) ([#56](https://github.com/accelmars/anchor/pull/56))
- Anchor validate shorthand and zero-ref plain-text warning — top-level `anchor validate` runs reference check (alias for `anchor file validate`); moves that rewrite 0 refs now warn when plain-text .md mentions of the old path remain (#55) ([#55](https://github.com/accelmars/anchor/pull/55))
- HTML href rewriting and link-text sync — <a href="path"> links tracked and rewritten on move; link text that mirrors the path string updates in sync (#54) ([#54](https://github.com/accelmars/anchor/pull/54))
- Backtick inline-code path rewriting — `path/` spans in .md files rewritten on move; non-.md files with occurrences flagged via stderr warning (#52) ([#52](https://github.com/accelmars/anchor/pull/52))


### Bug Fixes
- Workspace init safety — parent workspace detection prevents silent nesting; --yes defaults to CWD when no candidate detected (#53) ([#53](https://github.com/accelmars/anchor/pull/53))

## [0.3.0] - 2026-04-26

### Features
- Anchor recover — inspect stale tmp dirs after a crash, roll back pre-commit ops automatically, and warn on partial commits with manual resolution steps (#49) ([#49](https://github.com/accelmars/anchor/pull/49))
- Cross-filesystem move fallback — detect EXDEV in COMMIT phase and fall back to copy+delete for files and directories (#48) ([#48](https://github.com/accelmars/anchor/pull/48))
- Axum HTTP server and platform composition interface — `anchor serve` exposes GET /health and POST /file/validate; `routes()` + `build_state()` exported for platform binary composition (#47) ([#47](https://github.com/accelmars/anchor/pull/47))
- TOML config reference detection — anchor file validate reports broken paths in .toml files (#46) ([#46](https://github.com/accelmars/anchor/pull/46))
- YAML frontmatter path reference detection — `anchor file validate` reports broken `$(anchor root)/` paths in .md frontmatter blocks (#44) ([#44](https://github.com/accelmars/anchor/pull/44))
- \`anchor plan validate\` — validates src existence and dst absence before apply; completes the diff → validate → apply pre-flight workflow (#42) ([#42](https://github.com/accelmars/anchor/pull/42))


### Bug Fixes
- CWD-relative path resolution in \`anchor file mv\` — src and dst now resolve relative to the caller's directory, matching standard Unix mv behavior (#50) ([#50](https://github.com/accelmars/anchor/pull/50))

## [0.2.1] - 2026-04-26

### Features
- Inline 'similar:' hint on broken refs — anchor file validate shows closest workspace match under each broken ref; JSON schema unchanged (#31) ([#31](https://github.com/accelmars/anchor/pull/31))
- 'Did you mean?' suggestions on missing-path errors — anchor file mv SrcNotFound, anchor file refs absent target, and anchor init --path DirectoryNotFound all surface suggest_similar output (#29) ([#29](https://github.com/accelmars/anchor/pull/29))
- Suggest_similar utility — basename-aware "Did you mean?" suggestions with Levenshtein threshold and prefix ranking (#28) ([#28](https://github.com/accelmars/anchor/pull/28))
- Anchor plan list command — lists built-in, workspace, and user templates in three sections (#27) ([#27](https://github.com/accelmars/anchor/pull/27))
- Plan adapter integration tests — 5 end-to-end tests covering diff (read-only), apply (create dir + move + ref rewrite), pre-flight rejection, stop-and-report failure, and wizard scaffold output (#25) ([#25](https://github.com/accelmars/anchor/pull/25))
- Anchor plan new wizard — 5 built-in templates (batch-move, categorize, archive, rename, scaffold) generate plan TOML without manual editing (#24) ([#24](https://github.com/accelmars/anchor/pull/24))
- Anchor apply command — pre-flight validates all Move ops before execution; sequential per-op transactions; Stopped after M/N and Done. N/N progress output
- Anchor diff command — read-only plan preview with per-op ref counts, CreateDir existence checks, and similar-path suggestions for missing sources (#23) ([#23](https://github.com/accelmars/anchor/pull/23))
- TOML plan file model — Plan and Op types shared by anchor apply, diff, and plan new; version enforcement and human-readable render_plan_toml (#22) ([#22](https://github.com/accelmars/anchor/pull/22))


### Bug Fixes
- Cliff.toml header — clear header field to prevent duplicate CHANGELOG header in release-plz PRs (#40) ([#40, #39](https://github.com/accelmars/anchor/pull/40))
- Cliff.toml tag_pattern — match accelmars-anchor-v* format used by release-plz (#36) ([#36](https://github.com/accelmars/anchor/pull/36))
- CODEOWNERS — use @accelmars directly; personal account has no org teams (#34) ([#34](https://github.com/accelmars/anchor/pull/34))
- Clippy needless_range_loop in levenshtein (apply.rs) — use iter_mut().enumerate()
- Acked suppression reads .accelmars/anchor/acked — silent failure on fresh workspaces resolved; remaining mind→anchor renames in docs, headers, and test helpers (#21) ([#21](https://github.com/accelmars/anchor/pull/21))


### Documentation
- Add architecture overview and contributor guidance — README explains workspace model and anti-use-cases; CONTRIBUTING scopes accepted contributions and commit hygiene; internal ID removed from CHANGELOG 0.1.1 (#32) ([#32](https://github.com/accelmars/anchor/pull/32))
- Add security policy — SECURITY.md with vulnerability reporting contact, 48-hour SLA, and scope definition (#30) ([#30](https://github.com/accelmars/anchor/pull/30))

## [v0.2.0] - 2026-04-25

### Features
- Workspace discovery rewrite — .mind-root → .accelmars/


### Bug Fixes
- Rename remaining mind references to anchor in doc comments and README


### Documentation
- Update command reference in README to use anchor alias instead of mind
- Promote [Unreleased] to [0.1.1] in CHANGELOG (#20) ([#20](https://github.com/accelmars/anchor/pull/20))

## [0.1.1] - 2026-04-17

### Features
- Add --verbose and --format json to mind file mv and validate (#16) ([#16](https://github.com/accelmars/anchor/pull/16))
- Mind init hardening — step indicator, error retry, --yes, --path (#15) ([#15](https://github.com/accelmars/anchor/pull/15))
- Refs zero-result disambiguation + --format json (MX-002)
- (**MF-010**) .mindacked acknowledged refs for mind file validate
- (**MF-009**) .mindignore pattern exclusions for scanner
- (**MF-006**) Mind file mv command
- (**MF-007**) Implement mind file validate + mind file refs (#8) ([#8](https://github.com/accelmars/anchor/pull/8))
- (**MF-005**) Transaction infrastructure — lock, temp, manifest, PLAN phase (#6) ([#6](https://github.com/accelmars/anchor/pull/6))
- (**MF-004**) Implement resolver and canonical path model (#5) ([#5](https://github.com/accelmars/anchor/pull/5))
- (**MF-003**) Implement scanner, reference parser, and Reference model (#4) ([#4](https://github.com/accelmars/anchor/pull/4))
- Implement mind init wizard with atomic writes and Phase 2 bridge guards (#2) ([#2](https://github.com/accelmars/anchor/pull/2))
- Scaffold mind-engine repo with CLI structure and mind root command (#1) ([#1](https://github.com/accelmars/anchor/pull/1))


### Bug Fixes
- Remove empty .mind/tmp/ after successful op dir cleanup (#18) ([#18](https://github.com/accelmars/anchor/pull/18))
- Eliminate set_current_dir race in workspace tests
- Set binary name to 'mind' per 01-OVERVIEW.md spec (#3) ([#3](https://github.com/accelmars/anchor/pull/3))


### Documentation
- Add CONTRIBUTING.md, CHANGELOG.md, and EXIT-CODES.md (#17) ([#17](https://github.com/accelmars/anchor/pull/17))
- Write complete public README for v0.1.0 Phase 1 release


