use std::path::{Path, PathBuf};

use snap::config;
use snap::filesystem::{self, Tree};
use snap::replay;
use snap::repository::{self, Change, Patch, Repository};
use snap::text;
use snap::version::ContributorId;
use snap::writer::Writer;

use super::SnapError;

#[expect(clippy::unnecessary_wraps, reason = "consistent command signature")]
pub fn version<O: std::io::Write, E: std::io::Write>(
    writer: &mut Writer<O, E>,
) -> Result<(), SnapError> {
    writer.stdout(&format!("snap {}\n", env!("CARGO_PKG_VERSION")));
    Ok(())
}

pub fn init<O: std::io::Write, E: std::io::Write>(
    writer: &mut Writer<O, E>,
    path: Option<&str>,
) -> Result<(), SnapError> {
    let cwd = std::env::current_dir().map_err(|e| SnapError::Internal(e.to_string()))?;
    let target = match path {
        Some(p) => cwd.join(p),
        None => cwd,
    };

    if target.join(".snap").exists() {
        return Err(SnapError::Expected("repository already exists".to_owned()));
    }

    if let Some(existing) = find_repo_above(&target) {
        if existing != target {
            return Err(SnapError::Expected(
                "cannot initialize inside repository".to_owned(),
            ));
        }
    }

    std::fs::create_dir_all(&target).map_err(|e| SnapError::Internal(e.to_string()))?;
    let snap_dir = target.join(".snap");
    std::fs::create_dir_all(&snap_dir).map_err(|e| SnapError::Internal(e.to_string()))?;

    let repo = repository::Repository::empty();
    let json = repository::serialize(&repo);
    std::fs::write(snap_dir.join("repository.json"), json)
        .map_err(|e| SnapError::Internal(e.to_string()))?;

    writer.stdout("()\n");
    Ok(())
}

pub fn config<O: std::io::Write, E: std::io::Write>(
    _writer: &mut Writer<O, E>,
    global: bool,
    id_str: &str,
) -> Result<(), SnapError> {
    let id = ContributorId::new(id_str)
        .map_err(|_| SnapError::Expected(format!("invalid contributor id: {id_str}")))?;

    if global {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| SnapError::Expected("HOME is not set".to_owned()))?;
        let path = Path::new(&home).join(".snapconfig.json");
        config::write_config_file(&path, &id).map_err(|e| SnapError::Internal(e.to_string()))?;
    } else {
        let repo_root = find_repo_from_cwd()
            .ok_or_else(|| SnapError::Expected("not a Snap repository".to_owned()))?;
        let path = repo_root.join(".snap/config.json");
        config::write_config_file(&path, &id).map_err(|e| SnapError::Internal(e.to_string()))?;
    }

    Ok(())
}

pub fn status<O: std::io::Write, E: std::io::Write>(
    writer: &mut Writer<O, E>,
) -> Result<(), SnapError> {
    let repo_root = require_repo()?;
    let repo = load_repo(&repo_root)?;
    let current_tree = replay_to_frontier(&repo)?;
    let working_tree = filesystem::scan_working_tree(&repo_root)
        .map_err(|e| SnapError::Expected(e.to_string()))?;

    writer.stdout(&format!("version {}\n", repo.frontier));

    let mut changes = compute_working_changes(&current_tree, &working_tree);
    changes.sort_by(|a, b| a.1.cmp(&b.1));

    for (code, path) in &changes {
        writer.stdout(&format!("{code} {path}\n"));
    }

    Ok(())
}

pub fn log<O: std::io::Write, E: std::io::Write>(
    writer: &mut Writer<O, E>,
) -> Result<(), SnapError> {
    let repo_root = require_repo()?;
    let repo = load_repo(&repo_root)?;

    let order =
        replay::canonical_order(&repo.patches).map_err(|e| SnapError::Internal(e.to_string()))?;

    for &idx in order.iter().rev() {
        let patch = &repo.patches[idx];
        let version = patch.result_version();
        let escaped = escape_log_message(&patch.message);
        writer.stdout(&format!(
            "{version}\t{}\t{escaped}\n",
            patch.author.as_str()
        ));
    }

    Ok(())
}

pub fn diff_working<O: std::io::Write, E: std::io::Write>(
    writer: &mut Writer<O, E>,
) -> Result<(), SnapError> {
    let repo_root = require_repo()?;
    let repo = load_repo(&repo_root)?;
    let current_tree = replay_to_frontier(&repo)?;
    let working_tree = filesystem::scan_working_tree(&repo_root)
        .map_err(|e| SnapError::Expected(e.to_string()))?;

    let mut paths: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for path in current_tree.keys() {
        paths.insert(path);
    }
    for path in working_tree.keys() {
        paths.insert(path);
    }

    for path in paths {
        let old = current_tree.get(path);
        let new = working_tree.get(path);

        match (old, new) {
            (None, Some(new_content)) => {
                format_diff_addition(writer, path, new_content);
            }
            (Some(old_content), None) => {
                format_diff_deletion(writer, path, old_content);
            }
            (Some(old_content), Some(new_content)) if old_content != new_content => {
                format_diff_modification(writer, path, old_content, new_content);
            }
            _ => {}
        }
    }

    Ok(())
}

pub fn commit<O: std::io::Write, E: std::io::Write>(
    writer: &mut Writer<O, E>,
    message: &str,
) -> Result<(), SnapError> {
    let repo_root = require_repo()?;
    let contributor = resolve_contributor(&repo_root)?;
    let repo = load_repo(&repo_root)?;

    repository::validate_commit_message(message).map_err(|e| SnapError::Expected(e.to_string()))?;

    let current_tree = replay_to_frontier(&repo)?;
    let working_tree = filesystem::scan_working_tree(&repo_root)
        .map_err(|e| SnapError::Expected(e.to_string()))?;

    if filesystem::is_clean(&working_tree, &current_tree) {
        return Err(SnapError::Expected("working tree is clean".to_owned()));
    }

    let revision = repo.frontier.get(&contributor) + 1;

    if revision > 9_007_199_254_740_991 {
        return Err(SnapError::Expected("revision overflow".to_owned()));
    }

    for patch in &repo.patches {
        if patch.author == contributor && patch.revision == revision {
            return Err(SnapError::Expected(format!(
                "patch collision: {} revision {revision}",
                contributor.as_str()
            )));
        }
    }

    let changes = build_changes(&current_tree, &working_tree)?;

    let patch = Patch {
        author: contributor,
        revision,
        base: repo.frontier.clone(),
        message: message.to_owned(),
        changes,
    };

    let new_version = patch.result_version();

    let mut new_repo = repo;
    new_repo.frontier = new_version.clone();
    new_repo.patches.push(patch);

    atomic_write_repo(&repo_root, &new_repo)?;

    writer.stdout(&format!("{new_version}\n"));
    Ok(())
}

pub fn require_repo_stub(_cmd: &str) -> Result<(), SnapError> {
    require_repo()?;
    Err(SnapError::Expected("not implemented".to_owned()))
}

// ── Helpers ────────────────────────────────────────────────────────

fn require_repo() -> Result<PathBuf, SnapError> {
    find_repo_from_cwd().ok_or_else(|| SnapError::Expected("not a Snap repository".to_owned()))
}

fn find_repo_from_cwd() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    find_repo_above(&cwd)
}

pub fn find_repo_above(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        if current.join(".snap").is_dir() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn load_repo(repo_root: &Path) -> Result<Repository, SnapError> {
    let json_path = repo_root.join(".snap/repository.json");
    let json =
        std::fs::read_to_string(&json_path).map_err(|e| SnapError::Internal(e.to_string()))?;
    repository::parse(&json).map_err(|e| SnapError::Internal(e.to_string()))
}

fn replay_to_frontier(repo: &Repository) -> Result<Tree, SnapError> {
    let result = replay::replay(&repo.patches, &repo.frontier)
        .map_err(|e| SnapError::Internal(e.to_string()))?;
    Ok(result.tree)
}

fn resolve_contributor(repo_root: &Path) -> Result<ContributorId, SnapError> {
    config::resolve_contributor(Some(repo_root))
        .map_err(|e| SnapError::Expected(e.to_string()))?
        .ok_or_else(|| {
            SnapError::Expected(
                "contributor.id is required; configure it locally or globally".to_owned(),
            )
        })
}

fn compute_working_changes(current: &Tree, working: &Tree) -> Vec<(char, String)> {
    let mut changes = Vec::new();

    for (path, content) in working {
        match current.get(path) {
            None => changes.push(('A', path.clone())),
            Some(old) if old != content => changes.push(('M', path.clone())),
            _ => {}
        }
    }

    for path in current.keys() {
        if !working.contains_key(path) {
            changes.push(('D', path.clone()));
        }
    }

    changes
}

fn build_changes(current: &Tree, working: &Tree) -> Result<Vec<Change>, SnapError> {
    let mut paths: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for path in current.keys() {
        paths.insert(path);
    }
    for path in working.keys() {
        paths.insert(path);
    }

    let mut changes = Vec::new();

    for path in paths {
        let old = current.get(path);
        let new = working.get(path);

        match (old, new) {
            (None, Some(new_content)) => {
                changes.push(make_create_change(path, new_content));
            }
            (Some(_), None) => {
                changes.push(Change::Delete {
                    path: path.to_owned(),
                });
            }
            (Some(old_content), Some(new_content)) if old_content != new_content => {
                changes.push(make_modify_change(path, old_content, new_content));
            }
            _ => {}
        }
    }

    if changes.is_empty() {
        return Err(SnapError::Internal(
            "no changes found despite dirty tree".to_owned(),
        ));
    }

    Ok(changes)
}

fn make_create_change(path: &str, content: &[u8]) -> Change {
    if text::is_text(content) {
        let text_content = std::str::from_utf8(content).expect("is_text guarantees valid UTF-8");
        let new_tokens = text::tokenize(text_content);
        let old_tokens: Vec<&str> = Vec::new();
        let edit = text::diff(&old_tokens, &new_tokens);
        Change::Text {
            path: path.to_owned(),
            edit,
        }
    } else {
        Change::Put {
            path: path.to_owned(),
            content: content.to_vec(),
        }
    }
}

fn make_modify_change(path: &str, old_content: &[u8], new_content: &[u8]) -> Change {
    let old_is_text = text::is_text(old_content);
    let new_is_text = text::is_text(new_content);

    if new_is_text && old_is_text {
        let old_str = std::str::from_utf8(old_content).expect("is_text guarantees valid UTF-8");
        let new_str = std::str::from_utf8(new_content).expect("is_text guarantees valid UTF-8");
        let old_tokens = text::tokenize(old_str);
        let new_tokens = text::tokenize(new_str);
        let edit = text::diff(&old_tokens, &new_tokens);
        Change::Text {
            path: path.to_owned(),
            edit,
        }
    } else {
        Change::Put {
            path: path.to_owned(),
            content: new_content.to_vec(),
        }
    }
}

fn escape_log_message(message: &str) -> String {
    message
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
}

fn atomic_write_repo(repo_root: &Path, repo: &Repository) -> Result<(), SnapError> {
    let json = repository::serialize(repo);
    let snap_dir = repo_root.join(".snap");
    let tmp_path = snap_dir.join("repository.json.tmp");
    let final_path = snap_dir.join("repository.json");

    std::fs::write(&tmp_path, &json).map_err(|e| SnapError::Internal(e.to_string()))?;
    std::fs::rename(&tmp_path, &final_path).map_err(|e| SnapError::Internal(e.to_string()))?;

    Ok(())
}

// ── Diff formatting ────────────────────────────────────────────────

fn format_diff_addition<O: std::io::Write, E: std::io::Write>(
    writer: &mut Writer<O, E>,
    path: &str,
    content: &[u8],
) {
    if text::is_text(content) {
        let s = std::str::from_utf8(content).expect("is_text guarantees valid UTF-8");
        let tokens = text::tokenize(s);
        writer.stdout(&format!("--- /dev/null\n+++ b/{path}\n"));
        writer.stdout(&format!("@@ -1,0 +1,{} @@\n", tokens.len()));
        format_diff_tokens_inserted(writer, &tokens);
    } else {
        writer.stdout(&format!("Binary files /dev/null and b/{path} differ\n"));
    }
}

fn format_diff_deletion<O: std::io::Write, E: std::io::Write>(
    writer: &mut Writer<O, E>,
    path: &str,
    content: &[u8],
) {
    if text::is_text(content) {
        let s = std::str::from_utf8(content).expect("is_text guarantees valid UTF-8");
        let tokens = text::tokenize(s);
        writer.stdout(&format!("--- a/{path}\n+++ /dev/null\n"));
        writer.stdout(&format!("@@ -1,{} +1,0 @@\n", tokens.len()));
        for token in &tokens {
            write_diff_token(writer, '-', token);
        }
    } else {
        writer.stdout(&format!("Binary files a/{path} and /dev/null differ\n"));
    }
}

fn format_diff_modification<O: std::io::Write, E: std::io::Write>(
    writer: &mut Writer<O, E>,
    path: &str,
    old_content: &[u8],
    new_content: &[u8],
) {
    let old_is_text = text::is_text(old_content);
    let new_is_text = text::is_text(new_content);

    if old_is_text && new_is_text {
        let old_str = std::str::from_utf8(old_content).expect("is_text guarantees valid UTF-8");
        let new_str = std::str::from_utf8(new_content).expect("is_text guarantees valid UTF-8");
        let old_tokens = text::tokenize(old_str);
        let new_tokens = text::tokenize(new_str);
        let edit = text::diff(&old_tokens, &new_tokens);

        writer.stdout(&format!("--- a/{path}\n+++ b/{path}\n"));
        writer.stdout(&format!(
            "@@ -1,{} +1,{} @@\n",
            old_tokens.len(),
            new_tokens.len()
        ));
        format_unified_edit(writer, &edit, &old_tokens);
    } else {
        writer.stdout(&format!("Binary files a/{path} and b/{path} differ\n"));
    }
}

fn format_diff_tokens_inserted<O: std::io::Write, E: std::io::Write>(
    writer: &mut Writer<O, E>,
    tokens: &[&str],
) {
    for token in tokens {
        write_diff_token(writer, '+', token);
    }
}

fn format_unified_edit<O: std::io::Write, E: std::io::Write>(
    writer: &mut Writer<O, E>,
    edit: &text::EditScript,
    old_tokens: &[&str],
) {
    let mut old_pos = 0;
    for op in edit.ops() {
        match op {
            text::EditOp::Retain(n) => {
                for token in &old_tokens[old_pos..old_pos + n] {
                    write_diff_token(writer, ' ', token);
                }
                old_pos += n;
            }
            text::EditOp::Delete(n) => {
                for token in &old_tokens[old_pos..old_pos + n] {
                    write_diff_token(writer, '-', token);
                }
                old_pos += n;
            }
            text::EditOp::Insert(tokens) => {
                for token in tokens {
                    write_diff_token(writer, '+', token);
                }
            }
        }
    }
}

fn write_diff_token<O: std::io::Write, E: std::io::Write>(
    writer: &mut Writer<O, E>,
    prefix: char,
    token: &str,
) {
    writer.stdout(&format!("{prefix}{token}"));
    if !token.ends_with('\n') {
        writer.stdout("\n\\ No newline at end of file\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree_from(entries: &[(&str, &[u8])]) -> Tree {
        entries
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.to_vec()))
            .collect()
    }

    // ── escape_log_message ────────────────────────────────────

    #[test]
    fn escape_plain_message() {
        assert_eq!(escape_log_message("hello"), "hello");
    }

    #[test]
    fn escape_backslash_then_tab_then_newline() {
        assert_eq!(
            escape_log_message("first\tline\nsecond\\tail"),
            "first\\tline\\nsecond\\\\tail"
        );
    }

    #[test]
    fn escape_order_matters() {
        assert_eq!(escape_log_message("a\\b\tc\nd"), "a\\\\b\\tc\\nd");
    }

    // ── compute_working_changes ───────────────────────────────

    #[test]
    fn detects_additions() {
        let current = tree_from(&[]);
        let working = tree_from(&[("a.txt", b"hello\n")]);
        let changes = compute_working_changes(&current, &working);
        assert_eq!(changes, vec![('A', "a.txt".to_owned())]);
    }

    #[test]
    fn detects_modifications() {
        let current = tree_from(&[("a.txt", b"old\n")]);
        let working = tree_from(&[("a.txt", b"new\n")]);
        let changes = compute_working_changes(&current, &working);
        assert_eq!(changes, vec![('M', "a.txt".to_owned())]);
    }

    #[test]
    fn detects_deletions() {
        let current = tree_from(&[("a.txt", b"hello\n")]);
        let working = tree_from(&[]);
        let changes = compute_working_changes(&current, &working);
        assert_eq!(changes, vec![('D', "a.txt".to_owned())]);
    }

    #[test]
    fn no_changes_for_identical_trees() {
        let tree = tree_from(&[("a.txt", b"same\n")]);
        let changes = compute_working_changes(&tree, &tree);
        assert!(changes.is_empty());
    }

    // ── build_changes ─────────────────────────────────────────

    #[test]
    fn text_create_uses_text_change() {
        let current = tree_from(&[]);
        let working = tree_from(&[("f.txt", b"hello\n")]);
        let changes = build_changes(&current, &working).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], Change::Text { path, .. } if path == "f.txt"));
    }

    #[test]
    fn binary_create_uses_put_change() {
        let current = tree_from(&[]);
        let working = tree_from(&[("f.bin", &[0x00, 0xFF])]);
        let changes = build_changes(&current, &working).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], Change::Put { path, .. } if path == "f.bin"));
    }

    #[test]
    fn empty_file_create_uses_text_change() {
        let current = tree_from(&[]);
        let working = tree_from(&[("empty", b"")]);
        let changes = build_changes(&current, &working).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(
            matches!(&changes[0], Change::Text { path, edit } if path == "empty" && edit.ops().is_empty())
        );
    }

    #[test]
    fn text_modify_uses_text_change() {
        let current = tree_from(&[("f.txt", b"old\n")]);
        let working = tree_from(&[("f.txt", b"new\n")]);
        let changes = build_changes(&current, &working).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], Change::Text { path, .. } if path == "f.txt"));
    }

    #[test]
    fn binary_to_text_uses_put() {
        let current = tree_from(&[("f", &[0x00, 0xFF])]);
        let working = tree_from(&[("f", b"text\n")]);
        let changes = build_changes(&current, &working).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], Change::Put { .. }));
    }

    #[test]
    fn text_to_binary_uses_put() {
        let current = tree_from(&[("f", b"text\n")]);
        let working = tree_from(&[("f", &[0x00, 0xFF])]);
        let changes = build_changes(&current, &working).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], Change::Put { .. }));
    }

    #[test]
    fn deletion_uses_delete_change() {
        let current = tree_from(&[("f.txt", b"content\n")]);
        let working = tree_from(&[]);
        let changes = build_changes(&current, &working).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], Change::Delete { path } if path == "f.txt"));
    }

    #[test]
    fn changes_sorted_by_path() {
        let current = tree_from(&[]);
        let working = tree_from(&[("z.txt", b"z\n"), ("a.txt", b"a\n"), ("m.txt", b"m\n")]);
        let changes = build_changes(&current, &working).unwrap();
        let paths: Vec<&str> = changes.iter().map(Change::path).collect();
        assert_eq!(paths, vec!["a.txt", "m.txt", "z.txt"]);
    }

    // ── diff formatting ───────────────────────────────────────

    #[test]
    fn diff_text_addition() {
        let mut out = Vec::new();
        let err = Vec::new();
        let mut w = Writer::new(&mut out, err);
        format_diff_addition(&mut w, "f.txt", b"line\n");
        let output = String::from_utf8(out).unwrap();
        assert_eq!(
            output,
            "--- /dev/null\n+++ b/f.txt\n@@ -1,0 +1,1 @@\n+line\n"
        );
    }

    #[test]
    fn diff_binary_addition() {
        let mut out = Vec::new();
        let err = Vec::new();
        let mut w = Writer::new(&mut out, err);
        format_diff_addition(&mut w, "f.bin", &[0x00, 0xFF]);
        let output = String::from_utf8(out).unwrap();
        assert_eq!(output, "Binary files /dev/null and b/f.bin differ\n");
    }

    #[test]
    fn diff_empty_file_addition() {
        let mut out = Vec::new();
        let err = Vec::new();
        let mut w = Writer::new(&mut out, err);
        format_diff_addition(&mut w, "empty", b"");
        let output = String::from_utf8(out).unwrap();
        assert_eq!(output, "--- /dev/null\n+++ b/empty\n@@ -1,0 +1,0 @@\n");
    }

    #[test]
    fn diff_text_deletion() {
        let mut out = Vec::new();
        let err = Vec::new();
        let mut w = Writer::new(&mut out, err);
        format_diff_deletion(&mut w, "f.txt", b"line\n");
        let output = String::from_utf8(out).unwrap();
        assert_eq!(
            output,
            "--- a/f.txt\n+++ /dev/null\n@@ -1,1 +1,0 @@\n-line\n"
        );
    }

    #[test]
    fn diff_binary_deletion() {
        let mut out = Vec::new();
        let err = Vec::new();
        let mut w = Writer::new(&mut out, err);
        format_diff_deletion(&mut w, "f.bin", &[0x00, 0xFF]);
        let output = String::from_utf8(out).unwrap();
        assert_eq!(output, "Binary files a/f.bin and /dev/null differ\n");
    }

    #[test]
    fn diff_no_trailing_newline() {
        let mut out = Vec::new();
        let err = Vec::new();
        let mut w = Writer::new(&mut out, err);
        format_diff_addition(&mut w, "f.txt", b"no newline");
        let output = String::from_utf8(out).unwrap();
        assert_eq!(
            output,
            "--- /dev/null\n+++ b/f.txt\n@@ -1,0 +1,1 @@\n+no newline\n\\ No newline at end of file\n"
        );
    }
}
