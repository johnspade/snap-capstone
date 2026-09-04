# Rust implementation roadmap

## Dependency graph

```
Phase 1 ─ Foundation (parallel)
┌─────────────────┬─────────────────────────────────┬──────────────────────┐
│ 01 Version      │ 02 Text/diff/edit scripts       │ 03 Filesystem        │
│                 │                                  │                      │
└────────┬────────┴──────────┬──────────┬────────────┴──────────┬───────────┘
         │                   │          │                       │
Phase 2 ─ Core engine        │          │                       │
         │                   │          │                       │
         ▼                   ▼          ▼                       │
┌─────────────────────────────┐  ┌─────────────┐               │
│ 04 Repository format        │  │ 05 OT       │               │
│    (needs 01, 02)           │  │ (needs 02)  │               │
└──────────┬──────────────────┘  └──────┬──────┘               │
           │                            │                       │
           ▼                            ▼                       │
    ┌──────────────────────────────────────┐                    │
    │ 06 Replay engine (needs 04, 05)      │                    │
    └──────────────────┬───────────────────┘                    │
                       │                                        │
Phase 3 ─ Commands     │                                        │
                       │         ┌──────────────────────────────┘
                       │         │
                       ▼         ▼
              ┌──────────────────────────────────────┐
              │ 07 CLI + init + config (needs 04, 03)│
              └──────────────────┬───────────────────┘
                                 │
                       ┌─────────┘
                       ▼
              ┌─────────────────────────────────────┐
              │ 08 commit + status + log (needs 07, 06) │
              └──────┬──────────┬───────────────────┘
                     │          │
         ┌───────────┤          ├────────────┐
         ▼           ▼          ▼            ▼
   ┌──────────┐ ┌──────────┐ ┌──────────────────────────┐
   │ 09 diff  │ │ 10 revert│ │ 11 Merge: text + flow    │
   │(needs 08)│ │(needs 08)│ │    (needs 06, 08)        │
   └─────┬────┘ └──────────┘ └────────────┬─────────────┘
         │                                 │
         │                    ┌────────────▼─────────────┐
         │                    │ 12 Merge: path conflicts │
         │                    │    (needs 11)            │
         │                    └────────────┬─────────────┘
         │                                 │
         │                    ┌────────────▼─────────────┐
         │                    │ 13 Merge: namespace +    │
         │                    │    convergence (needs 12)│
         │                    └─────┬──────────┬─────────┘
         │                          │          │
Phase 4 ─ Extensions                │          │
         │                          │          │
         ▼                          ▼          ▼
   ┌─────────────────────────────────┐  ┌──────────────────┐
   │ 14 HTTP server + client         │  │ 15 Terminal       │
   │    (needs 13, 09)               │  │    (needs 13)     │
   └─────────────────┬───────────────┘  └────────┬─────────┘
                     │                           │
                     ▼                           ▼
              ┌────────────────────────────────────────┐
              │ 16 CLI grammar + boundaries            │
              │    (needs 14, 15)                      │
              └────────────────────────────────────────┘
```

## Parallel development opportunities

### Wave 1 — three independent tracks

All three foundation issues have zero dependencies. Start them simultaneously.

| Track A | Track B | Track C |
|---------|---------|---------|
| 01 Version | 02 Text/diff/edit scripts | 03 Filesystem |

### Wave 2 — two independent tracks

Once 02 completes, OT can start independently of repository format.

| Track A | Track B |
|---------|---------|
| 04 Repository (after 01+02) | 05 OT (after 02) |

### Wave 3 — two independent tracks

Replay needs both 04 and 05. CLI skeleton needs 04 and 03.
If 03 finished in wave 1 and 04 finishes before 05, the CLI track can start early.

| Track A | Track B |
|---------|---------|
| 06 Replay (after 04+05) | 07 CLI+init+config (after 04+03) |

### Wave 4 — sequential bottleneck

08 (commit+status+log) needs both 07 and 06. This is the convergence point.

| Single track |
|--------------|
| 08 commit + status + log (after 07+06) |

### Wave 5 — three independent tracks

Once 08 is done, diff, revert, and the first merge issue can all proceed in parallel.

| Track A | Track B | Track C |
|---------|---------|---------|
| 09 diff | 10 revert | 11 Merge: text |
| | | 12 Merge: conflicts |
| | | 13 Merge: namespace |

09 and 10 are done in one step each. The merge chain (11→12→13) is sequential
but independent of 09 and 10.

### Wave 6 — two independent tracks

Once 13 (final merge) completes, HTTP and terminal presentation can proceed in parallel.
HTTP also needs 09 (diff) which is available from wave 5.

| Track A | Track B |
|---------|---------|
| 14 HTTP (after 13+09) | 15 Terminal (after 13) |

### Wave 7 — final

| Single track |
|--------------|
| 16 CLI grammar + boundaries (after 14+15) |

## Critical path

The longest dependency chain determines minimum wall-clock time:

```
01 → 04 → 06 → 08 → 11 → 12 → 13 → 14 → 16
 └─ 02 ──┘  └─ 05 ┘       └─ 07 ┘         └─ 15 ┘
          └─ 03 ──────────── 07 ┘
```

**Critical path length: 10 issues** (01 → 04 → 06 → 08 → 11 → 12 → 13 → 14 → 16,
with 02 and 05 feeding in as parallel prerequisites).

**Maximum parallelism: 3 tracks** (waves 1 and 5).

## YAML test coverage

| YAML file | Issue |
|-----------|-------|
| 01-init | 07 |
| 02-init-paths | 07 |
| 03-configuration | 07 |
| 04-commit-status-log | 08 |
| 05-diff-goldens | 02 (logic), 09 (output) |
| 06-binary-and-empty | 08 |
| 07-revert | 10 |
| 08-unsupported-entries | 03 |
| 09-merge-text | 11 |
| 10-merge-conflicts | 12 |
| 11-namespace-conflicts | 13 |
| 12-http-server | 14 |
| 13-http-client | 14 |
| 14-cli-errors | 16 |
| 15-repository-validation | 04 |
| 16-dot-collision | 12 |
| 17-concurrent-creates | 12 |
| 18-three-way-convergence | 13 |
| 19-version-boundaries | 01 |
| 20-dirty-merge | 11 |
| 21-version-algebra | 01 |
| 22-ot-matrix | 05 (unit), 11 (E2E) |
| 23-strict-validation-matrix | 04 |
| 24-cli-grammar-matrix | 16 |
| 25-config-version-path-boundaries | 16 |
| 26-portability-and-failure-safety | 16 |
| 27-history-canonicality | 04 |
| 28-terminal-presentation | 15 |
