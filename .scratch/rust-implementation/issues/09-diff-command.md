Status: ready-for-agent

# 09 — diff command

## What to build

`snap diff` in all its modes (§7.6).

**No arguments:** Compare the current tree with the working tree.

**Two versions:** `snap diff <old> <new>` compares two locally known versions.

**Cross-repo:** `snap diff <old> <new> --repo <repository>` resolves `old` locally and `new` in another local repository. Validate every repository and version before producing output. For cross-repo diff, compare every dot present in both repositories and fail if parsed patch values differ (corruption check).

**Output format:** Changed paths sorted by path. For each text path, print unified-style:
```
--- a/<path>    (or /dev/null for absent)
+++ b/<path>    (or /dev/null for absent)
@@ -1,<old-count> +1,<new-count> @@
 <retained>
-<deleted>
+<inserted>
```

A token without final LF is followed by LF then `\ No newline at end of file`.

For binary changes: `Binary files a/<path> and b/<path> differ` (with `/dev/null` substitution for absent sides).

No differences = no stdout, success exit.

## Acceptance criteria

- [ ] Working-tree diff (no arguments)
- [ ] Two-version local diff
- [ ] Cross-repo diff with `--repo` flag
- [ ] Version validation: both versions must be known (materializable)
- [ ] Cross-repo dot corruption check
- [ ] Unified diff output with exact format per §7.6
- [ ] `/dev/null` substitution for absent sides
- [ ] No-newline-at-end-of-file marker
- [ ] Binary diff output line
- [ ] Empty diff = no output + success
- [ ] Integration tests for each diff mode
- [ ] YAML acceptance: `05-diff-goldens`

## Blocked by

- `08-commit-status-log` — needs commits to create repos with history to diff
