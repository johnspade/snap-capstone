Status: ready-for-agent

# 08 — commit, status, and log

## What to build

The single-contributor workflow: `commit`, `status`, and `log`. After this issue, a user can initialize a repo, commit changes, and inspect history.

**`snap commit <message>` (§7.5):** Requires contributor config and a dirty working tree. Reject messages > 4096 UTF-8 bytes. Diff the current tree against the working tree to produce changes. Create one patch based on the current frontier, incrementing the contributor's revision. Use text changes when new content is text and old path is absent or text; otherwise `put`; deletions use `delete`. Atomically replace `repository.json`. Print the new version. Reject clean tree, invalid message, overflow, or dot collision.

**`snap status` (§7.3):** Print `version <version>` then working changes sorted by path. Codes: `A` (absent→present), `M` (changed bytes), `D` (present→absent). Clean repo prints only the version line.

**`snap log` (§7.4):** Print patches in reverse canonical integration order, one tab-separated line each: `<result-version>\t<author>\t<message>`. In messages, escape `\` → `\\`, tab → `\t`, LF → `\n` (in that order).

**Binary and empty files (§4.3, §7.5):** Binary files (non-text) use `put` with base64 content. Empty files use an empty text edit script. The system must round-trip arbitrary bytes exactly.

## Acceptance criteria

- [ ] `snap commit` — creates patch, updates frontier, prints new version
- [ ] Working-tree diff: detects additions, modifications, deletions
- [ ] Change type selection: text vs put vs delete per §7.5 rules
- [ ] Message validation (nonempty, no forbidden control chars, <= 4096 bytes)
- [ ] Dot collision detection on commit
- [ ] `snap status` — version line + sorted change codes (A/M/D)
- [ ] `snap log` — reverse integration order, tab-separated, escaped messages
- [ ] Binary file round-trip via base64 `put` changes
- [ ] Empty file handling via empty text edit script
- [ ] Clean-tree rejection for commit
- [ ] Missing contributor config error: exact message per §8
- [ ] Integration tests for multi-commit workflows
- [ ] YAML acceptance: `04-commit-status-log`, `06-binary-and-empty`

## Blocked by

- `07-cli-skeleton-init-config-version` — needs CLI dispatch, init, and config
- `06-replay-engine` — commit builds on the current materialized tree from replay
