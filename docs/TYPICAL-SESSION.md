# Typical Session

A complete anchor workflow from workspace check to verified clean state.

```sh
# 1. Confirm workspace is initialized
anchor root

# 2. See what references a folder before moving it
anchor file refs foundations/gateway-engine/

# 3. Write the plan (by hand for large batches)
cat > restructure.toml << 'EOF'
version = "1"
description = "Group foundations under foundations/"

[[ops]]
type = "create_dir"
path = "foundations"

[[ops]]
type = "move"
src = "gateway-engine"
dst = "foundations/gateway-engine"
EOF

# 4. Validate
anchor plan validate restructure.toml

# 5. Preview — add --verbose to see each ref that will change
anchor diff --verbose restructure.toml

# 6. Apply
anchor apply restructure.toml
# anchor warns on stderr if any non-.md files have unhandled occurrences

# 7. Verify — must exit 0
anchor validate

# 8. Manually check plain-text refs (prose, table cells) in navigation docs
grep -r "gateway-engine" CONSTELLATION.md CLAUDE.md
```

---

## Notes

- Step 2 (`anchor file refs`) is optional but recommended for high-traffic folders.
- Step 5 (`anchor diff --verbose`) is optional for small plans; required before
  applying anything touching >50 files.
- Step 8 is manual — anchor does not rewrite bare prose paths. Check navigation
  documents after every structural move.
