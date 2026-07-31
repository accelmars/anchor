# anchor improvement ledger

The in-repo register of anchor's own defects and friction. Modelled on
`booster/docs/improvement/LEDGER.md`, which is where the convention was set.

> **Why this file exists.** Anchor's findings lived only in fleet-level documents outside the
> repo (`os/current/reference/*`, the memory corpus). A defect recorded somewhere the engine's
> own readers never look is invisible: **absence of a register is not absence of defects.**
> That was the explicit finding in the 2026-07-30 consolidated engine register — anchor, orbit,
> crew and assay all had none — and this closes it for anchor.

> **How to use it.** Append an entry when a session hits anchor friction. Keep the reproduction
> concrete (actual command, actual output) — a finding nobody can reproduce cannot be fixed.
> When an entry ships, mark it `FIXED` with the version and **leave it in place**: the history of
> what broke is worth more than a short file.

> **The Status column is a claim with a date on it.** Booster's ledger read `OPEN` for all seven
> of its findings while its own sections read `FIXED`, because nobody re-read the table when the
> fixes landed. Every row therefore carries a **Re-checked** date, and a *recurrence* row is more
> trustworthy than a status flag. Re-check before quoting.

| # | Finding | Sev | Status | Found | Re-checked |
|---|---|---|---|---|---|
| A-001 | `apply` / `verify` / `diff` scan different file sets — two glob engines, and the root is mode-dependent | P0 | OPEN | 2026-07-13 | 2026-07-31 |
| A-002 | multi-tenant resolver (AENG-018) | P1 | OPEN | 2026-07-13 | 2026-07-31 |
| A-003 | `file mv --allow-existing-broken-refs` (AENG-019) | P1 | OPEN | 2026-07-13 | 2026-07-31 |
| A-004 | rewrites markdown links only — prose references and package names are left dangling | P1 | OPEN | 2026-07-13 | 2026-07-31 |
| A-005 | the closed layer was never migrated to the designed architecture | P2 | OPEN | 2026-07-13 | 2026-07-31 |
| A-006 | AENG-021 / AENG-022 follow-ups | P2 | OPEN | 2026-07-13 | 2026-07-31 |
| A-007 | **the test suite poisons itself** — it leaves a workspace at the TMPDIR root, which then makes parent-detection fail every later test; anchor's gate is permanently red after one run | P0 | FIXED | 2026-07-31 | 2026-07-31 |

---

## A-001 — `apply`, `verify` and `diff` scan different file sets

**Severity:** P0 · **Found:** 2026-07-13 · **Owner:** anchor engine

Two different glob engines are in play and the scan root depends on the mode, so the three verbs
can disagree about which files are even in scope. The failure mode is the dangerous one: `verify`
reports clean over a *smaller* set than `apply` actually rewrote, so a broken reference can pass
the gate.

Carried on the fleet register since 2026-07-13 as AENG-023, off the critical path (sustained).

**Acceptance:** one scan implementation, one root resolution, shared by all three verbs; a test
that rewrites a file only one engine would have matched and proves `verify` sees it.

---

## A-007 — the test suite poisons itself; anchor's gate is permanently red

**Severity:** P0 · **Found:** 2026-07-31 · **Fixed:** 2026-07-31 · **Owner:** anchor engine

### Fix

Two changes, both in `src/cli/init.rs`:

1. **`detect_candidate` now stops at the OS temp root instead of walking into or past it.**
   Anything sharing a temp tree is ephemeral, unrelated process state — another tool's scratch
   clone, this binary's own earlier test runs — never the caller's real project ancestry. The walk
   now canonicalizes each ancestor and returns "no candidate" the moment it would inspect
   `std::env::temp_dir()` itself, rather than continuing to test its children for git repos. This
   is the acceptance-criterion-2 fix: `init --yes` with no real candidate now reliably lands at the
   path the caller passed, even with a stray workspace or repo sitting in the temp tree.
2. **The test suite's own tempdirs are now scoped under one private, per-process root**
   (`isolated_root()` / `test_tempdir()`, a `OnceLock<TempDir>` lazily created once and dropped —
   and therefore actually cleaned up — at process exit), instead of each test calling
   `tempfile::tempdir()` directly against the shared OS temp root. This is acceptance-criterion-1:
   the suite can no longer write above its own fixtures, and whatever it does write is guaranteed
   to be reclaimed when the test binary exits, not left behind permanently like the original
   `$TMPDIR/.accelmars`.

A new regression test, `test_stray_temp_tree_repo_is_never_a_candidate`, plants a real git repo
directly in the OS temp directory and proves `detect_candidate` still returns "no candidate" —
inducing the exact failure the fix exists to prevent, not just asserting the happy path.

**Verified:** `cargo test cli::init` — 18/18 pass; `cargo test` (full suite) — 523/523 pass, 0
fail; three consecutive `cargo test cli::init` runs with no cleanup in between all stay green, and
`$TMPDIR/.accelmars` never reappears.

**Nothing can be delivered to this repo.** `booster deliver` runs the test suite as a pre-push
gate, and the suite is red on `main` — so every delivery, including a docs-only one, is refused.

### The reproduction

```
$ cargo test cli::init          # on a machine that has never run it
test result: FAILED. 15 passed; 2 failed

$ cargo test cli::init          # immediately again, nothing changed
test result: FAILED. 11 passed; 6 failed
```

It gets **worse on every run and never recovers**. The cause:

```
$ ls -a "$TMPDIR/.accelmars"
.  ..  anchor  config.json
```

The suite leaves a real workspace at the **root of the shared OS temp directory**. Anchor's
parent-workspace detection — added in #53/#59 precisely to *prevent silent nesting* — then finds
that workspace above every `tempfile::tempdir()` and refuses to initialise inside it. So the
safety feature is working exactly as designed, on an artifact the tests themselves created.

Confirmed by deleting it: 6 failures drop to 2, and the directory is **recreated by the next
run**, taking it back to 6.

### Why it matters beyond the tests

The same walk-upward is live in the product. `anchor init --yes` in a directory with no detected
candidate is documented to "default to CWD" — but under an ancestor that happens to be a
workspace, the two remaining failures show it does not land where the caller asked. A user whose
home or temp tree contains a stray workspace can have `init` resolve somewhere they did not
choose. This is the same family as A-002's ambiguous resolution.

### Workaround

`rm -rf "$TMPDIR/.accelmars"` before running the suite. It will come back.

### Acceptance

1. The suite scopes `TMPDIR` per-run (or asserts on the resolved root) so it cannot write above
   its own fixtures — a test that mutates shared machine state is not isolated.
2. `init --yes` with no candidate lands at the path the caller passed, and a test proves it by
   running with an ancestor workspace present.
3. Both of the two clean-machine failures go green, and a second consecutive run stays green —
   the second run is the real assertion.

---

## A-002 — multi-tenant resolver (AENG-018)

**Severity:** P1 · **Found:** 2026-07-13

`anchor root` is the fleet's one-command check for which substrate governs a repo, and the answer
has to be unambiguous. Two live cases where it is not:

- a stray `~/.accelmars/` at the home root (tenants `accelmars` / `default` / `citadel`), mostly
  empty and disconnected from the real substrate at `~/accelmars/.accelmars/accelmars`;
- `pact-engine/.accelmars` holding **two** tenants (`accelmars`, `default`), which makes every
  `booster deliver` in that repo abort until `BOOSTER_TENANT` is set by hand — hit twice on
  2026-07-31.

**Engine-side ask:** ambiguous resolution should be a **loud refusal naming both candidate roots**,
never a silent pick and never a silent self-register failure.

---

## A-003 — `file mv --allow-existing-broken-refs` (AENG-019)

**Severity:** P1 · **Found:** 2026-07-13

A move into a tree that already contains broken references is refused wholesale, so a repo cannot
be incrementally repaired: you must fix every pre-existing break before making any move.

---

## A-004 — rewrites markdown links only

**Severity:** P1 · **Found:** 2026-07-13

Reference rewriting covers markdown link syntax. Prose mentions of a path, and package names, are
untouched — so `anchor validate` can pass while the document still tells a reader to look
somewhere that no longer exists. This is the same class as the persona→guild rename's ~2843
deferred prose references.

---

## A-005 — the closed layer was never migrated to the designed architecture

**Severity:** P2 · **Found:** 2026-07-13

Recorded so it is not rediscovered as a surprise.

---

## A-006 — AENG-021 / AENG-022 follow-ups

**Severity:** P2 · **Found:** 2026-07-13

Carried from the fleet register; detail lives with the original AENG entries.

---

_AccelMars Co., Ltd._
