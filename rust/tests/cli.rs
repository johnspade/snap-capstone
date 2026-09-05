#![cfg(not(miri))]

use std::io::{BufRead, BufReader, Write as _};
use std::path::Path;
use std::process::{Command, Stdio};

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
        self.run_with_env(cwd, args, &[("NO_COLOR", "1")])
    }

    fn run_color_in(&self, cwd: &Path, args: &[&str]) -> Output {
        self.run_with_env(cwd, args, &[("SNAP_COLOR", "always")])
    }

    fn run_env_in(&self, cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
        self.run_with_env(cwd, args, env)
    }

    fn run_with_env(&self, cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(snap_bin());
        cmd.args(args)
            .current_dir(cwd)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", self.dir.path().join("home"));
        for (k, v) in env {
            cmd.env(k, v);
        }
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

    fn start_serve(&self, cwd: &Path, port: &str) -> SnapServer {
        let mut cmd = Command::new(snap_bin());
        cmd.args(["--serve", port])
            .current_dir(cwd)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", self.dir.path().join("home"))
            .env("NO_COLOR", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Ok(val) = std::env::var("LLVM_PROFILE_FILE") {
            cmd.env("LLVM_PROFILE_FILE", val);
        }
        let mut child = cmd.spawn().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout);
        let mut url_line = String::new();
        reader.read_line(&mut url_line).unwrap();
        let url = url_line.trim().to_owned();
        SnapServer { child, url }
    }
}

struct SnapServer {
    child: std::process::Child,
    url: String,
}

impl SnapServer {
    fn url(&self) -> &str {
        &self.url
    }

    fn stop(self) -> Output {
        Command::new("kill")
            .args(["-s", "TERM", &self.child.id().to_string()])
            .status()
            .unwrap();
        let output = self.child.wait_with_output().unwrap();
        Output {
            stdout: String::from_utf8(output.stdout).unwrap_or_default(),
            stderr: String::from_utf8(output.stderr).unwrap(),
            exit_code: output.status.code().unwrap_or(-1),
        }
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
    assert_eq!(
        out2.exit_code, 0,
        "merge into right failed: {}",
        out2.stderr
    );

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

#[test]
fn merge_invalid_remote_exits_1() {
    let sb = Sandbox::new();
    let local = sb.path().join("local");
    std::fs::create_dir(&local).unwrap();
    sb.run_in(&local, &["init"]);

    let out = sb.run_in(&local, &["merge", "/tmp/nowhere"]);
    assert_eq!(out.exit_code, 1, "stderr: {}", out.stderr);
}

// ── merge: path-level conflict rules (§6.4) ──────────────────────

fn setup_conflict_repos(sb: &Sandbox) -> (std::path::PathBuf, std::path::PathBuf) {
    let base = sb.path().join("base");
    std::fs::create_dir(&base).unwrap();
    sb.run_in(&base, &["init"]);
    sb.run_in(&base, &["config", "contributor.id", "seed@x"]);
    (base, sb.path().join("right"))
}

#[test]
fn merge_rule1_identical_result_no_warning() {
    let sb = Sandbox::new();
    let (base, right) = setup_conflict_repos(&sb);

    std::fs::write(base.join("f.txt"), "original\n").unwrap();
    sb.run_in(&base, &["commit", "seed"]);

    copy_dir_all(&base, &right);
    sb.run_in(&base, &["config", "contributor.id", "alice@x"]);
    sb.run_in(&right, &["config", "contributor.id", "bob@x"]);

    std::fs::write(base.join("f.txt"), "same\n").unwrap();
    std::fs::write(right.join("f.txt"), "same\n").unwrap();
    sb.run_in(&base, &["commit", "alice"]);
    sb.run_in(&right, &["commit", "bob"]);

    let out = sb.run_in(&base, &["merge", right.to_str().unwrap()]);
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stderr, "",
        "identical results should produce no warnings"
    );
    let content = std::fs::read_to_string(base.join("f.txt")).unwrap();
    assert_eq!(content, "same\n");
}

#[test]
fn merge_rule2_incoming_delete_wins() {
    let sb = Sandbox::new();
    let (base, right) = setup_conflict_repos(&sb);

    std::fs::write(base.join("f.txt"), "original\n").unwrap();
    sb.run_in(&base, &["commit", "seed"]);

    copy_dir_all(&base, &right);
    sb.run_in(&base, &["config", "contributor.id", "alice@x"]);
    sb.run_in(&right, &["config", "contributor.id", "bob@x"]);

    std::fs::write(base.join("f.txt"), "modified\n").unwrap();
    sb.run_in(&base, &["commit", "modify"]);

    std::fs::remove_file(right.join("f.txt")).unwrap();
    sb.run_in(&right, &["commit", "delete"]);

    let out = sb.run_in(&base, &["merge", right.to_str().unwrap()]);
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert!(out.stderr.contains("delete-wins"), "stderr: {}", out.stderr);
    assert!(!base.join("f.txt").exists());
}

#[test]
fn merge_rule3_earlier_concurrent_delete_wins() {
    let sb = Sandbox::new();
    let (base, right) = setup_conflict_repos(&sb);

    std::fs::write(base.join("f.txt"), "original\n").unwrap();
    sb.run_in(&base, &["commit", "seed"]);

    copy_dir_all(&base, &right);
    sb.run_in(&base, &["config", "contributor.id", "alice@x"]);
    sb.run_in(&right, &["config", "contributor.id", "bob@x"]);

    std::fs::remove_file(base.join("f.txt")).unwrap();
    sb.run_in(&base, &["commit", "delete"]);

    std::fs::write(right.join("f.txt"), "replaced\n").unwrap();
    sb.run_in(&right, &["commit", "replace"]);

    let out = sb.run_in(&base, &["merge", right.to_str().unwrap()]);
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert!(out.stderr.contains("delete-wins"), "stderr: {}", out.stderr);
    assert!(!base.join("f.txt").exists());
}

#[test]
fn merge_rule4_later_create_wins() {
    let sb = Sandbox::new();
    let left = sb.path().join("left");
    let right = sb.path().join("right");
    std::fs::create_dir(&left).unwrap();
    std::fs::create_dir(&right).unwrap();
    sb.run_in(&left, &["init"]);
    sb.run_in(&right, &["init"]);
    sb.run_in(&left, &["config", "contributor.id", "alice@x"]);
    sb.run_in(&right, &["config", "contributor.id", "bob@x"]);

    std::fs::write(left.join("same.txt"), "alice\n").unwrap();
    std::fs::write(right.join("same.txt"), "bob\n").unwrap();
    sb.run_in(&left, &["commit", "alice creates"]);
    sb.run_in(&right, &["commit", "bob creates"]);

    let out = sb.run_in(&left, &["merge", right.to_str().unwrap()]);
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("later-create-wins"),
        "stderr: {}",
        out.stderr
    );
    let content = std::fs::read_to_string(left.join("same.txt")).unwrap();
    assert_eq!(content, "alice\n");
}

#[test]
fn merge_rule5_later_put_wins() {
    let sb = Sandbox::new();
    let (base, right) = setup_conflict_repos(&sb);

    std::fs::write(base.join("f.txt"), "original\n").unwrap();
    sb.run_in(&base, &["commit", "seed"]);

    copy_dir_all(&base, &right);
    sb.run_in(&base, &["config", "contributor.id", "alice@x"]);
    sb.run_in(&right, &["config", "contributor.id", "bob@x"]);

    std::fs::write(base.join("f.txt"), [0x00, 0x01]).unwrap();
    sb.run_in(&base, &["commit", "binary"]);

    std::fs::write(right.join("f.txt"), "text edit\n").unwrap();
    sb.run_in(&right, &["commit", "text"]);

    let out = sb.run_in(&base, &["merge", right.to_str().unwrap()]);
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("later-put-wins"),
        "stderr: {}",
        out.stderr
    );
    let content = std::fs::read(base.join("f.txt")).unwrap();
    assert_eq!(content, [0x00, 0x01], "later put content should win");
}

#[test]
fn merge_rule6_put_wins_incompatible_current() {
    let sb = Sandbox::new();
    let (base, right) = setup_conflict_repos(&sb);

    std::fs::write(base.join("f.txt"), "original\n").unwrap();
    sb.run_in(&base, &["commit", "seed"]);

    copy_dir_all(&base, &right);
    sb.run_in(&base, &["config", "contributor.id", "bob@x"]);
    sb.run_in(&right, &["config", "contributor.id", "alice@x"]);

    std::fs::write(base.join("f.txt"), [0x00, 0xFF]).unwrap();
    sb.run_in(&base, &["commit", "binary"]);

    std::fs::write(right.join("f.txt"), "edited\n").unwrap();
    sb.run_in(&right, &["commit", "text edit"]);

    let out = sb.run_in(&base, &["merge", right.to_str().unwrap()]);
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert!(out.stderr.contains("put-wins"), "stderr: {}", out.stderr);
    let content = std::fs::read(base.join("f.txt")).unwrap();
    assert_eq!(content, [0x00, 0xFF]);
}

#[test]
fn merge_warnings_sorted_by_path_then_reason() {
    let sb = Sandbox::new();
    let (base, right) = setup_conflict_repos(&sb);

    std::fs::write(base.join("beta.txt"), "original\n").unwrap();
    std::fs::write(base.join("alpha.txt"), "original\n").unwrap();
    sb.run_in(&base, &["commit", "seed"]);

    copy_dir_all(&base, &right);
    sb.run_in(&base, &["config", "contributor.id", "alice@x"]);
    sb.run_in(&right, &["config", "contributor.id", "bob@x"]);

    std::fs::write(base.join("beta.txt"), "left\n").unwrap();
    std::fs::write(base.join("alpha.txt"), "left\n").unwrap();
    sb.run_in(&base, &["commit", "left"]);

    std::fs::remove_file(right.join("beta.txt")).unwrap();
    std::fs::remove_file(right.join("alpha.txt")).unwrap();
    sb.run_in(&right, &["commit", "delete both"]);

    let out = sb.run_in(&base, &["merge", right.to_str().unwrap()]);
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    let lines: Vec<&str> = out.stderr.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("alpha.txt"), "first: {}", lines[0]);
    assert!(lines[1].contains("beta.txt"), "second: {}", lines[1]);
}

#[test]
fn merge_remerge_emits_no_new_warnings() {
    let sb = Sandbox::new();
    let (base, right) = setup_conflict_repos(&sb);

    std::fs::write(base.join("f.txt"), "original\n").unwrap();
    sb.run_in(&base, &["commit", "seed"]);

    copy_dir_all(&base, &right);
    sb.run_in(&base, &["config", "contributor.id", "alice@x"]);
    sb.run_in(&right, &["config", "contributor.id", "bob@x"]);

    std::fs::write(base.join("f.txt"), "modified\n").unwrap();
    sb.run_in(&base, &["commit", "modify"]);

    std::fs::remove_file(right.join("f.txt")).unwrap();
    sb.run_in(&right, &["commit", "delete"]);

    let out1 = sb.run_in(&base, &["merge", right.to_str().unwrap()]);
    assert_eq!(out1.exit_code, 0);
    assert!(out1.stderr.contains("delete-wins"));

    let out2 = sb.run_in(&base, &["merge", right.to_str().unwrap()]);
    assert_eq!(out2.exit_code, 0);
    assert_eq!(out2.stderr, "", "re-merge should emit no warnings");
}

// ── merge: dot collision ──────────────────────────────────────────

#[test]
fn merge_dot_collision_fails_before_mutation() {
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
    sb.run_in(&remote, &["commit", "remote"]);

    let repo_before = std::fs::read_to_string(local.join(".snap/repository.json")).unwrap();

    let out = sb.run_in(&local, &["merge", remote.to_str().unwrap()]);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("patch collision"),
        "stderr: {}",
        out.stderr
    );

    let repo_after = std::fs::read_to_string(local.join(".snap/repository.json")).unwrap();
    assert_eq!(repo_before, repo_after, "repository must not be mutated");
    assert_eq!(
        std::fs::read_to_string(local.join("f.txt")).unwrap(),
        "local\n",
        "working tree must not be mutated"
    );
}

// ── merge: namespace conflicts (§6.2) ─────────────────────────

#[test]
fn merge_namespace_file_replaces_directory() {
    // alice creates file "a", bob creates "a/b" (needs "a" as directory).
    // The incoming patch (alice's) wins — "a" stays as a file, "a/b" removed.
    let sb = Sandbox::new();
    let ancestor = sb.path().join("ancestor");
    let descendant = sb.path().join("descendant");
    std::fs::create_dir(&ancestor).unwrap();
    std::fs::create_dir(&descendant).unwrap();

    sb.run_in(&ancestor, &["init"]);
    sb.run_in(&descendant, &["init"]);
    sb.run_in(&ancestor, &["config", "contributor.id", "alice@x"]);
    sb.run_in(&descendant, &["config", "contributor.id", "bob@x"]);

    std::fs::write(ancestor.join("a"), "ancestor\n").unwrap();
    std::fs::create_dir(descendant.join("a")).unwrap();
    std::fs::write(descendant.join("a/b"), "descendant\n").unwrap();

    sb.run_in(&ancestor, &["commit", "ancestor"]);
    sb.run_in(&descendant, &["commit", "descendant"]);

    let out = sb.run_in(&ancestor, &["merge", descendant.to_str().unwrap()]);
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stderr, "warning: auto-resolved a/b: namespace-wins\n");

    let content = std::fs::read_to_string(ancestor.join("a")).unwrap();
    assert_eq!(content, "ancestor\n");
    assert!(!ancestor.join("a/b").exists());
}

#[test]
fn merge_namespace_directory_replaces_file() {
    // bob creates file "x", alice creates "x/y". alice integrates later → incoming
    // "x/y" wins, existing file "x" removed with namespace-wins.
    let sb = Sandbox::new();
    let early = sb.path().join("early");
    let late = sb.path().join("late");
    std::fs::create_dir(&early).unwrap();
    std::fs::create_dir(&late).unwrap();

    sb.run_in(&early, &["init"]);
    sb.run_in(&late, &["init"]);
    sb.run_in(&early, &["config", "contributor.id", "bob@x"]);
    sb.run_in(&late, &["config", "contributor.id", "alice@x"]);

    std::fs::write(early.join("x"), "ancestor\n").unwrap();
    std::fs::create_dir(late.join("x")).unwrap();
    std::fs::write(late.join("x/y"), "descendant\n").unwrap();

    sb.run_in(&early, &["commit", "ancestor"]);
    sb.run_in(&late, &["commit", "descendant"]);

    let out = sb.run_in(&early, &["merge", late.to_str().unwrap()]);
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stderr, "warning: auto-resolved x: namespace-wins\n");

    assert!(early.join("x").is_dir(), "x should be a directory");
    let content = std::fs::read_to_string(early.join("x/y")).unwrap();
    assert_eq!(content, "descendant\n");
}

#[test]
fn merge_namespace_conflict_bidirectional_convergence() {
    // Both directions of merge must produce the same result.
    let sb = Sandbox::new();
    let a = sb.path().join("a");
    let b = sb.path().join("b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();

    sb.run_in(&a, &["init"]);
    sb.run_in(&b, &["init"]);
    sb.run_in(&a, &["config", "contributor.id", "alice@x"]);
    sb.run_in(&b, &["config", "contributor.id", "bob@x"]);

    std::fs::write(a.join("p"), "file\n").unwrap();
    std::fs::create_dir(b.join("p")).unwrap();
    std::fs::write(b.join("p/q"), "nested\n").unwrap();

    sb.run_in(&a, &["commit", "file"]);
    sb.run_in(&b, &["commit", "nested"]);

    let a_copy = sb.path().join("a-copy");
    let b_copy = sb.path().join("b-copy");
    copy_dir_all(&a, &a_copy);
    copy_dir_all(&b, &b_copy);

    let out_ab = sb.run_in(&a, &["merge", b.to_str().unwrap()]);
    assert_eq!(out_ab.exit_code, 0, "A→B: {}", out_ab.stderr);

    let out_ba = sb.run_in(&b_copy, &["merge", a_copy.to_str().unwrap()]);
    assert_eq!(out_ba.exit_code, 0, "B→A: {}", out_ba.stderr);

    assert_eq!(out_ab.stdout, out_ba.stdout, "versions must match");

    // Check both trees have the same files
    let a_has_file = a.join("p").is_file();
    let b_has_file = b_copy.join("p").is_file();
    assert_eq!(a_has_file, b_has_file, "file presence must match");

    let a_has_nested = a.join("p/q").exists();
    let b_has_nested = b_copy.join("p/q").exists();
    assert_eq!(a_has_nested, b_has_nested, "nested presence must match");
}

#[test]
fn merge_namespace_multiple_descendants_removed() {
    // When a file replaces a directory, all files under that directory are removed.
    let sb = Sandbox::new();
    let a = sb.path().join("a");
    let b = sb.path().join("b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();

    sb.run_in(&a, &["init"]);
    sb.run_in(&b, &["init"]);
    sb.run_in(&a, &["config", "contributor.id", "alice@x"]);
    sb.run_in(&b, &["config", "contributor.id", "bob@x"]);

    // alice creates file "d"
    std::fs::write(a.join("d"), "file\n").unwrap();
    // bob creates "d/x" and "d/y" (directory structure)
    std::fs::create_dir(b.join("d")).unwrap();
    std::fs::write(b.join("d/x"), "x\n").unwrap();
    std::fs::write(b.join("d/y"), "y\n").unwrap();

    sb.run_in(&a, &["commit", "file"]);
    sb.run_in(&b, &["commit", "dir"]);

    let out = sb.run_in(&a, &["merge", b.to_str().unwrap()]);
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);

    // Both d/x and d/y should be removed with namespace-wins
    assert!(out.stderr.contains("d/x: namespace-wins"));
    assert!(out.stderr.contains("d/y: namespace-wins"));
    assert!(a.join("d").is_file(), "d should be a file");
    assert!(!a.join("d/x").exists());
    assert!(!a.join("d/y").exists());
}

#[test]
fn merge_namespace_overrides_per_path_rules() {
    // A namespace conflict takes precedence over per-path rules.
    // Both alice and bob base on a seed. alice creates "dir/child", bob creates "dir" (file).
    // Without namespace resolution, "dir" and "dir/child" would be handled by per-path rules.
    // With namespace resolution, the incoming path wins outright.
    let sb = Sandbox::new();
    let base = sb.path().join("base");
    std::fs::create_dir(&base).unwrap();
    sb.run_in(&base, &["init"]);
    sb.run_in(&base, &["config", "contributor.id", "seed@x"]);
    std::fs::write(base.join("other.txt"), "seed\n").unwrap();
    sb.run_in(&base, &["commit", "seed"]);

    let left = sb.path().join("left");
    let right = sb.path().join("right");
    copy_dir_all(&base, &left);
    copy_dir_all(&base, &right);

    sb.run_in(&left, &["config", "contributor.id", "alice@x"]);
    sb.run_in(&right, &["config", "contributor.id", "bob@x"]);

    std::fs::create_dir(left.join("node")).unwrap();
    std::fs::write(left.join("node/child"), "child\n").unwrap();
    sb.run_in(&left, &["commit", "add dir"]);

    std::fs::write(right.join("node"), "file\n").unwrap();
    sb.run_in(&right, &["commit", "add file"]);

    let out = sb.run_in(&left, &["merge", right.to_str().unwrap()]);
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("namespace-wins"),
        "stderr: {}",
        out.stderr
    );
}

// ── merge: three-way convergence ─────────────────────────────

#[test]
fn merge_three_way_text_convergence() {
    let sb = Sandbox::new();
    let base = sb.path().join("base");
    std::fs::create_dir(&base).unwrap();
    sb.run_in(&base, &["init"]);
    sb.run_in(&base, &["config", "contributor.id", "seed@x"]);
    std::fs::write(base.join("story.txt"), "start\nend\n").unwrap();
    sb.run_in(&base, &["commit", "base"]);

    let a = sb.path().join("a");
    let b = sb.path().join("b");
    let c = sb.path().join("c");
    copy_dir_all(&base, &a);
    copy_dir_all(&base, &b);
    copy_dir_all(&base, &c);

    sb.run_in(&a, &["config", "contributor.id", "a@x"]);
    sb.run_in(&b, &["config", "contributor.id", "b@x"]);
    sb.run_in(&c, &["config", "contributor.id", "c@x"]);

    std::fs::write(a.join("story.txt"), "start\nA\nend\n").unwrap();
    std::fs::write(b.join("story.txt"), "start\nB\nend\n").unwrap();
    std::fs::write(c.join("story.txt"), "end\n").unwrap();

    sb.run_in(&a, &["commit", "a"]);
    sb.run_in(&b, &["commit", "b"]);
    sb.run_in(&c, &["commit", "c"]);

    // Test all 6 association orders: (A,B,C), (A,C,B), (B,A,C), (B,C,A), (C,A,B), (C,B,A)
    let orders: [(&str, &str, &str); 6] = [
        ("a", "b", "c"),
        ("a", "c", "b"),
        ("b", "a", "c"),
        ("b", "c", "a"),
        ("c", "a", "b"),
        ("c", "b", "a"),
    ];

    let mut results: Vec<String> = Vec::new();
    let sources: std::collections::HashMap<&str, &Path> =
        [("a", a.as_path()), ("b", b.as_path()), ("c", c.as_path())]
            .iter()
            .copied()
            .collect();

    for (i, (first, second, third)) in orders.iter().enumerate() {
        let agg = sb.path().join(format!("agg-{i}"));
        copy_dir_all(sources[first], &agg);

        let out1 = sb.run_in(&agg, &["merge", sources[second].to_str().unwrap()]);
        assert_eq!(out1.exit_code, 0, "order {i} merge1: {}", out1.stderr);

        let out2 = sb.run_in(&agg, &["merge", sources[third].to_str().unwrap()]);
        assert_eq!(out2.exit_code, 0, "order {i} merge2: {}", out2.stderr);

        let content = std::fs::read_to_string(agg.join("story.txt")).unwrap();
        results.push(content);
    }

    // All 6 must produce identical content
    for (i, r) in results.iter().enumerate().skip(1) {
        assert_eq!(&results[0], r, "order 0 vs order {i} diverged");
    }
}

#[test]
fn merge_three_way_namespace_convergence() {
    // Three-way merge with namespace conflicts must converge regardless of order.
    let sb = Sandbox::new();
    let a = sb.path().join("a");
    let b = sb.path().join("b");
    let c = sb.path().join("c");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    std::fs::create_dir(&c).unwrap();

    sb.run_in(&a, &["init"]);
    sb.run_in(&b, &["init"]);
    sb.run_in(&c, &["init"]);
    sb.run_in(&a, &["config", "contributor.id", "a@x"]);
    sb.run_in(&b, &["config", "contributor.id", "b@x"]);
    sb.run_in(&c, &["config", "contributor.id", "c@x"]);

    // a creates file "n", b creates "n/child", c creates unrelated file
    std::fs::write(a.join("n"), "file\n").unwrap();
    std::fs::create_dir(b.join("n")).unwrap();
    std::fs::write(b.join("n/child"), "child\n").unwrap();
    std::fs::write(c.join("other"), "other\n").unwrap();

    sb.run_in(&a, &["commit", "a"]);
    sb.run_in(&b, &["commit", "b"]);
    sb.run_in(&c, &["commit", "c"]);

    // Merge in two different association orders and compare final versions
    let agg1 = sb.path().join("agg1");
    copy_dir_all(&a, &agg1);
    let out = sb.run_in(&agg1, &["merge", b.to_str().unwrap()]);
    assert_eq!(out.exit_code, 0, "agg1 merge1: {}", out.stderr);
    let out = sb.run_in(&agg1, &["merge", c.to_str().unwrap()]);
    assert_eq!(out.exit_code, 0, "agg1 merge2: {}", out.stderr);
    let ver1 = out.stdout;

    let agg2 = sb.path().join("agg2");
    copy_dir_all(&c, &agg2);
    let out = sb.run_in(&agg2, &["merge", b.to_str().unwrap()]);
    assert_eq!(out.exit_code, 0, "agg2 merge1: {}", out.stderr);
    let out = sb.run_in(&agg2, &["merge", a.to_str().unwrap()]);
    assert_eq!(out.exit_code, 0, "agg2 merge2: {}", out.stderr);
    let ver2 = out.stdout;

    assert_eq!(ver1, ver2, "versions must match");

    // Check trees converge: compare file presence
    let agg1_has_n_file = agg1.join("n").is_file();
    let agg2_has_n_file = agg2.join("n").is_file();
    assert_eq!(
        agg1_has_n_file, agg2_has_n_file,
        "n file presence must match"
    );

    let agg1_has_n_child = agg1.join("n/child").exists();
    let agg2_has_n_child = agg2.join("n/child").exists();
    assert_eq!(
        agg1_has_n_child, agg2_has_n_child,
        "n/child presence must match"
    );

    let agg1_has_other = agg1.join("other").exists();
    let agg2_has_other = agg2.join("other").exists();
    assert_eq!(agg1_has_other, agg2_has_other, "other presence must match");
}

// ── merge: concurrent creates bidirectional ───────────────────────

#[test]
fn merge_concurrent_creates_converge_bidirectionally() {
    let sb = Sandbox::new();
    let alice = sb.path().join("alice");
    let bob = sb.path().join("bob");
    std::fs::create_dir(&alice).unwrap();
    std::fs::create_dir(&bob).unwrap();

    sb.run_in(&alice, &["init"]);
    sb.run_in(&bob, &["init"]);
    sb.run_in(&alice, &["config", "contributor.id", "alice@x"]);
    sb.run_in(&bob, &["config", "contributor.id", "bob@x"]);

    std::fs::write(alice.join("same.txt"), "alice\n").unwrap();
    std::fs::write(bob.join("same.txt"), "bob\n").unwrap();
    sb.run_in(&alice, &["commit", "alice"]);
    sb.run_in(&bob, &["commit", "bob"]);

    let alice_copy = sb.path().join("alice-copy");
    copy_dir_all(&alice, &alice_copy);

    let out_ab = sb.run_in(&alice, &["merge", bob.to_str().unwrap()]);
    assert_eq!(out_ab.exit_code, 0, "A→B: {}", out_ab.stderr);
    assert!(out_ab.stderr.contains("later-create-wins"));

    let out_ba = sb.run_in(&bob, &["merge", alice_copy.to_str().unwrap()]);
    assert_eq!(out_ba.exit_code, 0, "B→A: {}", out_ba.stderr);
    assert!(out_ba.stderr.contains("later-create-wins"));

    let alice_content = std::fs::read_to_string(alice.join("same.txt")).unwrap();
    let bob_content = std::fs::read_to_string(bob.join("same.txt")).unwrap();
    assert_eq!(
        alice_content, bob_content,
        "trees must converge after merge in both directions"
    );
}

// ── HTTP server ──────────────────────────────────────────────────

#[test]
fn serve_prints_url_and_exits_on_sigterm() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);

    let server = sb.start_serve(&repo, "0");
    assert!(
        server.url().starts_with("http://127.0.0.1:"),
        "url: {}",
        server.url()
    );
    assert!(server.url().ends_with("/repository.json"));

    let out = server.stop();
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stderr, "");
}

#[test]
fn serve_get_returns_repository_json() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);
    sb.run_in(&repo, &["config", "contributor.id", "a@x"]);
    std::fs::write(repo.join("f.txt"), "hello\n").unwrap();
    sb.run_in(&repo, &["commit", "first"]);

    let server = sb.start_serve(&repo, "0");

    let agent = ureq::Agent::new_with_defaults();
    let resp = agent.get(server.url()).call().unwrap();
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(ct, "application/json; charset=utf-8");

    let body: serde_json::Value = resp
        .into_body()
        .read_to_string()
        .map(|s| serde_json::from_str::<serde_json::Value>(&s).unwrap())
        .unwrap();
    assert_eq!(body["format"], 1);
    assert!(body["patches"].is_array());

    server.stop();
}

#[test]
fn serve_head_returns_empty_body() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);

    let server = sb.start_serve(&repo, "0");

    let agent = ureq::Agent::new_with_defaults();
    let resp = agent.head(server.url()).call().unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.into_body().read_to_string().unwrap();
    assert!(body.is_empty());

    server.stop();
}

#[test]
fn serve_unknown_path_returns_404() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);

    let server = sb.start_serve(&repo, "0");
    let base = server.url().strip_suffix("/repository.json").unwrap();
    let url_404 = format!("{base}/nonexistent");

    let agent = ureq::config::Config::builder()
        .http_status_as_error(false)
        .build()
        .new_agent();
    let resp = agent.get(&url_404).call().unwrap();
    assert_eq!(resp.status(), 404);

    server.stop();
}

#[test]
fn serve_wrong_method_returns_405() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);

    let server = sb.start_serve(&repo, "0");

    let agent = ureq::config::Config::builder()
        .http_status_as_error(false)
        .build()
        .new_agent();
    let resp = agent.post(server.url()).send_empty().unwrap();
    assert_eq!(resp.status(), 405);
    let allow = resp.headers().get("allow").unwrap().to_str().unwrap();
    assert_eq!(allow, "GET, HEAD");

    server.stop();
}

#[test]
fn serve_snapshot_is_immutable() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);
    sb.run_in(&repo, &["config", "contributor.id", "a@x"]);
    std::fs::write(repo.join("f.txt"), "one\n").unwrap();
    sb.run_in(&repo, &["commit", "one"]);

    let server = sb.start_serve(&repo, "0");

    let agent = ureq::Agent::new_with_defaults();
    let body1: serde_json::Value = {
        let text = agent
            .get(server.url())
            .call()
            .unwrap()
            .into_body()
            .read_to_string()
            .unwrap();
        serde_json::from_str(&text).unwrap()
    };

    std::fs::write(repo.join("f.txt"), "two\n").unwrap();
    sb.run_in(&repo, &["commit", "two"]);

    let body2: serde_json::Value = {
        let text = agent
            .get(server.url())
            .call()
            .unwrap()
            .into_body()
            .read_to_string()
            .unwrap();
        serde_json::from_str(&text).unwrap()
    };

    assert_eq!(body1, body2, "snapshot must be immutable");

    server.stop();
}

#[test]
fn serve_invalid_repo_fails_at_startup() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);

    let snap_dir = repo.join(".snap");
    std::fs::write(
        snap_dir.join("repository.json"),
        r#"{"format":1,"frontier":[],"patches":[],"extra":true}"#,
    )
    .unwrap();

    let out = sb.run_in(&repo, &["--serve", "0"]);
    assert_eq!(out.exit_code, 1);
    assert!(!out.stderr.is_empty());
}

// ── HTTP client / remote merge ───────────────────────────────────

#[test]
fn remote_merge_end_to_end() {
    let sb = Sandbox::new();
    let remote = sb.path().join("remote");
    std::fs::create_dir(&remote).unwrap();
    sb.run_in(&remote, &["init"]);
    sb.run_in(&remote, &["config", "contributor.id", "remote@x"]);
    std::fs::write(remote.join("file.txt"), "remote\n").unwrap();
    sb.run_in(&remote, &["commit", "remote"]);

    let server = sb.start_serve(&remote, "0");

    let local = sb.path().join("local");
    std::fs::create_dir(&local).unwrap();
    sb.run_in(&local, &["init"]);

    let out = sb.run_in(&local, &["merge", server.url()]);
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stdout, "(remote@x->1)\n");
    assert_eq!(
        std::fs::read_to_string(local.join("file.txt")).unwrap(),
        "remote\n"
    );

    server.stop();
}

#[test]
fn remote_diff_end_to_end() {
    let sb = Sandbox::new();
    let remote = sb.path().join("remote");
    std::fs::create_dir(&remote).unwrap();
    sb.run_in(&remote, &["init"]);
    sb.run_in(&remote, &["config", "contributor.id", "remote@x"]);
    std::fs::write(remote.join("file.txt"), "remote\n").unwrap();
    sb.run_in(&remote, &["commit", "remote"]);

    let server = sb.start_serve(&remote, "0");

    let local = sb.path().join("local");
    std::fs::create_dir(&local).unwrap();
    sb.run_in(&local, &["init"]);

    let out = sb.run_in(
        &local,
        &["diff", "()", "(remote@x->1)", "--repo", server.url()],
    );
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert!(out.stdout.contains("+remote\n"));

    server.stop();
}

#[test]
fn remote_merge_malformed_json_rejected() {
    let sb = Sandbox::new();
    let local = sb.path().join("local");
    std::fs::create_dir(&local).unwrap();
    sb.run_in(&local, &["init"]);

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}/bad");

    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let body = "not-json";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let out = sb.run_in(&local, &["merge", &url]);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("invalid JSON"),
        "stderr: {}",
        out.stderr
    );

    handle.join().unwrap();
}

#[test]
fn remote_merge_non_200_rejected() {
    let sb = Sandbox::new();
    let local = sb.path().join("local");
    std::fs::create_dir(&local).unwrap();
    sb.run_in(&local, &["init"]);

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}/redirect");

    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let response = "HTTP/1.1 302 Found\r\nLocation: /other\r\nContent-Length: 0\r\n\r\n";
        stream.write_all(response.as_bytes()).unwrap();
    });

    let out = sb.run_in(&local, &["merge", &url]);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("HTTP 302"), "stderr: {}", out.stderr);

    handle.join().unwrap();
}

// ── Terminal presentation (§7.11) ────────────────────────────────

fn s(code: u8, text: &str) -> String {
    format!("\x1b[{code}m{text}\x1b[0m")
}

#[test]
fn terminal_version_is_bold() {
    let sb = Sandbox::new();
    let out = sb.run_color_in(sb.path(), &["--version"]);
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, format!("{}\n", s(1, "snap 1.0.0")));
    assert_eq!(out.stderr, "");
}

#[test]
fn terminal_init_success_line() {
    let sb = Sandbox::new();
    let out = sb.run_color_in(sb.path(), &["init", "repo"]);
    assert_eq!(out.exit_code, 0);
    assert_eq!(
        out.stdout,
        format!(
            "{} {} {}\n",
            s(32, "✓"),
            s(1, "Initialized repository"),
            s(36, "()")
        )
    );
    assert_eq!(out.stderr, "");
}

#[test]
fn terminal_commit_success_line() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);
    sb.run_in(&repo, &["config", "contributor.id", "a@x"]);
    std::fs::write(repo.join("f.txt"), "hello\n").unwrap();

    let out = sb.run_color_in(&repo, &["commit", "first"]);
    assert_eq!(out.exit_code, 0);
    assert_eq!(
        out.stdout,
        format!(
            "{} {} {}\n",
            s(32, "✓"),
            s(1, "Committed"),
            s(36, "(a@x->1)")
        )
    );
}

#[test]
fn terminal_status_clean() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);

    let out = sb.run_color_in(&repo, &["status"]);
    assert_eq!(out.exit_code, 0);
    assert_eq!(
        out.stdout,
        format!(
            "{}  {}\n\n  {} Working tree clean\n",
            s(1, "Snap status"),
            s(36, "()"),
            s(32, "✓")
        )
    );
}

#[test]
fn terminal_status_dirty() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);
    sb.run_in(&repo, &["config", "contributor.id", "a@x"]);
    std::fs::write(repo.join("f.txt"), "hello\n").unwrap();

    let out = sb.run_color_in(&repo, &["status"]);
    assert_eq!(out.exit_code, 0);
    assert_eq!(
        out.stdout,
        format!(
            "{}  {}\n\n  {} f.txt {}\n",
            s(1, "Snap status"),
            s(36, "()"),
            s(32, "+"),
            s(2, "(added)")
        )
    );
}

#[test]
fn terminal_log_single_entry() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);
    sb.run_in(&repo, &["config", "contributor.id", "a@x"]);
    std::fs::write(repo.join("f.txt"), "hello\n").unwrap();
    sb.run_in(&repo, &["commit", "first"]);

    let out = sb.run_color_in(&repo, &["log"]);
    assert_eq!(out.exit_code, 0);
    assert_eq!(
        out.stdout,
        format!(
            "{} {}\n  {} {} {}\n",
            s(36, "●"),
            s(1, "first"),
            s(36, "(a@x->1)"),
            s(2, "by"),
            s(35, "a@x")
        )
    );
}

#[test]
fn terminal_log_multiple_entries_separated() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);
    sb.run_in(&repo, &["config", "contributor.id", "a@x"]);
    std::fs::write(repo.join("f.txt"), "v1\n").unwrap();
    sb.run_in(&repo, &["commit", "first"]);
    std::fs::write(repo.join("f.txt"), "v2\n").unwrap();
    sb.run_in(&repo, &["commit", "second"]);

    let out = sb.run_color_in(&repo, &["log"]);
    assert_eq!(out.exit_code, 0);
    let expected = format!(
        "{} {}\n  {} {} {}\n\n{} {}\n  {} {} {}\n",
        s(36, "●"),
        s(1, "second"),
        s(36, "(a@x->2)"),
        s(2, "by"),
        s(35, "a@x"),
        s(36, "●"),
        s(1, "first"),
        s(36, "(a@x->1)"),
        s(2, "by"),
        s(35, "a@x"),
    );
    assert_eq!(out.stdout, expected);
}

#[test]
fn terminal_diff_styled_lines() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);
    sb.run_in(&repo, &["config", "contributor.id", "a@x"]);
    std::fs::write(repo.join("f.txt"), "context\nold\n").unwrap();
    sb.run_in(&repo, &["commit", "first"]);
    std::fs::write(repo.join("f.txt"), "context\nnew\n").unwrap();

    let out = sb.run_color_in(&repo, &["diff"]);
    assert_eq!(out.exit_code, 0);
    let expected = format!(
        "{}\n{}\n{}\n context\n{}\n{}\n",
        s(1, "--- a/f.txt"),
        s(1, "+++ b/f.txt"),
        s(36, "@@ -1,2 +1,2 @@"),
        s(31, "-old"),
        s(32, "+new"),
    );
    assert_eq!(out.stdout, expected);
}

#[test]
fn terminal_diff_binary_and_no_newline() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);
    sb.run_in(&repo, &["config", "contributor.id", "a@x"]);
    std::fs::write(repo.join("f.bin"), [0x00, 0xFF]).unwrap();
    std::fs::write(repo.join("tail.txt"), "tail").unwrap();

    let out = sb.run_color_in(&repo, &["diff"]);
    assert_eq!(out.exit_code, 0);
    let expected = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n",
        s(33, "Binary files /dev/null and b/f.bin differ"),
        s(1, "--- /dev/null"),
        s(1, "+++ b/tail.txt"),
        s(36, "@@ -1,0 +1,1 @@"),
        s(32, "+tail"),
        s(2, "\\ No newline at end of file"),
    );
    assert_eq!(out.stdout, expected);
}

#[test]
fn terminal_error_is_styled() {
    let sb = Sandbox::new();
    let out = sb.run_color_in(sb.path(), &["unknown"]);
    assert_eq!(out.exit_code, 1);
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        format!("{}\n", s(31, "✗ snap: invalid command or arguments"))
    );
}

#[test]
fn terminal_merge_warning_is_styled() {
    let sb = Sandbox::new();
    let left = sb.path().join("left");
    let right = sb.path().join("right");
    std::fs::create_dir(&left).unwrap();
    std::fs::create_dir(&right).unwrap();
    sb.run_in(&left, &["init"]);
    sb.run_in(&right, &["init"]);
    sb.run_in(&left, &["config", "contributor.id", "a@x"]);
    sb.run_in(&right, &["config", "contributor.id", "b@x"]);
    std::fs::write(left.join("same"), "left\n").unwrap();
    std::fs::write(right.join("same"), "right\n").unwrap();
    sb.run_in(&left, &["commit", "left"]);
    sb.run_in(&right, &["commit", "right"]);

    let out = sb.run_color_in(&left, &["merge", right.to_str().unwrap()]);
    assert_eq!(out.exit_code, 0);
    assert_eq!(
        out.stderr,
        format!(
            "{} {}\n",
            s(33, "⚠"),
            s(33, "auto-resolved same: later-create-wins")
        )
    );
    assert_eq!(
        out.stdout,
        format!(
            "{} {} {}\n",
            s(32, "✓"),
            s(1, "Merged"),
            s(36, "(a@x->1,b@x->1)")
        )
    );
}

#[test]
fn terminal_revert_success_line() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);
    sb.run_in(&repo, &["config", "contributor.id", "a@x"]);
    std::fs::write(repo.join("f.txt"), "hello\n").unwrap();
    sb.run_in(&repo, &["commit", "first"]);
    std::fs::write(repo.join("f.txt"), "changed\n").unwrap();
    sb.run_in(&repo, &["commit", "second"]);

    let out = sb.run_color_in(&repo, &["revert", "(a@x->1)"]);
    assert_eq!(out.exit_code, 0);
    assert_eq!(
        out.stdout,
        format!(
            "{} {} {}\n",
            s(32, "✓"),
            s(1, "Reverted"),
            s(36, "(a@x->3)")
        )
    );
}

// ── SNAP_COLOR / NO_COLOR precedence ─────────────────────────────

#[test]
fn snap_color_never_produces_plain() {
    let sb = Sandbox::new();
    let repo = sb.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.run_in(&repo, &["init"]);
    sb.run_in(&repo, &["config", "contributor.id", "a@x"]);
    std::fs::write(repo.join("f.txt"), "hello\n").unwrap();
    sb.run_in(&repo, &["commit", "first"]);

    let out = sb.run_env_in(&repo, &["status"], &[("SNAP_COLOR", "never")]);
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, "version (a@x->1)\n");
}

#[test]
fn snap_color_auto_with_no_color_produces_plain() {
    let sb = Sandbox::new();
    let out = sb.run_env_in(
        sb.path(),
        &["--version"],
        &[("SNAP_COLOR", "auto"), ("NO_COLOR", "1")],
    );
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, "snap 1.0.0\n");
}

#[test]
fn snap_color_auto_no_color_empty_produces_plain() {
    let sb = Sandbox::new();
    let out = sb.run_env_in(
        sb.path(),
        &["--version"],
        &[("SNAP_COLOR", "auto"), ("NO_COLOR", "")],
    );
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, "snap 1.0.0\n");
}

#[test]
fn snap_color_unset_no_color_unset_piped_is_plain() {
    let sb = Sandbox::new();
    let out = sb.run_with_env(sb.path(), &["--version"], &[]);
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, "snap 1.0.0\n");
}

#[test]
fn snap_color_always_overrides_no_color() {
    let sb = Sandbox::new();
    let out = sb.run_env_in(
        sb.path(),
        &["--version"],
        &[("SNAP_COLOR", "always"), ("NO_COLOR", "1")],
    );
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, format!("{}\n", s(1, "snap 1.0.0")));
}

#[test]
fn snap_color_invalid_value_errors() {
    let sb = Sandbox::new();
    let out = sb.run_env_in(sb.path(), &["--version"], &[("SNAP_COLOR", "sometimes")]);
    assert_eq!(out.exit_code, 1);
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        "snap: SNAP_COLOR must be auto, always, or never\n"
    );
}

#[test]
fn snap_color_invalid_error_is_always_plain() {
    let sb = Sandbox::new();
    let out = sb.run_env_in(sb.path(), &["--version"], &[("SNAP_COLOR", "maybe")]);
    assert_eq!(out.exit_code, 1);
    assert!(!out.stderr.contains("\x1b["));
}
