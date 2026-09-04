#![cfg(not(miri))]

use std::path::Path;
use std::process::Command;

fn snap_bin() -> std::path::PathBuf {
    let mut path = std::env::current_exe().unwrap();
    path.pop(); // remove test binary name
    path.pop(); // remove deps/
    path.push("snap");
    path
}

struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("home")).unwrap();
        std::fs::create_dir_all(dir.path().join("tmp")).unwrap();
        Self { dir }
    }

    fn run_in(&self, cwd: &Path, args: &[&str]) -> Output {
        let mut cmd = Command::new(snap_bin());
        cmd.args(args)
            .current_dir(cwd)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", self.dir.path().join("home"))
            .env("NO_COLOR", "1");
        if let Ok(val) = std::env::var("LLVM_PROFILE_FILE") {
            cmd.env("LLVM_PROFILE_FILE", val);
        }
        let output = cmd.output().unwrap();
        Output {
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
            exit_code: output.status.code().unwrap(),
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_in(self.dir.path(), args)
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }
}

struct Output {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

// ── --version ──────────────────────────────────────────────────────

#[test]
fn version_prints_semver() {
    let sb = Sandbox::new();
    let out = sb.run(&["--version"]);
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.starts_with("snap "), "stdout: {:?}", out.stdout);
    let version = out.stdout.trim().strip_prefix("snap ").unwrap();
    assert!(version.split('.').count() == 3, "not semver: {version}");
    assert_eq!(out.stderr, "");
}

#[test]
fn version_rejects_extra_args() {
    let sb = Sandbox::new();
    let out = sb.run(&["--version", "extra"]);
    assert_eq!(out.exit_code, 1);
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "snap: invalid command or arguments\n");
}

// ── init ───────────────────────────────────────────────────────────

#[test]
fn init_creates_empty_repo() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let out = sb.run_in(&repo, &["init"]);

    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, "()\n");
    assert_eq!(out.stderr, "");

    let repo_json = std::fs::read_to_string(repo.join(".snap/repository.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&repo_json).unwrap();
    assert_eq!(parsed["format"], 1);
    assert_eq!(parsed["frontier"], serde_json::json!([]));
    assert_eq!(parsed["patches"], serde_json::json!([]));
}

#[test]
fn init_preserves_existing_files() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    std::fs::write(repo.join("existing.txt"), "keep me\n").unwrap();

    let out = sb.run_in(&repo, &["init"]);
    assert_eq!(out.exit_code, 0);

    assert_eq!(
        std::fs::read_to_string(repo.join("existing.txt")).unwrap(),
        "keep me\n"
    );
}

#[test]
fn init_rejects_reinit() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();

    let out = sb.run_in(&repo, &["init"]);
    assert_eq!(out.exit_code, 0);

    let out = sb.run_in(&repo, &["init"]);
    assert_eq!(out.exit_code, 1);
    assert_eq!(out.stdout, "");
    assert!(
        out.stderr.contains("repository already exists"),
        "stderr: {:?}",
        out.stderr
    );
}

#[test]
fn init_rejects_nesting() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);

    let child = repo.join("child");
    std::fs::create_dir(&child).unwrap();

    let out = sb.run_in(&child, &["init"]);
    assert_eq!(out.exit_code, 1);
    assert_eq!(out.stdout, "");
    assert!(
        out.stderr.contains("cannot initialize inside repository"),
        "stderr: {:?}",
        out.stderr
    );
    assert!(!child.join(".snap").exists());
}

#[test]
fn init_with_path_creates_repo() {
    let sb = Sandbox::new();
    let out = sb.run(&["init", "new/repository"]);
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, "()\n");
    assert!(
        sb.path()
            .join("new/repository/.snap/repository.json")
            .exists()
    );
}

#[test]
fn init_rejects_extra_args() {
    let sb = Sandbox::new();
    let out = sb.run(&["init", "a", "b"]);
    assert_eq!(out.exit_code, 1);
    assert_eq!(out.stderr, "snap: invalid command or arguments\n");
}

#[test]
fn init_rejects_unknown_flag() {
    let sb = Sandbox::new();
    let out = sb.run(&["init", "--unknown"]);
    assert_eq!(out.exit_code, 1);
    assert_eq!(out.stderr, "snap: invalid command or arguments\n");
    assert!(!sb.path().join("--unknown").exists());
}

// ── config ─────────────────────────────────────────────────────────

#[test]
fn config_global_writes_snapconfig() {
    let sb = Sandbox::new();
    let out = sb.run(&["config", "--global", "contributor.id", "global@example.com"]);
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "");

    let path = sb.path().join("home/.snapconfig.json");
    let content = std::fs::read_to_string(path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(
        parsed,
        serde_json::json!({"contributor": {"id": "global@example.com"}})
    );
}

#[test]
fn config_local_writes_in_repo() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);

    let out = sb.run_in(&repo, &["config", "contributor.id", "local@example.com"]);
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "");

    let path = repo.join(".snap/config.json");
    let content = std::fs::read_to_string(path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(
        parsed,
        serde_json::json!({"contributor": {"id": "local@example.com"}})
    );
}

#[test]
fn config_rejects_invalid_id() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);

    let out = sb.run_in(&repo, &["config", "contributor.id", "bad-id"]);
    assert_eq!(out.exit_code, 1);
    assert_eq!(out.stdout, "");
    assert!(
        out.stderr.contains("invalid contributor id"),
        "stderr: {:?}",
        out.stderr
    );
}

#[test]
fn config_local_requires_repo() {
    let sb = Sandbox::new();
    let out = sb.run(&["config", "contributor.id", "a@x"]);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("not a Snap repository"),
        "stderr: {:?}",
        out.stderr
    );
}

#[test]
fn config_global_after_positionals_is_invalid() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);

    let out = sb.run_in(&repo, &["config", "contributor.id", "a@x", "--global"]);
    assert_eq!(out.exit_code, 1);
    assert_eq!(out.stderr, "snap: invalid command or arguments\n");
}

#[test]
fn config_duplicate_global_is_invalid() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);

    let out = sb.run_in(
        &repo,
        &["config", "--global", "--global", "contributor.id", "a@x"],
    );
    assert_eq!(out.exit_code, 1);
    assert_eq!(out.stderr, "snap: invalid command or arguments\n");
}

#[test]
fn config_overwrites_and_strips_unknown_fields() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);

    let config_path = repo.join(".snap/config.json");
    std::fs::write(
        &config_path,
        r#"{"contributor":{"id":"old@x"},"unknown":true}"#,
    )
    .unwrap();

    let out = sb.run_in(&repo, &["config", "contributor.id", "new@x"]);
    assert_eq!(out.exit_code, 0);

    let content = std::fs::read_to_string(config_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(parsed, serde_json::json!({"contributor": {"id": "new@x"}}));
}

// ── unknown commands ───────────────────────────────────────────────

#[test]
fn unknown_command_errors() {
    let sb = Sandbox::new();
    let out = sb.run(&["unknown"]);
    assert_eq!(out.exit_code, 1);
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "snap: invalid command or arguments\n");
}

#[test]
fn no_args_errors() {
    let sb = Sandbox::new();
    let out = sb.run(&[]);
    assert_eq!(out.exit_code, 1);
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "snap: invalid command or arguments\n");
}

// ── repo-requiring commands without repo ───────────────────────────

#[test]
fn status_without_repo_errors() {
    let sb = Sandbox::new();
    let out = sb.run(&["status"]);
    assert_eq!(out.exit_code, 1);
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "snap: not a Snap repository\n");
}

#[test]
fn log_without_repo_errors() {
    let sb = Sandbox::new();
    let out = sb.run(&["log"]);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("not a Snap repository"));
}

// ── diff usage message ─────────────────────────────────────────────

#[test]
fn diff_wrong_arg_count_shows_usage() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);

    let out = sb.run_in(&repo, &["diff", "()"]);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("usage: snap diff"));
}

#[test]
fn diff_extra_args_shows_usage() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);

    let out = sb.run_in(&repo, &["diff", "()", "()", "--unknown", "repo"]);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("usage: snap diff"));
}

// ── diff two versions ─────────────────────────────────────────────

fn setup_two_version_repo(sb: &Sandbox) -> std::path::PathBuf {
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);
    sb.run_in(&repo, &["config", "contributor.id", "a@x"]);
    std::fs::write(repo.join("f.txt"), "old\n").unwrap();
    sb.run_in(&repo, &["commit", "first"]);
    std::fs::write(repo.join("f.txt"), "new\n").unwrap();
    sb.run_in(&repo, &["commit", "second"]);
    repo
}

#[test]
fn diff_two_versions_shows_changes() {
    let sb = Sandbox::new();
    let repo = setup_two_version_repo(&sb);
    let out = sb.run_in(&repo, &["diff", "(a@x->1)", "(a@x->2)"]);
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stderr, "");
    assert!(out.stdout.contains("--- a/f.txt\n+++ b/f.txt\n"));
    assert!(out.stdout.contains("-old\n"));
    assert!(out.stdout.contains("+new\n"));
}

#[test]
fn diff_same_version_no_output() {
    let sb = Sandbox::new();
    let repo = setup_two_version_repo(&sb);
    let out = sb.run_in(&repo, &["diff", "(a@x->1)", "(a@x->1)"]);
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "");
}

#[test]
fn diff_empty_to_version() {
    let sb = Sandbox::new();
    let repo = setup_two_version_repo(&sb);
    let out = sb.run_in(&repo, &["diff", "()", "(a@x->1)"]);
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stderr, "");
    assert!(out.stdout.contains("--- /dev/null\n+++ b/f.txt\n"));
    assert!(out.stdout.contains("+old\n"));
}

#[test]
fn diff_version_to_empty() {
    let sb = Sandbox::new();
    let repo = setup_two_version_repo(&sb);
    let out = sb.run_in(&repo, &["diff", "(a@x->1)", "()"]);
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stderr, "");
    assert!(out.stdout.contains("--- a/f.txt\n+++ /dev/null\n"));
    assert!(out.stdout.contains("-old\n"));
}

#[test]
fn diff_unknown_version_fails() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);
    let out = sb.run_in(&repo, &["diff", "(unknown@x->1)", "()"]);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("unknown version"));
}

#[test]
fn diff_invalid_version_syntax_fails() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);
    let out = sb.run_in(&repo, &["diff", "not-a-version", "()"]);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("invalid version"));
}

// ── diff cross-repo ───────────────────────────────────────────────

#[test]
fn diff_cross_repo_shows_changes() {
    let sb = Sandbox::new();
    let local = sb.path().join("local");
    let remote = sb.path().join("remote");
    std::fs::create_dir(&local).unwrap();
    std::fs::create_dir(&remote).unwrap();

    sb.run_in(&local, &["init"]);
    sb.run_in(&local, &["config", "contributor.id", "a@x"]);
    std::fs::write(local.join("f.txt"), "local\n").unwrap();
    sb.run_in(&local, &["commit", "local"]);

    sb.run_in(&remote, &["init"]);
    sb.run_in(&remote, &["config", "contributor.id", "b@y"]);
    std::fs::write(remote.join("g.txt"), "remote\n").unwrap();
    sb.run_in(&remote, &["commit", "remote"]);

    let out = sb.run_in(
        &local,
        &[
            "diff",
            "(a@x->1)",
            "(b@y->1)",
            "--repo",
            remote.to_str().unwrap(),
        ],
    );
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stderr, "");
    assert!(out.stdout.contains("--- a/f.txt\n+++ /dev/null\n"));
    assert!(out.stdout.contains("--- /dev/null\n+++ b/g.txt\n"));
}

#[test]
fn diff_cross_repo_dot_collision_fails() {
    let sb = Sandbox::new();
    let local = sb.path().join("local");
    let remote = sb.path().join("remote");
    std::fs::create_dir(&local).unwrap();
    std::fs::create_dir(&remote).unwrap();

    sb.run_in(&local, &["init"]);
    sb.run_in(&local, &["config", "contributor.id", "a@x"]);
    std::fs::write(local.join("f.txt"), "local\n").unwrap();
    sb.run_in(&local, &["commit", "local"]);

    sb.run_in(&remote, &["init"]);
    sb.run_in(&remote, &["config", "contributor.id", "a@x"]);
    std::fs::write(remote.join("f.txt"), "different\n").unwrap();
    sb.run_in(&remote, &["commit", "different"]);

    let out = sb.run_in(
        &local,
        &["diff", "()", "(a@x->1)", "--repo", remote.to_str().unwrap()],
    );
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("patch collision"));
    assert_eq!(out.stdout, "");
}

#[test]
fn diff_cross_repo_not_a_repo_fails() {
    let sb = Sandbox::new();
    let local = sb.path().join("local");
    std::fs::create_dir(&local).unwrap();
    sb.run_in(&local, &["init"]);

    let out = sb.run_in(&local, &["diff", "()", "()", "--repo", "/nonexistent"]);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("not a Snap repository"));
}

#[test]
fn diff_cross_repo_shared_dots_identical_ok() {
    let sb = Sandbox::new();
    let local = sb.path().join("local");
    let remote = sb.path().join("remote");
    std::fs::create_dir(&local).unwrap();
    std::fs::create_dir(&remote).unwrap();

    sb.run_in(&local, &["init"]);
    sb.run_in(&local, &["config", "contributor.id", "a@x"]);
    std::fs::write(local.join("f.txt"), "shared\n").unwrap();
    sb.run_in(&local, &["commit", "shared"]);

    let local_repo = std::fs::read_to_string(local.join(".snap/repository.json")).unwrap();
    std::fs::create_dir(remote.join(".snap")).unwrap();
    std::fs::write(remote.join(".snap/repository.json"), &local_repo).unwrap();

    let out = sb.run_in(
        &local,
        &[
            "diff",
            "(a@x->1)",
            "(a@x->1)",
            "--repo",
            remote.to_str().unwrap(),
        ],
    );
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "");
}

// ── --serve grammar ────────────────────────────────────────────────

#[test]
fn serve_extra_args_is_invalid() {
    let sb = Sandbox::new();
    let out = sb.run(&["--serve", "0", "extra"]);
    assert_eq!(out.exit_code, 1);
    assert_eq!(out.stderr, "snap: invalid command or arguments\n");
}

// ── init finds repo in parent directories ──────────────────────────

#[test]
fn init_detects_repo_in_grandparent() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);

    let deep = repo.join("a/b/c");
    std::fs::create_dir_all(&deep).unwrap();

    let out = sb.run_in(&deep, &["init"]);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("cannot initialize inside repository"));
}

// ── config from subdirectory finds repo ────────────────────────────

#[test]
fn config_from_subdirectory_finds_repo() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);

    let sub = repo.join("sub");
    std::fs::create_dir(&sub).unwrap();

    let out = sb.run_in(&sub, &["config", "contributor.id", "a@x"]);
    assert_eq!(out.exit_code, 0);

    let config = std::fs::read_to_string(repo.join(".snap/config.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&config).unwrap();
    assert_eq!(parsed["contributor"]["id"], "a@x");
}

// ── revert ────────────────────────────────────────────────────────

fn setup_repo_with_commit(sb: &Sandbox) -> std::path::PathBuf {
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let out = sb.run_in(&repo, &["init"]);
    assert_eq!(out.exit_code, 0, "init: {}", out.stderr);
    let out = sb.run_in(&repo, &["config", "contributor.id", "a@x"]);
    assert_eq!(out.exit_code, 0, "config: {}", out.stderr);
    std::fs::write(repo.join("f.txt"), "hello\n").unwrap();
    let out = sb.run_in(&repo, &["commit", "first"]);
    assert_eq!(out.exit_code, 0, "commit: {}", out.stderr);
    assert_eq!(out.stdout, "(a@x->1)\n");
    repo
}

#[test]
fn revert_to_empty_version() {
    let sb = Sandbox::new();
    let repo = setup_repo_with_commit(&sb);

    let out = sb.run_in(&repo, &["revert", "()"]);
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, "(a@x->2)\n");
    assert_eq!(out.stderr, "");
    assert!(!repo.join("f.txt").exists());
}

#[test]
fn revert_restores_file_content() {
    let sb = Sandbox::new();
    let repo = setup_repo_with_commit(&sb);

    std::fs::write(repo.join("f.txt"), "modified\n").unwrap();
    let out = sb.run_in(&repo, &["commit", "modify"]);
    assert_eq!(out.exit_code, 0);

    let out = sb.run_in(&repo, &["revert", "(a@x->1)"]);
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, "(a@x->3)\n");
    assert_eq!(
        std::fs::read_to_string(repo.join("f.txt")).unwrap(),
        "hello\n"
    );
}

#[test]
fn revert_is_additive() {
    let sb = Sandbox::new();
    let repo = setup_repo_with_commit(&sb);

    std::fs::write(repo.join("f.txt"), "v2\n").unwrap();
    let out = sb.run_in(&repo, &["commit", "second"]);
    assert_eq!(out.exit_code, 0);

    let out = sb.run_in(&repo, &["revert", "(a@x->1)"]);
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, "(a@x->3)\n");

    let out = sb.run_in(&repo, &["log"]);
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("revert to (a@x->1)"));
    assert_eq!(out.stdout.lines().count(), 3);
}

#[test]
fn revert_unknown_version_fails() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);
    sb.run_in(&repo, &["config", "contributor.id", "a@x"]);

    let out = sb.run_in(&repo, &["revert", "(unknown@x->1)"]);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("unknown version"));
}

#[test]
fn revert_same_tree_fails() {
    let sb = Sandbox::new();
    let repo = setup_repo_with_commit(&sb);

    let out = sb.run_in(&repo, &["revert", "(a@x->1)"]);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("target tree is already current"));
}

#[test]
fn revert_dirty_tree_fails() {
    let sb = Sandbox::new();
    let repo = setup_repo_with_commit(&sb);

    std::fs::write(repo.join("f.txt"), "modified\n").unwrap();
    let out = sb.run_in(&repo, &["commit", "second"]);
    assert_eq!(out.exit_code, 0);

    std::fs::write(repo.join("dirty"), "dirty").unwrap();
    let out = sb.run_in(&repo, &["revert", "(a@x->1)"]);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("working tree is dirty"));
}

#[test]
fn revert_requires_contributor() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);

    let out = sb.run_in(&repo, &["revert", "()"]);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("contributor.id is required"));
}

#[test]
fn revert_invalid_version_fails() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);

    let out = sb.run_in(&repo, &["revert", "not-a-version"]);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("invalid version"));
}

#[test]
fn revert_file_to_directory_transition() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);
    sb.run_in(&repo, &["config", "contributor.id", "a@x"]);

    std::fs::write(repo.join("node"), "file\n").unwrap();
    sb.run_in(&repo, &["commit", "file"]);

    std::fs::remove_file(repo.join("node")).unwrap();
    std::fs::create_dir(repo.join("node")).unwrap();
    std::fs::write(repo.join("node/child"), "child\n").unwrap();
    sb.run_in(&repo, &["commit", "directory"]);

    let out = sb.run_in(&repo, &["revert", "(a@x->1)"]);
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, "(a@x->3)\n");
    assert_eq!(
        std::fs::read_to_string(repo.join("node")).unwrap(),
        "file\n"
    );
    assert!(!repo.join("node/child").exists());
}

#[test]
fn revert_without_repo_fails() {
    let sb = Sandbox::new();
    let out = sb.run(&["revert", "()"]);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("not a Snap repository"));
}

// ── merge ─────────────────────────────────────────────────────────

fn setup_seeded_repos(sb: &Sandbox) -> (std::path::PathBuf, std::path::PathBuf) {
    let left = sb.path().join("left");
    std::fs::create_dir(&left).unwrap();
    sb.run_in(&left, &["init"]);
    sb.run_in(&left, &["config", "contributor.id", "seed@x"]);
    std::fs::write(left.join("notes.txt"), "base\n").unwrap();
    sb.run_in(&left, &["commit", "base"]);

    // copy_tree: copy left to right
    let right = sb.path().join("right");
    copy_dir_all(&left, &right);

    sb.run_in(&left, &["config", "contributor.id", "alice@x"]);
    sb.run_in(&right, &["config", "contributor.id", "bob@x"]);

    (left, right)
}

fn copy_dir_all(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let ft = entry.file_type().unwrap();
        let target = to.join(entry.file_name());
        if ft.is_dir() {
            copy_dir_all(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
fn merge_concurrent_text_edits() {
    let sb = Sandbox::new();
    let (left, right) = setup_seeded_repos(&sb);

    std::fs::write(left.join("notes.txt"), "base\nleft\n").unwrap();
    std::fs::write(right.join("notes.txt"), "base\nright\n").unwrap();

    sb.run_in(&left, &["commit", "left"]);
    sb.run_in(&right, &["commit", "right"]);

    let out = sb.run_in(&left, &["merge", right.to_str().unwrap()]);
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stdout, "(alice@x->1,bob@x->1,seed@x->1)\n");
    assert_eq!(out.stderr, "");

    let content = std::fs::read_to_string(left.join("notes.txt")).unwrap();
    assert_eq!(content, "base\nright\nleft\n");
}

#[test]
fn merge_bidirectional_convergence() {
    let sb = Sandbox::new();
    let (left, right) = setup_seeded_repos(&sb);

    std::fs::write(left.join("notes.txt"), "base\nleft\n").unwrap();
    std::fs::write(right.join("notes.txt"), "base\nright\n").unwrap();

    sb.run_in(&left, &["commit", "left"]);
    sb.run_in(&right, &["commit", "right"]);

    let out1 = sb.run_in(&left, &["merge", right.to_str().unwrap()]);
    assert_eq!(out1.exit_code, 0, "merge into left failed: {}", out1.stderr);
    let out2 = sb.run_in(&right, &["merge", left.to_str().unwrap()]);
    assert_eq!(out2.exit_code, 0, "merge into right failed: {}", out2.stderr);

    let left_notes = std::fs::read_to_string(left.join("notes.txt")).unwrap();
    let right_notes = std::fs::read_to_string(right.join("notes.txt")).unwrap();
    assert_eq!(left_notes, right_notes, "trees must converge");
}

#[test]
fn merge_idempotent() {
    let sb = Sandbox::new();
    let (left, right) = setup_seeded_repos(&sb);

    std::fs::write(left.join("notes.txt"), "base\nleft\n").unwrap();
    std::fs::write(right.join("notes.txt"), "base\nright\n").unwrap();

    sb.run_in(&left, &["commit", "left"]);
    sb.run_in(&right, &["commit", "right"]);

    let out1 = sb.run_in(&left, &["merge", right.to_str().unwrap()]);
    assert_eq!(out1.exit_code, 0, "first merge failed: {}", out1.stderr);

    let out = sb.run_in(&left, &["merge", right.to_str().unwrap()]);
    assert_eq!(out.exit_code, 0, "re-merge failed: {}", out.stderr);
    assert_eq!(out.stdout, "(alice@x->1,bob@x->1,seed@x->1)\n");
    assert_eq!(out.stderr, "");
}

#[test]
fn merge_equal_history_is_noop() {
    let sb = Sandbox::new();
    let left = sb.path().join("left");
    std::fs::create_dir(&left).unwrap();
    sb.run_in(&left, &["init"]);

    let right = sb.path().join("right");
    copy_dir_all(&left, &right);

    let out = sb.run_in(&left, &["merge", right.to_str().unwrap()]);
    assert_eq!(out.exit_code, 0, "merge failed: {}", out.stderr);
    assert_eq!(out.stdout, "()\n");
    assert_eq!(out.stderr, "");
}

#[test]
fn merge_refuses_dirty_tree() {
    let sb = Sandbox::new();
    let local = sb.path().join("local");
    std::fs::create_dir(&local).unwrap();
    sb.run_in(&local, &["init"]);

    let remote = sb.path().join("remote");
    std::fs::create_dir(&remote).unwrap();
    sb.run_in(&remote, &["init"]);
    sb.run_in(&remote, &["config", "contributor.id", "r@x"]);
    std::fs::write(remote.join("f.txt"), "remote\n").unwrap();
    sb.run_in(&remote, &["commit", "remote"]);

    std::fs::write(local.join("dirty.txt"), "dirty\n").unwrap();

    let out = sb.run_in(&local, &["merge", remote.to_str().unwrap()]);
    assert_eq!(out.exit_code, 1);
    assert_eq!(out.stderr, "snap: working tree is dirty\n");
    assert_eq!(out.stdout, "");

    // Verify no mutation occurred
    assert!(std::fs::read_to_string(local.join("dirty.txt")).unwrap() == "dirty\n");
    assert!(!local.join("f.txt").exists());
}

#[test]
fn merge_refuses_symlink_in_working_tree() {
    let sb = Sandbox::new();
    let local = sb.path().join("local");
    std::fs::create_dir(&local).unwrap();
    sb.run_in(&local, &["init"]);

    let remote = sb.path().join("remote");
    std::fs::create_dir(&remote).unwrap();
    sb.run_in(&remote, &["init"]);

    std::os::unix::fs::symlink("nonexistent", local.join("link")).unwrap();

    let out = sb.run_in(&local, &["merge", remote.to_str().unwrap()]);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("unsupported working tree entry: link"));
}

#[test]
fn merge_identical_concurrent_changes_no_warning() {
    let sb = Sandbox::new();
    let (left, right) = setup_seeded_repos(&sb);

    // Both make the exact same change
    std::fs::write(left.join("notes.txt"), "base\nsame\n").unwrap();
    std::fs::write(right.join("notes.txt"), "base\nsame\n").unwrap();

    sb.run_in(&left, &["commit", "identical"]);
    sb.run_in(&right, &["commit", "identical"]);

    let out = sb.run_in(&left, &["merge", right.to_str().unwrap()]);
    assert_eq!(out.exit_code, 0, "merge failed: {}", out.stderr);
    assert_eq!(out.stderr, "");

    let content = std::fs::read_to_string(left.join("notes.txt")).unwrap();
    assert_eq!(content, "base\nsame\n");
}

#[test]
fn merge_no_repo_errors() {
    let sb = Sandbox::new();
    let out = sb.run(&["merge", "/tmp/nowhere"]);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("not a Snap repository"));
}
