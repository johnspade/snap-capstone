Status: ready-for-agent

# 02 — Text tokenization, canonical diff, and edit scripts

## What to build

Text handling: detection, tokenization, the canonical diff algorithm (§5), and edit script types with application (§4.4).

A file is text when its bytes are valid UTF-8 and contain no NUL. Tokenize by splitting immediately after every LF byte, retaining LF in the token. The empty file has zero tokens.

The canonical diff is the DP recurrence from §5 — not Myers, not Hirschberg. Given old tokens `A[0..n]` and new tokens `B[0..m]`, compute `D(i,j)` as minimum edits to transform `A[i..]` into `B[j..]`. Walk from `(0,0)`: equal tokens produce retain, otherwise delete when `D(i+1,j) <= D(i,j+1)`, otherwise insert. Coalesce adjacent same-kind operations.

Edit scripts are arrays of `retain(n)`, `delete(n)`, `insert([tokens])`. Validate: counts positive, no adjacent same-kind, must consume all old tokens, result tokens must be canonical (each except possibly the last ends in LF). An empty script is valid only for creating an empty text file.

Implement script application (old tokens + script = new tokens) and script construction (old tokens + new tokens = script via canonical diff).

## Acceptance criteria

- [ ] Text detection: UTF-8 validity check, NUL rejection
- [ ] Line tokenization: split after LF, retain LF in token, empty file = no tokens
- [ ] Canonical DP diff producing edit scripts per §5's exact recurrence and delete-on-tie rule
- [ ] Edit script types: `Retain(n)`, `Delete(n)`, `Insert(Vec<String>)`
- [ ] Script application: old tokens + script = new tokens
- [ ] Script validation: positive counts, no adjacent same-kind, complete consumption, canonical result tokens
- [ ] Unit tests for: empty file, single line, no-final-LF, CRLF tokens, repeated equal lines (the tie-breaking matters here), all-insert, all-delete
- [ ] Unit tests verifying delete-on-tie produces the exact same script as the spec's recurrence
- [ ] YAML: `05-diff-goldens` exercises this logic (output format tested in issue 09)

## Blocked by

None — can start immediately.
