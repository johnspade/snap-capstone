# Snap

A local version control system with vector-clock versions, patch-based history, and deterministic automatic merges.

## Language

**Version**:
A vector clock — a map from contributor ID to the latest revision by that contributor. Describes a causal frontier, not a branch or a single commit.
_Avoid_: commit hash, ref, HEAD

**Patch**:
An immutable, causally anchored change to the file tree. Each patch names its exact base version, carries file changes, and increments one contributor's revision by one.
_Avoid_: commit, changeset, delta

**Frontier**:
The repository's current version — the join of all integrated patches' result versions.
_Avoid_: HEAD, tip, latest

**Contributor**:
The author identity (an email-shaped ASCII string) used as a vector-clock component. One contributor must not author concurrently in disconnected copies.
_Avoid_: user, author, committer

**Dot**:
A `(contributor, revision)` pair that uniquely identifies one patch.
_Avoid_: patch ID, commit ID

**Replay**:
Deterministic reconstruction of a file tree from the empty tree by integrating patches in causal-then-Snap order. Same patch set and frontier always produce the same bytes.
_Avoid_: rebuild, rebase, apply

**Snap order**:
An arbitrary total order on versions used to sequence concurrent patches during replay. Lexicographic comparison of counters over the sorted union of contributor IDs. Extends causal order but has no chronological meaning.
_Avoid_: canonical order, sort order

**Integration**:
The process of applying one patch during replay, resolving conflicts against the canonical tree built so far using OT and path-level rules.
_Avoid_: merge (which is a user-facing command), apply

**Operational Transformation (OT)**:
Line-level transformation of a text edit so it applies cleanly after concurrent changes to the same file. Used during integration, not exposed to users.
_Avoid_: merge algorithm, conflict resolution

**Materialization**:
Writing an in-memory tree to the filesystem. Happens once after replay completes — replay itself is a pure computation.
_Avoid_: checkout, restore
