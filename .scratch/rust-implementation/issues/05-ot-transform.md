Status: ready-for-agent

# 05 — OT transform

## What to build

The text edit Operational Transformation function from §6.3. Given an incoming edit `P` and an aggregate context edit `Q`, produce a transformed `P'` that applies after `Q`.

Process both streams left to right using the 6-row priority table:

| Next operations       | Output in transformed P     | Consumption |
|-----------------------|-----------------------------|-------------|
| Q insert              | retain(length(Q insert))    | Q only      |
| P insert              | same P insert               | P only      |
| P retain, Q retain    | retain(min)                 | both        |
| P delete, Q retain    | delete(min)                 | both        |
| P retain, Q delete    | nothing                     | both        |
| P delete, Q delete    | nothing                     | both        |

`Q insert` row has priority (concurrent inserts appear in canonical integration order). Split counts as needed when operations don't align. Both scripts must consume the same base token count. Continue until both streams end, processing trailing insertions. Coalesce adjacent output operations.

Implement as a `VecDeque`-based loop with pattern matching on both stream fronts — a direct translation of the functional recursive style (pattern-match heads, split counts, push remainders back) as decided in the ADR.

## Acceptance criteria

- [ ] Transform function: `(incoming_edit, context_edit) -> transformed_edit`
- [ ] All 6 rows of the transform table implemented
- [ ] Count splitting when operations don't align
- [ ] Q-insert priority (concurrent inserts in integration order)
- [ ] Trailing insertion handling
- [ ] Output coalescing (no adjacent same-kind operations)
- [ ] Both streams fully consumed (error if not)
- [ ] Unit tests for every row combination in isolation
- [ ] Unit tests for split counts (e.g., retain(5) against retain(3) splits to retain(3) + retain(2))
- [ ] Unit tests for concurrent inserts at the same cursor position
- [ ] Unit tests for trailing inserts on both sides
- [ ] Unit tests for empty-to-nonempty and nonempty-to-empty transforms
- [ ] YAML: `22-ot-matrix` exercises this through merge (available in issue 11)

## Blocked by

- `02-text-tokenization-diff-and-edit-scripts` — OT operates on the same edit script types
