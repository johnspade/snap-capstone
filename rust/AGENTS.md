# Rust — agent guidance

## Development environment

The Rust toolchain is managed by a Nix flake. `cd rust/` and let direnv
activate the devShell (or run `nix develop` manually). All cargo subcommands,
linters, and validation scripts are available inside the shell — do not
install toolchains globally.

## Verification

Run `validate` inside the devShell before committing. It runs `nix flake
check --keep-going` (fmt, clippy, test, deny, audit, doc, coverage) and
Miri. CI runs the same checks.

For a full validation including mutation testing: `validate-all`.

Individual checks from the devShell:

| What | Command |
|------|---------|
| Format | `cargo fmt --check` |
| Clippy | `cargo clippy --all-targets --all-features -- -D warnings` |
| Tests | `cargo nextest run --all-features` |
| License/ban audit | `cargo deny check` |
| Advisory audit | `cargo audit` |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features` |
| Coverage | `cargo llvm-cov --workspace --all-features` |
| Miri | `nix run .#miri` |
| Mutation (diff) | `cargo-mutants-diff` |

All checks at once via Nix (same as CI):

```
nix flake check --keep-going
```

## Warnings

Fix compiler and clippy warnings properly instead of suppressing them with
`#[allow(...)]` attributes. If a warning indicates dead code, remove it.
If it flags a function as too long, refactor it. Silencing warnings hides
real problems.

For genuine false positives, use `#[expect(..., reason = "...")]` instead
of `#[allow]` so the suppression self-documents and warns if it becomes
unnecessary.

## Commit messages

Follow [Conventional Commits](https://www.conventionalcommits.org/).

Format: `<type>: <description>` or `<type>(scope): <description>`

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`,
`ci`, `chore`.

Use `!` after the type for breaking changes: `feat!: remove legacy endpoint`.

## Project context

See the root [`AGENTS.md`](../AGENTS.md) for spec, YAML test suite, and
scope discipline. See the root [`SPEC.md`](../SPEC.md) for the canonical
product contract.
