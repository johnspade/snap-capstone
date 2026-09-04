Status: ready-for-agent

# 14 — HTTP server and client

## What to build

`snap --serve` (§7.9) and HTTP repository fetching for remote merge and diff (§9).

**Server (`tiny_http` + `signal_hook`):**
- Validate and snapshot the current repository at startup
- Bind to `127.0.0.1`; port defaults to `8765`, `0` selects an OS-assigned port
- Print and flush `http://127.0.0.1:<actual-port>/repository.json` (always plain, even in terminal mode)
- `GET /repository.json` returns the startup snapshot with `Content-Type: application/json; charset=utf-8`
- `HEAD /repository.json` returns same status and headers without body
- Other paths return `404`; other methods return `405` with `Allow: GET, HEAD`
- Serve until SIGINT or SIGTERM, then exit 0

**Client (`ureq`):**
- When a repository operand starts with `http://` or `https://`, perform one GET of that exact URL
- Require status 200, parse body as repository JSON, validate normally
- HTTP is read-only — no redirects, no caching, no auth
- Use for `snap merge <url>` and `snap diff <old> <new> --repo <url>`

**Remote merge:** Same as local merge but the other repository is fetched via HTTP.

**Remote diff:** Same as cross-repo diff but the remote repository is fetched via HTTP.

## Acceptance criteria

- [ ] `--serve` binds, prints URL, and serves repository snapshot
- [ ] GET returns JSON with correct Content-Type
- [ ] HEAD returns headers without body
- [ ] 404 for unknown paths, 405 for wrong methods with Allow header
- [ ] Signal handling: clean exit on SIGINT/SIGTERM
- [ ] Startup URL is always plain (no ANSI escapes)
- [ ] HTTP client fetches, parses, and validates remote repositories
- [ ] Remote merge works end-to-end (merge from a running --serve)
- [ ] Remote diff works end-to-end (diff --repo with HTTP URL)
- [ ] Malformed remote (bad JSON, invalid repo) rejected without local mutation
- [ ] Integration tests with actual HTTP round-trips
- [ ] YAML acceptance: `12-http-server`, `13-http-client`

## Blocked by

- `13-merge-namespace-conflicts-and-convergence` — remote merge needs complete merge implementation
- `09-diff-command` — remote diff needs cross-repo diff support
