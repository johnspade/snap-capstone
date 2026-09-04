Status: ready-for-agent

# 03 — Filesystem operations

## What to build

Working tree scanning, path validation, symlink/special-file rejection, dirty detection, and tree materialization.

**Scanning:** Enumerate all regular files below the repository root except `.snap/` and its contents. Report symlinks and other non-regular entries as errors — never follow or silently ignore them (§2, §10).

**Path validation (§2):** Tracked paths are UTF-8 relative paths using `/` separators. Must be nonempty, no ASCII control characters or backslashes, no empty/`.`/`..` segments, first segment not `.snap`. No Unicode or case normalization. Sort by unsigned lexicographic UTF-8 bytes.

**Prefix-free enforcement (§2):** If `a` is a file, no `a/...` path may be present. Validate this for every tracked tree.

**Dirty detection:** Compare the working tree's path/byte map against the current tree. Clean when they match exactly and no unsupported entries exist.

**Materialization:** Write an in-memory tree (`HashMap<String, Vec<u8>>`) to disk. Remove files that block required directories, create required directories, write target files, remove newly empty directories. The filesystem must represent exactly the target path/byte map.

Use per-module error types (e.g. `filesystem::FsError`).

## Acceptance criteria

- [ ] Working tree scanning: recursive file enumeration, `.snap/` exclusion
- [ ] Path validation per §2 (all rejection cases)
- [ ] Symlink and special-file detection and rejection
- [ ] Dirty detection comparing working tree against a reference tree
- [ ] Tree materialization: write files, manage directories, remove conflicts
- [ ] Prefix-free validation
- [ ] Unit tests with temp directories: scan, materialize, round-trip, symlink rejection, path validation edge cases
- [ ] YAML acceptance: `08-unsupported-entries`

## Blocked by

None — can start immediately.
