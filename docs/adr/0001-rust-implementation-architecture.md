# Rust implementation architecture

The Rust implementation uses a `lib.rs` with focused modules and a thin `main.rs` for CLI dispatch. We chose synchronous, minimal-dependency crates throughout and direct translations of the spec's algorithms over optimized alternatives, prioritizing correctness and auditability over performance.

## Considered options

### Module structure

A single binary crate was simpler but makes unit testing harder. A `lib.rs` + `main.rs` split lets tests import types directly and keeps the CLI boundary thin.

### CLI parsing

Hand-rolled parsing would avoid the `clap` dependency for a small grammar (8 commands, few flags), but `clap` reduces edge-case bugs in option positioning and error formatting. The CLI grammar is fully spec'd, so `clap`'s declarative style maps cleanly.

### Error handling

A single `SnapError` enum (~20 variants) was considered but conflates unrelated failure modes. Per-module error enums with `thiserror` (`version::ParseError`, `repository::ValidationError`, `filesystem::FsError`, etc.) composed via `#[from]` at a top-level `CommandError` mirror the effectful-Scala pattern of typed error channels per operation. `anyhow` was rejected because the spec requires exact error messages tested in the YAML suite.

### HTTP server

`hyper`/`axum` pull in `tokio` for one blocking endpoint. `tiny_http` is synchronous and minimal — the server has one route and no concurrency requirement. Paired with `signal_hook` for SIGINT/SIGTERM handling.

### HTTP client

`reqwest` brings `tokio` and TLS machinery. `ureq` is synchronous, supports both `http://` and `https://`, and matches the synchronous style of the rest of the binary. No async runtime anywhere.

### Diff algorithm

Myers is `O(n*d)` but requires proving output equivalence with the spec's exact recurrence and delete-on-tie rule. Direct DP from §5 is ~50 lines, trivially correct against the golden tests, and the line-level inputs are small. Optimize later if needed, validated by the same goldens.

### OT transform

Implemented as a `VecDeque`-based loop with pattern matching on both stream fronts — a direct Rust translation of the functional recursive style (pattern-match heads, split counts, push remainders back). This maps 1:1 to the spec's 6-row table.

### Replay

Trees are materialized in memory (`HashMap<String, FileContent>`) during replay. Intermediate versions are cached in a `HashMap<Version, Tree>` so each patch's base tree is a map lookup, not a re-replay. Disk writes happen once at the end — replay is a pure computation.

### Presentation

All output flows through a `Writer` struct that knows the per-stream mode (plain/terminal). Commands never touch stdout/stderr directly. A `style(code, text)` method wraps or passes through. No intermediate output representation — the spec's terminal mode is mechanical decoration, not restructuring.

## Consequences

- No async runtime in the dependency tree. Every IO operation is blocking.
- The direct DP diff is `O(n*m)` — acceptable for line-tokenized files in a local VCS, but not suitable if files grow to tens of thousands of lines.
- Caching all intermediate trees during replay is `O(patches * tree_size)` memory. Fine for the small histories the spec targets; would need structural sharing for large repos.
- `parse-don't-validate` at the value level (versions, changes, file contents) but repository-level invariants (causal closure, sorting) are checked at construction time, not encoded in types.
