
## [1.0.0] - 2026-08-13

### Features
- (**workspace**) [**BREAKING**] Honour the engine startup env contract before walking the filesystem (#132) ([#132](https://github.com/accelmars/anchor/pull/132))


### Bug Fixes
- (**init**) A-007 — bound detect_candidate at the OS temp root, isolate test tempdirs (#131) ([#131](https://github.com/accelmars/anchor/pull/131))


### Documentation
- (**improvement**) Stand up anchor's own in-repo issue register (#130) ([#130](https://github.com/accelmars/anchor/pull/130))

## [0.12.3] - 2026-07-28

### Bug Fixes
- (**validate**) An over-long backtick span must not abort the whole run (#128) ([#128](https://github.com/accelmars/anchor/pull/128))

## [0.12.1] - 2026-06-05

### Bug Fixes
- (**text_rename**) Unify apply/verify enumeration + fence/frontmatter detection (AENG-023) (#123) ([#123](https://github.com/accelmars/anchor/pull/123))

## [0.12.0] - 2026-06-05

### Features
- (**text_rename**) Cascade-safe ordered rule sets + per-line skip-list (#121) ([#121](https://github.com/accelmars/anchor/pull/121))
- (**text**) Add 'anchor text verify' term-absence completeness gate (#120) ([#120](https://github.com/accelmars/anchor/pull/120))
- (**resolver**) Normalize_external_path() for tilde/absolute path inputs (#115) ([#115](https://github.com/accelmars/anchor/pull/115))

## [0.11.0] - 2026-05-27

### Features
- Add anchor text find command for occurrence enumeration (#118) ([#118](https://github.com/accelmars/anchor/pull/118))
- Add text-rename op-type for content substitution (#117) ([#117](https://github.com/accelmars/anchor/pull/117))

## [0.10.0] - 2026-05-21

### Features
- (**os-env**) Inject default/ slug + derive Clone/Eq (OS-ARC22, unblocks pact) (#114) ([#114](https://github.com/accelmars/anchor/pull/114))

## [0.9.1] - 2026-05-21

### Bug Fixes
- (**apply**) Pre-broken classifier — position-only identity (AENG-020) (#113) ([#113](https://github.com/accelmars/anchor/pull/113))

## [0.9.0] - 2026-05-21

### Bug Fixes
- Existence guard for backtick refs — skip rewrite when target path was never a real workspace path, eliminating 100% false-positive rewrites on org-name/package-prefix collisions (#110) ([#110](https://github.com/accelmars/anchor/pull/110))
- Disable release-plz for both packages (release = false) (#109) ([#109](https://github.com/accelmars/anchor/pull/109))
- Let release-plz track accelmars-os-env — remove release=false to enable PR#2629 fallback for local-dep cargo package failures (#107) ([#2629, #107](https://github.com/accelmars/anchor/pull/2629))
- Add publish_no_verify = true — skip cargo package verify step for accelmars-os-env path dep not on crates.io (#106) ([#106](https://github.com/accelmars/anchor/pull/106))


### Documentation
- Update release-plz.toml comments — document workspace member exclusion rationale and v0.8.1 baseline (#105) ([#105](https://github.com/accelmars/anchor/pull/105))

## [0.8.1] - 2026-05-05

### Features
- Anchor apply batch pipeline — single workspace scan, forward/reverse virtual maps, intra-chain ref rewrites correct across N ops (#96) ([#96](https://github.com/accelmars/anchor/pull/96))
- Slug-scoped engine state — anchor reads/writes from `.accelmars/<slug>/anchor/` in integrated mode; all callers updated to pass engine_home (#94) ([#94](https://github.com/accelmars/anchor/pull/94))


### Bug Fixes
- Add version = \"0.1.0\" to resolver-env dep declaration — cargo package manifest validation requires version field on path deps (#102) ([#102](https://github.com/accelmars/anchor/pull/102))
- Exclude accelmars-resolver-env from release-plz — private workspace member causes cargo package failure on initial release detection (#101) ([#101](https://github.com/accelmars/anchor/pull/101))
- Inline accelmars-resolver-env — remove private git dep so release-plz cargo package succeeds (#99) ([#99](https://github.com/accelmars/anchor/pull/99))
- Release-plz CI — set publish = false in Cargo.toml so cargo package skips git dep version check (#98) ([#98](https://github.com/accelmars/anchor/pull/98))
- Release-plz CI — add version = "0.1.1" to accelmars-resolver-env git dep so cargo package resolves correctly (#97) ([#97](https://github.com/accelmars/anchor/pull/97))
- Anchor plan new -t writes an executable plan skeleton — valid [[ops]] TOML with multi-op examples instead of wizard DSL (#95) ([#95](https://github.com/accelmars/anchor/pull/95))


### Refactor
- Rename accelmars-resolver-env to accelmars-os-env — crate renamed to reflect OS-layer identity (#103) ([#103](https://github.com/accelmars/anchor/pull/103))
- Move resolver-env into workspace — proper crate at crates/resolver-env, publish = false (#100) ([#100, #99, #2629](https://github.com/accelmars/anchor/pull/100))

## [0.8.0] - 2026-05-04

### Features
- Multi-tenant workspace resolver — integrated mode with slug selection ladder, `anchor root` semantic update, and `anchor mode/tenants/tenant` discovery verbs (#93) ([#93](https://github.com/accelmars/anchor/pull/93))

## [0.7.3] - 2026-05-03

### Documentation
- Backfill CHANGELOG for v0.7.1 and v0.7.2 (#91) ([#91, #89](https://github.com/accelmars/anchor/pull/91))

## [0.7.1] - 2026-05-03

### Bug Fixes
- (**windows**) Gate nix-crate imports behind #[cfg(unix)] — anchor now compiles on Windows; cargo-dist can build all 5 platform binaries (#89) ([#89](https://github.com/accelmars/anchor/pull/89))

## [0.7.0] - 2026-05-02

### Features
- Scope_boundaries config replaces .anchorscope marker files — declare "scope_boundaries": ["foundations/*"] in config.json instead of placing empty marker files; prefix/* glob auto-includes new foundations (#83) ([#83](https://github.com/accelmars/anchor/pull/83))
- Prose-mention detection for backtick refs — arrow lines, state_log entries, moved/renamed keywords skip rewrite; [prose?] in diff --verbose; --allow-prose-rewrites reverts to prior behavior (#82) ([#82](https://github.com/accelmars/anchor/pull/82))
- Path-based field inference in add-required — inference-rules.toml auto-fills engine-class fields by folder position (provider: from stem in 15-providers/, type+pass_status from constants in 31-evals/) (#81) ([#81](https://github.com/accelmars/anchor/pull/81))
- Frontmatter migrate plan-file mode — `anchor frontmatter migrate <plan.toml>` applies add_field and set_field ops across multiple files atomically; --to remains fully supported (#80) ([#80](https://github.com/accelmars/anchor/pull/80))
- Non-markdown rewrite visibility and allow-broken override — diff --verbose lists non-MD rewrites under a dedicated header; apply --allow-broken suppresses acknowledged false-positive rollbacks with persistence to .accelmars/anchor/acked (#79) ([#79](https://github.com/accelmars/anchor/pull/79))
- Foundation-scoped rewrites and migrate frontmatter scaffold — .anchorscope gates cross-foundation prose rewrites; migrate --to 1 scaffolds unfrontmattered files with inferred title (#78) ([#78](https://github.com/accelmars/anchor/pull/78))


### Bug Fixes
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


