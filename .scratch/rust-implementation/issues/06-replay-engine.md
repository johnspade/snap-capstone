Status: ready-for-agent

# 06 — Replay engine

## What to build

Deterministic replay: given a set of patches and a target version, reconstruct the file tree from the empty tree by integrating patches in the correct order (§6.1–§6.4).

**Patch selection and ordering (§6.1):** Select patches where `n <= V[c]`. Start from empty tree. Repeatedly find patches whose bases are fully integrated, choose the least by: (1) Snap order of result version, (2) UTF-8 order of author, (3) numeric revision. Integrate that patch. If no ready patch remains before replay completes, the history has a cycle or missing dependency.

**Single-patch integration (§6.2):** Materialize the patch's exact base tree `B`. Let `C` be the canonical tree built so far. For each changed path, evaluate in order: (1) B=C → apply directly, (2) C=T → keep unchanged, (3) B/C/T all text and patch is text → OT via §6.3, (4) path-level rules via §6.4.

**Namespace conflict resolution (§6.2):** Before per-path evaluation, check if paths the patch makes present conflict with existing paths by ancestry (file vs directory). Incoming path wins with `namespace-wins` warning.

**Path-level rules (§6.4):** Six rules in priority order — identical (no warning), delete-wins, earlier-delete-wins, later-create-wins, later-put-wins, put-wins. Each discarding rule emits a `(path, reason)` warning pair.

**Caching:** Cache intermediate trees in a `HashMap<Version, Tree>` so each patch's base is a lookup, not a re-replay. Replay is a pure computation — no disk writes.

**Warnings:** Collect unique `(path, reason)` pairs, sorted by path then reason.

## Acceptance criteria

- [ ] Patch selection for a given target version
- [ ] Correct integration order (Snap order, author, revision)
- [ ] Single-patch integration with all 4 path resolution branches
- [ ] OT integration for concurrent text changes (using issue 05's transform)
- [ ] All 6 path-level conflict rules from §6.4
- [ ] Namespace conflict resolution with `namespace-wins` warnings
- [ ] Intermediate tree caching
- [ ] Cycle/missing-dependency detection
- [ ] Warning collection: unique pairs, sorted by path then reason
- [ ] Unit tests: replay with no conflicts, each path-level rule in isolation, namespace conflicts
- [ ] Property tests: same patches in different integration orders produce the same tree and warnings
- [ ] Replay is a pure function (no side effects)

## Blocked by

- `04-repository-format-and-validation` — replay operates on validated repository data
- `05-ot-transform` — replay invokes OT for concurrent text changes
