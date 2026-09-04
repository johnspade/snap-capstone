Status: ready-for-agent

# 11 — Merge: text merge and basic flow

## What to build

The `snap merge <repository>` command (§7.8) — the basic flow and text OT merge. This is the first of three merge issues; it gets merge working end-to-end for the common case of concurrent text edits on the same files.

**Merge command shell:**
- Requires a clean working tree (no contributor config needed)
- Load and validate the other repository (local path)
- Union the patch sets and join the frontiers
- Canonically replay the joined history via the replay engine (issue 06)
- Install the result tree on disk and update `repository.json`
- Create no patch, increment no revision
- Print new warnings to stderr, joined version to stdout

**Text OT merge:** When both sides edit the same text file, derive the aggregate context edit `Q = diff(B, C)` and transform the incoming edit through `Q` via issue 05's OT. The result applies to `C`.

**Identical concurrent changes:** When `C` equals `T` (both sides made the same change), keep it unchanged with no warning (§6.2 rule 2).

**Idempotent merge:** Merging equal or already-contained history succeeds, changes nothing, emits no warnings, and prints the unchanged version.

**Dirty-tree refusal:** Merge refuses if the working tree is dirty or contains unsupported entries. Validation failures cause no mutation (§10).

**Validation-before-mutation (§10):** Complete all parsing, validation, replay, dirty checks, and target tree construction before writing anything.

## Acceptance criteria

- [ ] Merge command: load remote repo, union patches, join frontiers
- [ ] Full replay of joined history
- [ ] Text OT merge for concurrent text edits on the same file
- [ ] Identical concurrent changes produce no warning
- [ ] Idempotent re-merge (same or contained history = no-op)
- [ ] Dirty-tree refusal before import
- [ ] Validation-before-mutation: no writes on failure
- [ ] New warnings printed to stderr, version to stdout
- [ ] Integration tests for text merge scenarios
- [ ] YAML acceptance: `09-merge-text`, `20-dirty-merge`

## Blocked by

- `06-replay-engine` — merge invokes replay for the joined history
- `08-commit-status-log` — needs commits in two repos to merge
