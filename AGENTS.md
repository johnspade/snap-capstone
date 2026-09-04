# Snap — agent guidance

## Sources of truth

[`SPEC.md`](SPEC.md) is the canonical product contract. Public behavior must be
demonstrated in the language-neutral YAML suite under [`tests/`](tests/).
You may add language-specific unit tests while developing, but they cannot
replace the shared acceptance suite.

When implementation work reveals an ambiguity or contradiction, correct the
spec first or in the same commit and add a regression case to the public YAML
suite. Do not silently make the implementation authoritative.

## Implementation layout

Work in the language directory present at the project root. Keep responsibilities
separate: versions, text/diff and OT, repository validation and replay,
filesystem materialization, working-tree changes, HTTP, commands, and CLI
dispatch.

The YAML harness is implementation-language neutral. Never import reference
code into it or add shell setup operations to test around a missing typed
operation. Extend its tagged unions additively so existing format-1 cases keep
their meaning.

## Verification

After implementation changes, run the shared acceptance suite from the
devShell:

```bash
acceptance
```

Or directly via the test harness:

```bash
cd test-harness
npm run run -- --candidate ../result/bin/snap
```

After harness changes, also run:

```bash
cd test-harness
npm run check
npm test
```

## Pull requests

When creating a PR, follow the template in
`.github/pull_request_template.md` (What / Why / Testing sections).

## Scope discipline

Snap’s small surface is deliberate. Do not add branches, staging, checkout,
push, authentication, object storage, or unresolved-conflict machinery. Spend
complexity on deterministic behavior, strict validation, and exact tests—not
on production scalability or command count.

## Agent skills

### Issue tracker

Local markdown under `.scratch/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Default vocabulary (needs-triage, needs-info, ready-for-agent, ready-for-human, wontfix). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout — one `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.

