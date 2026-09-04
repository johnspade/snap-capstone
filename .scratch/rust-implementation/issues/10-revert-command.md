Status: ready-for-agent

# 10 — revert command

## What to build

`snap revert <version>` (§7.7).

Requires contributor configuration, a clean working tree, and a locally known target version. Diffs the current tree to the target tree and authors one new patch with message `revert to <version>` (the canonical version string). Installs the target contents, updates the repository, and prints the new version.

Revert is additive — it never removes patches or moves the frontier backward. It creates a new patch that makes the tree match the target.

If current and target trees are equal, fail with `snap: target tree is already current`.

Revert messages may exceed the 4096-byte commit limit because they contain a complete version string.

## Acceptance criteria

- [ ] Requires contributor config and clean working tree
- [ ] Target version must be locally known (materializable)
- [ ] Diffs current tree to target tree, creates one patch
- [ ] Patch message is exactly `revert to <version>`
- [ ] Installs target tree contents on disk
- [ ] Prints the new version (not the target version)
- [ ] Revert is additive — no patches removed, frontier moves forward
- [ ] Fails with exact message when current and target trees are equal
- [ ] Handles file-to-directory and directory-to-file transitions during revert
- [ ] Integration tests for revert scenarios
- [ ] YAML acceptance: `07-revert`

## Blocked by

- `08-commit-status-log` — needs commits to create history to revert to
