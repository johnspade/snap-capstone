Status: ready-for-agent

# 15 — Terminal presentation

## What to build

The terminal output mode (§7.11): ANSI SGR color and symbols for all command output, controlled by `SNAP_COLOR` and `NO_COLOR` environment variables.

**Mode selection:**
- `SNAP_COLOR` unset or `auto`: terminal mode on each stream independently when that stream is a TTY, unless `NO_COLOR` is present (any value including empty)
- `SNAP_COLOR=always`: terminal mode on both streams, overrides `NO_COLOR`
- `SNAP_COLOR=never`: plain mode on both streams
- Any other value: error before command execution (`snap: SNAP_COLOR must be auto, always, or never`)

**ANSI wrapping:** `S(n, text)` = `ESC[<n>m<text>ESC[0m`. Codes: bold `1`, dim `2`, red `31`, green `32`, yellow `33`, magenta `35`, cyan `36`.

**Per-command terminal layouts (§7.11):**
- `init`/`commit`/`revert`/`merge`: `S(32,"✓") + " " + S(1,label) + " " + S(36,version)`
- `status`: header with version, clean-tree indicator or colored change rows
- `log`: colored bullet, bold message, version + author metadata
- `diff`: colored line prefixes (`---`/`+++` bold, `@@` cyan, `-` red, `+` green, `\` dim, `Binary` yellow)
- `--version`: bold
- Warnings: `S(33,"⚠") + " " + S(33,detail)`
- Errors: `S(31,"✗ " + detail)`
- `config`: silent; `--serve` URL: always plain

Extend the `Writer` struct from issue 07 with terminal mode. The `Writer` already channels all output — add a `style(code, text)` method that wraps or passes through based on the stream's mode.

**TTY unit tests:** Each implementation must unit-test `auto` mode selection for TTY and non-TTY stdout/stderr independently (the YAML harness uses pipes, so it can't test TTY detection).

## Acceptance criteria

- [ ] `SNAP_COLOR` and `NO_COLOR` precedence logic per §7.11
- [ ] Invalid `SNAP_COLOR` value rejected with exact error message
- [ ] ANSI SGR wrapping in `Writer.style()`
- [ ] Terminal output for every command family matches §7.11 exactly
- [ ] `--serve` URL always plain regardless of mode
- [ ] `config` always silent regardless of mode
- [ ] Unit tests for mode selection logic (all `SNAP_COLOR`/`NO_COLOR` combinations)
- [ ] Unit tests for TTY vs non-TTY detection in `auto` mode
- [ ] Exact byte-level output verification
- [ ] YAML acceptance: `28-terminal-presentation`

## Blocked by

- `13-merge-namespace-conflicts-and-convergence` — terminal mode tests cover merge warnings and all prior commands
