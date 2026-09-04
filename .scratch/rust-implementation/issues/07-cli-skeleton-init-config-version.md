Status: ready-for-agent

# 07 — CLI skeleton, init, config, and --version

## What to build

The CLI entry point and the first three commands that make `snap` a runnable binary: `init`, `config`, and `--version`.

**CLI dispatch:** Use `clap` for argument parsing. The grammar is fully spec'd (§7) — exact option positions, no unknown options, no extra operands. Repository location: walk from CWD to filesystem root looking for `.snap/`. Errors go to stderr, results to stdout. Exit codes: 0 success, 1 expected error, 2 internal failure.

**Writer struct:** All output flows through a `Writer` that knows the per-stream mode (plain/terminal). Initially implement plain mode only — terminal mode is added in issue 15. Commands never touch stdout/stderr directly.

**`snap init [path]` (§7.1):** Path defaults to `.`. Create directory if absent. Create empty `.snap/repository.json` (`format: 1`, empty frontier, empty patches). Reject reinitializing or nesting inside an existing repository. Print `()`.

**`snap config [--global] contributor.id <id>` (§7.2):** Validate the ID. Without `--global`, write `.snap/config.json`. With `--global`, write `$HOME/.snapconfig.json` (no repository needed). Preserve no unknown fields. Silent on success.

**Configuration reading (§8):** Read local `.snap/config.json` first. If it has an ID, skip global. Otherwise read `$HOME/.snapconfig.json`. Missing file = no value. Malformed file or invalid ID in a read file = error.

**`snap --version` (§7.10):** Print `snap <semver>`, no repository needed.

**Error format (§10):** Plain mode errors are `snap: <detail>` on stderr.

## Acceptance criteria

- [ ] `clap`-based CLI dispatch with exact option grammar
- [ ] Repository location by walking up from CWD
- [ ] `Writer` struct with plain mode output
- [ ] `snap init` — creates repo, rejects re-init and nesting, prints `()`
- [ ] `snap config` — validates ID, writes local or global config, silent on success
- [ ] Config reading with local-over-global precedence
- [ ] `snap --version` — prints version without requiring a repo
- [ ] Error output format: `snap: <detail>` on stderr, exit code 1
- [ ] Integration tests for init, config, and version flows
- [ ] YAML acceptance: `01-init`, `02-init-paths`, `03-configuration`

## Blocked by

- `04-repository-format-and-validation` — init writes `repository.json`, config validates IDs
- `03-filesystem-operations` — init creates directories, repo location scans the filesystem
