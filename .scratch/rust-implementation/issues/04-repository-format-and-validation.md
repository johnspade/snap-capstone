Status: ready-for-agent

# 04 — Repository format and validation

## What to build

Parsing, construction, and full validation of `repository.json` (§4.1–§4.5).

**JSON parsing:** Read `repository.json` as UTF-8 JSON. Reject duplicate object keys, unknown fields, non-integer numbers, and invalid typed values. The parsed typed value is authoritative, not the serialized bytes.

**Schema:** `format: 1`, `frontier` as a version, `patches` as an array of patch objects. Each patch has `author`, `revision`, `base` (a version), `message`, and `changes`. Changes are `text`, `put`, or `delete` variants (§4.3). `put` content is standard padded RFC 4648 base64.

**Patch identity (§4.2):** Dot is `(author, revision)`. `revision = base[author] + 1`. All other result components equal the base. Same dot with different parsed values is corruption.

**Validation (§4.5) — all 6 steps:**
1. Exact schema, all versions, IDs, paths, messages, changes
2. Patch sorting (by author then numeric revision), one value per dot, contiguous contributor revisions
3. Every patch's complete base closure and `revision = base[author] + 1`
4. Acyclic causality (no ready patch remaining = cycle or missing dep)
5. Every change against its materialized exact base
6. Deterministic replay of the declared frontier

**Messages (§4.2):** Nonempty UTF-8, may contain tab and LF but no other ASCII control characters. `commit` limits to 4096 bytes; generated revert messages may be longer.

**Writing:** Two-space indentation, trailing LF. `patches` sorted by author then numeric revision. Only the causal closure of `frontier`, no unreachable patches.

Use `serde` + `serde_json` with duplicate-key validation. Per-module error types (e.g. `repository::ValidationError`).

## Acceptance criteria

- [ ] Parse `repository.json` with duplicate-key rejection
- [ ] All change variant types: text (with edit script), put (with base64), delete
- [ ] Patch identity: dot derivation, result version computation
- [ ] All 6 validation steps from §4.5
- [ ] Serial-contributor rule enforcement (§3.5)
- [ ] Message validation (nonempty, no forbidden control chars, length limit for commit)
- [ ] Repository writing with canonical formatting (2-space indent, trailing LF, sorted patches)
- [ ] Unit tests for every validation rejection case
- [ ] YAML acceptance: `15-repository-validation`, `23-strict-validation-matrix`, `27-history-canonicality`

## Blocked by

- `01-version-type-and-algebra` — versions are used everywhere in repository format
- `02-text-tokenization-diff-and-edit-scripts` — changes contain edit scripts and text content
