Status: ready-for-agent

# 16 — CLI grammar and boundary validation

## What to build

Final polish: exact error messages, exit codes, argument rejection, and boundary edge cases across all commands.

**CLI grammar (§7):** Options occur exactly in the positions shown in the spec. Unknown options, extra operands, and missing option values are errors. Each command rejects: unknown flags, misplaced flags, duplicate flags, extra positional arguments. Every rejection produces a `snap: <detail>` error on stderr with exit code 1.

**Exit codes (§10):** 0 for success, 1 for expected errors, 2 for unexpected internal failures.

**Configuration boundaries (§8):** Malformed config files (bad JSON, non-unique keys, unknown fields, invalid ID) are errors. Missing `$HOME` makes global config unavailable. The exact "contributor.id is required" message when ID is missing.

**Version boundaries (§3.1–§3.2):** Maximum revision (`9007199254740991`), overflow rejection, all canonical syntax edge cases when passed as CLI arguments.

**Path boundaries (§2):** All path validation edge cases when encountered in the working tree or repository.

**Portability (§10):** Text byte preservation through local repository exchange. Malformed remote repositories never mutate local state.

## Acceptance criteria

- [ ] Every command rejects unknown, misplaced, duplicate, and extra arguments
- [ ] Exact error messages match spec format
- [ ] Exit codes: 0/1/2 per §10
- [ ] Config boundary cases: malformed files, missing HOME, exact error messages
- [ ] Version boundary cases: max revision, overflow, all parse rejections as CLI args
- [ ] Path boundary cases: all §2 rejections in working tree and repository context
- [ ] Text byte preservation through round-trip (commit in one repo, merge to another, verify bytes)
- [ ] Malformed remote never mutates local state
- [ ] E2E tests covering every error path
- [ ] YAML acceptance: `14-cli-errors`, `24-cli-grammar-matrix`, `25-config-version-path-boundaries`, `26-portability-and-failure-safety`

## Blocked by

- `14-http-server-and-client` — tests cover remote failure cases
- `15-terminal-presentation` — tests cover error presentation in both modes
