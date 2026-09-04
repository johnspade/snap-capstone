Status: ready-for-agent

# 12 — Merge: path-level conflict rules and dot collision

## What to build

All §6.4 path-level conflict resolution rules and cross-repository dot collision detection. Builds on the merge flow from issue 11.

**Path-level rules (§6.4):** For each incoming change where B, C, T are not resolved by the basic rules (B=C, C=T, or text OT), apply in priority order:

1. C and T identical → keep C, no warning
2. T absent → incoming delete wins (`delete-wins`)
3. B present, C absent → earlier concurrent delete wins (`delete-wins`)
4. B absent, C and T present → incoming (later) create wins (`later-create-wins`)
5. Incoming change is `put` → incoming atomic replacement wins (`later-put-wins`)
6. P is text, C is non-text → incompatible current content wins (`put-wins`)

Each discarding rule emits a `(path, reason)` warning pair. Merge prints only new warnings (present in joined replay but absent from pre-merge local replay).

**Concurrent creates (§6.4 rule 4):** When both sides create the same path independently, the canonically later create wins regardless of merge direction.

**Dot collision detection (§3.5):** When importing patches, if the same `(contributor, revision)` dot exists in both repositories with structurally different parsed values, the repository is corrupt. Merge fails before writing.

## Acceptance criteria

- [ ] All 6 path-level rules from §6.4 implemented
- [ ] Correct warning emission for each discarding rule
- [ ] Warning deduplication: only new warnings printed (not present in pre-merge replay)
- [ ] Warning sorting: by path, then reason
- [ ] Concurrent creates: canonically later wins regardless of merge direction
- [ ] Dot collision detection: same dot, different values → corruption error before mutation
- [ ] Integration tests for each conflict rule in isolation
- [ ] Integration tests for concurrent creates in both merge directions
- [ ] YAML acceptance: `10-merge-conflicts`, `16-dot-collision`, `17-concurrent-creates`

## Blocked by

- `11-merge-text-and-basic-flow` — extends the merge command with conflict rules
