Status: ready-for-agent

# 01 — Version type and algebra

## What to build

The `Version` type and all operations the rest of the system builds on: parsing from canonical CLI syntax (§3.2), display back to canonical form, JSON serialization as `[id, revision]` pairs (§3.2), contributor ID validation (§3.1), four-way causal comparison (§3.3), join (§3.3), and Snap order (§3.4).

The type should enforce parse-don't-validate: a constructed `Version` is always canonical. An absent component is zero and is omitted. Contributor IDs are validated at construction — email-shaped ASCII, exactly one `@`, no control chars, no whitespace, no `,()` or `->` substring, max 254 bytes. Revisions are positive integers up to `9007199254740991`.

Use per-module error types with `thiserror` (e.g. `version::ParseError`) as decided in the ADR.

## Acceptance criteria

- [ ] `Version` type with parse-don't-validate construction
- [ ] Contributor ID validation per §3.1 (email shape, forbidden characters, length limit)
- [ ] Canonical string parsing and display per §3.2 — round-trips exactly
- [ ] JSON serialization/deserialization as ordered `[id, revision]` pairs
- [ ] Four-way comparison: equal, before, after, concurrent (§3.3)
- [ ] `join(V, W)` returns componentwise max (§3.3)
- [ ] Snap order: total order extending causal order (§3.4)
- [ ] Rejects: duplicate IDs, explicit zeroes, leading zeroes, overflow, invalid IDs, whitespace, noncanonical ordering
- [ ] Unit tests for every comparison outcome and parse rejection
- [ ] Property tests: join commutativity, join associativity, join idempotency, Snap order extends causal order, parse/display round-trip
- [ ] YAML acceptance: `21-version-algebra`, `19-version-boundaries` (these test through CLI commands available in later issues — version logic must be correct so those pass when commands are wired up)

## Blocked by

None — can start immediately.
