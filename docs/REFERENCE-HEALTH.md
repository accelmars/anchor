# Reference Health

Two commands for keeping workspace references clean.

---

## Check for broken refs

```sh
anchor file validate
```

Scans all `.md` files in the workspace. Reports any `[text](path)` links where the
target doesn't exist, with file and line number.

```
BROKEN REFERENCES (2):
  my-file.md:14  → old-folder/guide.md  (not found)
    similar: new-folder/guide.md
2 broken references in 2 files.
```

The `similar:` hint is a best-guess fuzzy match — sometimes accurate, sometimes not.

Exit codes: 0 = clean, 1 = broken refs found, 2 = system error.

`anchor validate` is an alias for `anchor file validate`.

---

## Find what references a file

```sh
anchor file refs path/to/file.md
```

Lists every file in the workspace that links to the given path. Run this before
moving a frequently-referenced file to understand the blast radius.

```
References to: path/to/file.md

  docs/guide.md:47
  people/alice/MIND.md:8

2 files reference this file.
```

Exit 0 always — zero results is not an error.

---

## Acknowledging known broken refs

Place patterns in `.accelmars/anchor/acked` (gitignore syntax) to suppress known
broken refs from `anchor file validate` output. Acknowledged refs are still counted
in the `acknowledged` field of `--format json` output but do not affect the exit
code.
