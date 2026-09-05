Status: done

# 13 — Merge: namespace conflicts and convergence

## What to build

Namespace conflict resolution (§6.2) and proof of three-way convergence. This is the final merge issue.

**Namespace conflicts (§6.2):** Before per-path evaluation, resolve prefix-free violations. Let `S` be the paths that `P` makes present, and `C'` be `C` with every path that `P` authored as a deletion removed. If a path in `S` has a different current ancestor or descendant in `C'`:
- Mark the incoming path for installation as its authored result `T`
- Mark every conflicting current path for removal
- Each removed path emits `namespace-wins`

These decisions override per-path rules. The authored result is prefix-free, so two paths in `S` cannot conflict. Duplicate removals and warnings collapse. Form the target tree by removing marked current paths and installing marked authored results simultaneously with all other resolved changes.

**Three-way convergence:** When three contributors work concurrently and merge in different association orders (e.g., `(A merge B) merge C` vs `A merge (B merge C)` vs any other pairing), the final tree and warning set must be identical.

**Property tests:** Generate valid causal patch graphs and verify that import permutations produce the same joined frontier, patch set, warnings, and tree. This validates the §6.5 guarantee.

## Acceptance criteria

- [x] Namespace conflict detection (file vs ancestor/descendant)
- [x] Incoming path wins with `namespace-wins` warning
- [x] Conflicting current paths removed
- [x] Namespace resolution overrides per-path rules
- [x] Duplicate removal and warning collapsing
- [x] Three-way convergence: same tree regardless of merge association order
- [x] Property tests: random valid patch graphs, permuted imports → same result
- [x] Integration tests for file-replaces-directory and directory-replaces-file
- [x] YAML acceptance: `11-namespace-conflicts`, `18-three-way-convergence`

## Blocked by

- `12-merge-path-level-conflicts-and-dot-collision` — builds on the full conflict resolution pipeline
