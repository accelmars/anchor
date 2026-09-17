# anchor

**Move Markdown files without breaking the links that point to them.**

[![License: Apache 2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Release](https://img.shields.io/github/v/release/accelmars/anchor?sort=semver)](https://github.com/accelmars/anchor/releases)

anchor is `mv` for Markdown workspaces.

You move a file. anchor moves it, finds the Markdown links pointing at the old location,
and updates them.

That's it.

---

## Example

You have:

```text
notes/database.md
projects/app.md      contains:  See [database notes](../notes/database.md).
README.md            contains:  - [db](notes/database.md)
```

Move the note:

```sh
anchor mv notes/database.md archive/database.md
```

```text
Moved notes/database.md → archive/database.md
Updated 2 links in 2 files.
```

Both links now point at `archive/database.md`. You did not have to know they existed.

---

## Install

```sh
cargo install --git https://github.com/accelmars/anchor --tag accelmars-anchor-v2.1.0
```

Requires Rust 1.70+. The installed command is `anchor`.

anchor is not on crates.io and there is no plan to publish it there — install from source
with the command above. It pulls one AccelMars crate,
[`accelmars-os-env`](https://github.com/accelmars/os-env), by git tag; that repository is
public precisely so this install works.

---

## Set up, once

```sh
anchor init --path .
```

anchor operates on a **workspace** — one root directory, and everything beneath it.
`init` records where that root is.

Run it without `--path` and anchor will *guess* the root by walking upward, which is right
for a multi-repository workspace and wrong for a single project: it can select a parent
directory you did not mean. **For one project, pass `--path .`** so the root is the
directory you are standing in. `anchor root` prints the answer at any time.

This creates `.accelmars/anchor/` with local config. It is not committed; run `init` once
per machine.

---

## The three commands most people need

### Move something

```sh
anchor mv <from> <to>
```

Moves a file or directory and rewrites the Markdown links pointing at it.

```text
Moved notes/database.md → archive/database.md
Updated 2 links in 2 files.
```

If nothing linked to it, anchor says so rather than staying quiet:

```text
Moved notes/scratch.md → archive/scratch.md
No links pointed to it.
```

### Check the workspace

```sh
anchor check
```

```text
✓ 3 files scanned. No broken references.
```

If something is broken, anchor prints each one with its file and line, and exits non-zero
so a script can act on it.

### See what links to a file

```sh
anchor refs notes/database.md
```

```text
References to: notes/database.md

  projects/app.md:3
  README.md:3

2 files reference this file.
```

Worth running before you reorganize something important.

---

## That's enough for normal use

```text
move files  →  anchor fixes links  →  keep working
```

You do not need to write a plan for an ordinary move. You do not need to search and replace
paths by hand. You do not need to know how any of it works.

Directories work the same way:

```sh
anchor mv notes/database archive/database
```

---

## Bigger reorganizations

Moving one thing? `anchor mv`.

Moving many things that should be reviewed and applied as a single change? anchor has a
plan workflow:

```text
write plan → validate → preview → apply
```

```sh
anchor plan new
anchor diff moves.toml
anchor apply moves.toml
```

Plans exist so a large reorganization can be inspected before any file moves. You do not
need them for normal use. → [docs/PLAN-WORKFLOW.md](docs/PLAN-WORKFLOW.md)

---

## What anchor changes

anchor understands Markdown links:

```markdown
[Database notes](../notes/database.md)
```

When the target moves, it rewrites the references it understands. It does **not** perform
arbitrary search-and-replace, so a stray mention of a path in prose is left alone.

For exactly what is and is not rewritten, see [docs/REWRITES.md](docs/REWRITES.md).

---

## When to use anchor

You have a Markdown workspace with links between files — `docs/`, `notes/`, `wiki/`,
`decisions/` — and reorganizing it makes you wonder *"what links to this?"*

## When not to use anchor

| Instead of anchor | Use |
|---|---|
| Moving source code | `git mv` — anchor is not a code refactoring tool |
| Moving images or binaries | your normal filesystem or Git tools |
| Global text replacement | `sed`, or your editor |
| A repo with no Markdown cross-links | plain `mv` — anchor would add nothing |

---

## Safety

anchor should make reorganizing boring.

Before changing anything it validates the operation. After moving, it rewrites the affected
references and verifies the result. If it cannot complete the operation safely it rolls back
and tells you, rather than guessing — a move that would leave a broken reference behind
fails instead of half-finishing.

For a large or unusual reorganization, use the plan workflow to preview the whole change first.

---

## Local by default

anchor runs on your machine. It collects no telemetry. No workspace data is sent to AccelMars.

`anchor serve` exposes a small read-only HTTP API for local tools; it binds loopback only.
See [the HTTP server](#the-http-server) below.

---

## Advanced

Most people never need these.

| | |
|---|---|
| `anchor plan …` | preview and apply multi-step reorganizations → [docs/PLAN-WORKFLOW.md](docs/PLAN-WORKFLOW.md) |
| `anchor frontmatter …` | audit, migrate, normalize and validate Markdown frontmatter |
| `anchor serve` | local HTTP API |
| `anchor text find` | text inspection primitives |
| `anchor recover` | recover from an interrupted operation |

### The HTTP server

```sh
anchor serve --port 3000
```

Two endpoints: `GET /health` and `POST /file/validate` (no body; returns the broken
references it finds). Both are **read-only** — the HTTP API cannot move or rewrite anything.

It binds **loopback only** (`127.0.0.1`). The API is unauthenticated, so anyone who can
reach it can read the paths of workspace files containing broken references, and can make
the process rescan your workspace on demand. That is fine on loopback and is not a decision
anchor should make for you on a shared network. To expose it anyway:

```sh
anchor serve --host 0.0.0.0   # every interface; prints a warning
```

Before v2.0.0 the server bound `0.0.0.0` by default. If you relied on that, pass `--host`
explicitly.

---

## Full command reference

| Command | Purpose |
|---|---|
| `anchor mv <src> <dst>` | Move safely and update references |
| `anchor check` | Find broken Markdown references |
| `anchor refs <file>` | Find what references a file |
| `anchor init [--path .]` | Create local workspace configuration |
| `anchor root` | Show the workspace root |
| `anchor mode` | Show the workspace mode |
| `anchor plan new` | Create a multi-operation plan |
| `anchor diff <plan>` | Preview a plan |
| `anchor apply <plan>` | Apply a plan |
| `anchor frontmatter …` | Frontmatter maintenance |
| `anchor serve` | Local HTTP API |
| `anchor recover` | Recover an interrupted operation |

`anchor mv`, `anchor check` and `anchor refs` are the names to learn. The longer forms
`anchor file mv`, `anchor file validate`, `anchor file refs` and `anchor validate` do the
same thing and keep working — every flag is identical.

Machine consumers: `anchor mv --format json` emits a stable JSON result. Full flags and
schemas are in [docs/COMMAND-REFERENCE.md](docs/COMMAND-REFERENCE.md).

---

## Support

anchor is released because it is useful on its own, not sold. There is no SLA — see
[SUPPORT.md](https://github.com/accelmars/.github/blob/main/SUPPORT.md). Bugs and
scope-fitting PRs are welcome.

---

## Go deeper

| Question | Doc |
|---|---|
| How do multi-file plans work? | [docs/PLAN-WORKFLOW.md](docs/PLAN-WORKFLOW.md) |
| Exactly what gets rewritten? | [docs/REWRITES.md](docs/REWRITES.md) |
| How do I inspect broken references? | [docs/REFERENCE-HEALTH.md](docs/REFERENCE-HEALTH.md) |
| What are anchor's limitations? | [docs/LIMITATIONS.md](docs/LIMITATIONS.md) |
| How do ignores work? | [docs/IGNORE.md](docs/IGNORE.md) |
| What does a full session look like? | [docs/TYPICAL-SESSION.md](docs/TYPICAL-SESSION.md) |
| Every command and flag | [docs/COMMAND-REFERENCE.md](docs/COMMAND-REFERENCE.md) |
| What do exit codes mean? | [docs/EXIT-CODES.md](docs/EXIT-CODES.md) |

---

## License

Apache 2.0 — see [LICENSE](LICENSE).

---

> **`mv` for Markdown workspaces, without the broken links.**
