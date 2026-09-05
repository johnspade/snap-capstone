use std::path::{Path, PathBuf};

use snap::config;
use snap::filesystem::{self, Tree};
use snap::http;
use snap::replay;
use snap::repository::{self, Change, Patch, Repository};
use snap::text;
use snap::version::{ContributorId, Version};
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

    format_tree_diff(writer, &current_tree, &working_tree);
    Ok(())
}

pub fn diff_versions<O: std::io::Write, E: std::io::Write>(
    writer: &mut Writer<O, E>,
    old_str: &str,
    new_str: &str,
) -> Result<(), SnapError> {
    let repo_root = require_repo()?;
    let repo = load_repo(&repo_root)?;

    let old_version = parse_version(old_str)?;
    let new_version = parse_version(new_str)?;
    validate_version_known(&old_version, &repo)?;
    validate_version_known(&new_version, &repo)?;

    let old_tree = replay_to_version(&repo, &old_version)?;
    let new_tree = replay_to_version(&repo, &new_version)?;

    format_tree_diff(writer, &old_tree, &new_tree);
    Ok(())
}

pub fn diff_cross_repo<O: std::io::Write, E: std::io::Write>(
    writer: &mut Writer<O, E>,
    old_str: &str,
    new_str: &str,
    repo_path: &str,
) -> Result<(), SnapError> {
    let repo_root = require_repo()?;
    let local_repo = load_repo(&repo_root)?;

    let old_version = parse_version(old_str)?;
    let new_version = parse_version(new_str)?;
    validate_version_known(&old_version, &local_repo)?;

    let remote_repo = load_remote_repo(repo_path)?;
    validate_version_known(&new_version, &remote_repo)?;
    validate_shared_dots(&local_repo, &remote_repo)?;

    let old_tree = replay_to_version(&local_repo, &old_version)?;
    let new_tree = replay_to_version(&remote_repo, &new_version)?;

    format_tree_diff(writer, &old_tree, &new_tree);
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

pub fn revert<O: std::io::Write, E: std::io::Write>(
    writer: &mut Writer<O, E>,
    version_str: &str,
) -> Result<(), SnapError> {
    let repo_root = require_repo()?;
    let target_version: Version = version_str
        .parse()
        .map_err(|_| SnapError::Expected(format!("invalid version: {version_str}")))?;
    let repo = load_repo(&repo_root)?;

    validate_version_known(&target_version, &repo)?;

    let target_result = replay::replay(&repo.patches, &target_version)
        .map_err(|e| SnapError::Internal(e.to_string()))?;
    let target_tree = target_result.tree;

    let contributor = resolve_contributor(&repo_root)?;

    let current_tree = replay_to_frontier(&repo)?;

    let working_tree = filesystem::scan_working_tree(&repo_root)
        .map_err(|e| SnapError::Expected(e.to_string()))?;
    if !filesystem::is_clean(&working_tree, &current_tree) {
        return Err(SnapError::Expected("working tree is dirty".to_owned()));
    }

    if current_tree == target_tree {
        return Err(SnapError::Expected(
            "target tree is already current".to_owned(),
        ));
    }

    let changes = build_changes(&current_tree, &target_tree)?;

    let message = format!("revert to {target_version}");
    let revision = repo.frontier.get(&contributor) + 1;

    let patch = Patch {
        author: contributor,
        revision,
        base: repo.frontier.clone(),
        message,
        changes,
    };

    let new_version = patch.result_version();

    let mut new_repo = repo;
    new_repo.frontier = new_version.clone();
    new_repo.patches.push(patch);

    filesystem::materialize(&repo_root, &target_tree)
        .map_err(|e| SnapError::Internal(e.to_string()))?;
    atomic_write_repo(&repo_root, &new_repo)?;

    writer.stdout(&format!("{new_version}\n"));
    Ok(())
}

pub fn merge<O: std::io::Write, E: std::io::Write>(
    writer: &mut Writer<O, E>,
    remote_operand: &str,
) -> Result<(), SnapError> {
    let repo_root = require_repo()?;
    let local_repo = load_repo(&repo_root)?;

    let working_tree = filesystem::scan_working_tree(&repo_root)
        .map_err(|e| SnapError::Expected(e.to_string()))?;
    let local_tree = replay_to_frontier(&local_repo)?;
    if !filesystem::is_clean(&working_tree, &local_tree) {
        return Err(SnapError::Expected("working tree is dirty".to_owned()));
    }

    let remote_repo = load_remote_repo(remote_operand)?;

    let merged = union_repositories(&local_repo, &remote_repo)?;

    let local_result = replay::replay(&local_repo.patches, &local_repo.frontier)
        .map_err(|e| SnapError::Internal(e.to_string()))?;
    let merged_result = replay::replay(&merged.patches, &merged.frontier)
        .map_err(|e| SnapError::Internal(e.to_string()))?;

    let new_warnings: Vec<&(String, String)> = merged_result
        .warnings
        .iter()
        .filter(|w| !local_result.warnings.contains(w))
        .collect();

    for (path, reason) in &new_warnings {
        writer.stderr(&format!("warning: auto-resolved {path}: {reason}\n"));
    }

    filesystem::materialize(&repo_root, &merged_result.tree)
        .map_err(|e| SnapError::Internal(e.to_string()))?;
    atomic_write_repo(&repo_root, &merged)?;

    writer.stdout(&format!("{}\n", merged.frontier));
    Ok(())
}

fn union_repositories(local: &Repository, remote: &Repository) -> Result<Repository, SnapError> {
    let mut patches = local.patches.clone();

    for remote_patch in &remote.patches {
        let existing = patches
            .iter()
            .find(|p| p.author == remote_patch.author && p.revision == remote_patch.revision);
        match existing {
            Some(local_patch) => {
                if local_patch != remote_patch {
                    return Err(SnapError::Expected(format!(
                        "patch collision: {} revision {}",
                        remote_patch.author.as_str(),
                        remote_patch.revision
                    )));
                }
            }
            None => {
                patches.push(remote_patch.clone());
            }
        }
    }

    let frontier = local.frontier.join(&remote.frontier);

    Ok(Repository { frontier, patches })
}
pub fn serve<O: std::io::Write, E: std::io::Write>(
    writer: &mut Writer<O, E>,
    port_str: Option<&str>,
) -> Result<(), SnapError> {
    let repo_root = require_repo()?;
    let repo = load_repo(&repo_root).map_err(|e| match e {
        SnapError::Internal(msg) => SnapError::Expected(msg),
        SnapError::Expected(_) => e,
    })?;
    replay_to_frontier(&repo).map_err(|e| match e {
        SnapError::Internal(msg) => SnapError::Expected(msg),
        SnapError::Expected(_) => e,
    })?;

    let port: u16 = match port_str {
        None => 8765,
        Some(s) => s
            .parse()
            .map_err(|_| SnapError::Expected(format!("invalid port: {s}")))?,
    };

    http::serve(&repo, port, writer.stdout_mut()).map_err(SnapError::Expected)?;

    Ok(())
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

fn replay_to_version(repo: &Repository, version: &Version) -> Result<Tree, SnapError> {
    let result =
        replay::replay(&repo.patches, version).map_err(|e| SnapError::Internal(e.to_string()))?;
    Ok(result.tree)
}

fn parse_version(s: &str) -> Result<Version, SnapError> {
    s.parse::<Version>()
        .map_err(|_| SnapError::Expected(format!("invalid version: {s}")))
}

fn validate_version_known(version: &Version, repo: &Repository) -> Result<(), SnapError> {
    for (id, rev) in version.components() {
        for r in 1..=*rev {
            if !repo
                .patches
                .iter()
                .any(|p| p.author == *id && p.revision == r)
            {
                return Err(SnapError::Expected(format!("unknown version: {version}")));
            }
        }
    }
    Ok(())
}

fn load_remote_repo(operand: &str) -> Result<Repository, SnapError> {
    if http::is_http_url(operand) {
        http::fetch_remote_repository(operand).map_err(SnapError::Expected)
    } else {
        let cwd = std::env::current_dir().map_err(|e| SnapError::Internal(e.to_string()))?;
        let remote_root = cwd.join(operand);
        if !remote_root.join(".snap").is_dir() {
            return Err(SnapError::Expected(format!(
                "not a Snap repository: {operand}"
            )));
        }
        load_repo(&remote_root).map_err(|e| match e {
            SnapError::Internal(msg) => SnapError::Expected(msg),
            SnapError::Expected(_) => e,
        })
    }
}

fn validate_shared_dots(local: &Repository, remote: &Repository) -> Result<(), SnapError> {
    for lp in &local.patches {
        for rp in &remote.patches {
            if lp.author == rp.author && lp.revision == rp.revision && lp != rp {
                return Err(SnapError::Expected(format!(
                    "patch collision: {} revision {}",
                    lp.author.as_str(),
                    lp.revision
                )));
            }
        }
    }
    Ok(())
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

fn format_tree_diff<O: std::io::Write, E: std::io::Write>(
    writer: &mut Writer<O, E>,
    old_tree: &Tree,
    new_tree: &Tree,
) {
    let mut paths: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for path in old_tree.keys() {
        paths.insert(path);
    }
    for path in new_tree.keys() {
        paths.insert(path);
    }

    for path in paths {
        let old = old_tree.get(path);
        let new = new_tree.get(path);

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
}

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
    use snap::text::{EditOp, EditScript};
    use snap::version::ContributorId;

    fn tree_from(entries: &[(&str, &[u8])]) -> Tree {
        entries
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.to_vec()))
            .collect()
    }

    fn cid(s: &str) -> ContributorId {
        ContributorId::new(s).unwrap()
    }

    fn ver(s: &str) -> snap::version::Version {
        s.parse().unwrap()
    }

    fn make_patch(
        author: &str,
        revision: u64,
        base: &str,
        message: &str,
        changes: Vec<Change>,
    ) -> repository::Patch {
        repository::Patch {
            author: cid(author),
            revision,
            base: ver(base),
            message: message.to_owned(),
            changes,
        }
    }

    fn text_change(path: &str, ops: Vec<EditOp>) -> Change {
        Change::Text {
            path: path.to_owned(),
            edit: EditScript::new(ops).unwrap(),
        }
    }

    // ── union_repositories ────────────────────────────────────

    #[test]
    fn union_disjoint_patches() {
        let local = Repository {
            frontier: ver("(a@x->1)"),
            patches: vec![make_patch(
                "a@x",
                1,
                "()",
                "a1",
                vec![text_change(
                    "a.txt",
                    vec![EditOp::Insert(vec!["a\n".to_owned()])],
                )],
            )],
        };
        let remote = Repository {
            frontier: ver("(b@y->1)"),
            patches: vec![make_patch(
                "b@y",
                1,
                "()",
                "b1",
                vec![text_change(
                    "b.txt",
                    vec![EditOp::Insert(vec!["b\n".to_owned()])],
                )],
            )],
        };
        let merged = union_repositories(&local, &remote).unwrap();
        assert_eq!(merged.patches.len(), 2);
        assert_eq!(merged.frontier, ver("(a@x->1,b@y->1)"));
    }

    #[test]
    fn union_overlapping_identical_patches() {
        let patch = make_patch(
            "a@x",
            1,
            "()",
            "shared",
            vec![text_change(
                "f.txt",
                vec![EditOp::Insert(vec!["shared\n".to_owned()])],
            )],
        );
        let local = Repository {
            frontier: ver("(a@x->1)"),
            patches: vec![patch.clone()],
        };
        let remote = Repository {
            frontier: ver("(a@x->1)"),
            patches: vec![patch],
        };
        let merged = union_repositories(&local, &remote).unwrap();
        assert_eq!(merged.patches.len(), 1);
        assert_eq!(merged.frontier, ver("(a@x->1)"));
    }

    #[test]
    fn union_dot_collision_errors() {
        let local = Repository {
            frontier: ver("(a@x->1)"),
            patches: vec![make_patch(
                "a@x",
                1,
                "()",
                "local",
                vec![text_change(
                    "f.txt",
                    vec![EditOp::Insert(vec!["local\n".to_owned()])],
                )],
            )],
        };
        let remote = Repository {
            frontier: ver("(a@x->1)"),
            patches: vec![make_patch(
                "a@x",
                1,
                "()",
                "remote",
                vec![text_change(
                    "f.txt",
                    vec![EditOp::Insert(vec!["remote\n".to_owned()])],
                )],
            )],
        };
        let err = union_repositories(&local, &remote).unwrap_err();
        match err {
            SnapError::Expected(msg) => assert!(msg.contains("patch collision")),
            SnapError::Internal(_) => panic!("expected Expected error"),
        }
    }

    #[test]
    fn union_already_contained_is_idempotent() {
        let patch = make_patch(
            "a@x",
            1,
            "()",
            "a1",
            vec![text_change(
                "f.txt",
                vec![EditOp::Insert(vec!["hello\n".to_owned()])],
            )],
        );
        let local = Repository {
            frontier: ver("(a@x->1)"),
            patches: vec![patch.clone()],
        };
        let remote = Repository {
            frontier: ver("(a@x->1)"),
            patches: vec![patch],
        };
        let merged = union_repositories(&local, &remote).unwrap();
        assert_eq!(merged, local);
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

    // ── format_tree_diff ─────────────────────────────────────

    #[test]
    fn tree_diff_identical_trees_no_output() {
        let mut out = Vec::new();
        let err = Vec::new();
        let mut w = Writer::new(&mut out, err);
        let tree = tree_from(&[("a.txt", b"hello\n")]);
        format_tree_diff(&mut w, &tree, &tree);
        assert!(out.is_empty());
    }

    #[test]
    fn tree_diff_addition_deletion_modification() {
        let mut out = Vec::new();
        let err = Vec::new();
        let mut w = Writer::new(&mut out, err);
        let old_tree = tree_from(&[("b.txt", b"old\n"), ("c.txt", b"keep\n")]);
        let new_tree = tree_from(&[("a.txt", b"new\n"), ("b.txt", b"changed\n")]);
        format_tree_diff(&mut w, &old_tree, &new_tree);
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("--- /dev/null\n+++ b/a.txt\n"));
        assert!(output.contains("--- a/b.txt\n+++ b/b.txt\n"));
        assert!(output.contains("--- a/c.txt\n+++ /dev/null\n"));
    }

    #[test]
    fn tree_diff_paths_sorted() {
        let mut out = Vec::new();
        let err = Vec::new();
        let mut w = Writer::new(&mut out, err);
        let old = tree_from(&[]);
        let new = tree_from(&[("z.txt", b"z\n"), ("a.txt", b"a\n")]);
        format_tree_diff(&mut w, &old, &new);
        let output = String::from_utf8(out).unwrap();
        let a_pos = output.find("+++ b/a.txt").unwrap();
        let z_pos = output.find("+++ b/z.txt").unwrap();
        assert!(a_pos < z_pos);
    }

    #[test]
    fn tree_diff_binary_files() {
        let mut out = Vec::new();
        let err = Vec::new();
        let mut w = Writer::new(&mut out, err);
        let old = tree_from(&[("f.bin", &[0x00, 0x01])]);
        let new = tree_from(&[("f.bin", &[0xFF, 0xFE])]);
        format_tree_diff(&mut w, &old, &new);
        let output = String::from_utf8(out).unwrap();
        assert_eq!(output, "Binary files a/f.bin and b/f.bin differ\n");
    }

    #[test]
    fn tree_diff_empty_to_empty() {
        let mut out = Vec::new();
        let err = Vec::new();
        let mut w = Writer::new(&mut out, err);
        format_tree_diff(&mut w, &tree_from(&[]), &tree_from(&[]));
        assert!(out.is_empty());
    }

    // ── validate_version_known ───────────────────────────────

    #[test]
    fn validate_empty_version_always_known() {
        let repo = Repository::empty();
        assert!(validate_version_known(&Version::empty(), &repo).is_ok());
    }

    #[test]
    fn validate_version_known_with_matching_patches() {
        use snap::text::EditScript;
        let repo = Repository {
            frontier: "(a@x->1)".parse().unwrap(),
            patches: vec![Patch {
                author: ContributorId::new("a@x").unwrap(),
                revision: 1,
                base: Version::empty(),
                message: "first".to_owned(),
                changes: vec![Change::Text {
                    path: "f".to_owned(),
                    edit: EditScript::new(vec![]).unwrap(),
                }],
            }],
        };
        assert!(validate_version_known(&"(a@x->1)".parse().unwrap(), &repo).is_ok());
    }

    #[test]
    fn validate_version_unknown_missing_patch() {
        let repo = Repository::empty();
        let version: Version = "(a@x->1)".parse().unwrap();
        let err = validate_version_known(&version, &repo).unwrap_err();
        match err {
            SnapError::Expected(msg) => assert!(msg.contains("unknown version")),
            SnapError::Internal(_) => panic!("expected Expected error"),
        }
    }

    #[test]
    fn validate_version_unknown_gap_in_revisions() {
        use snap::text::EditScript;
        let repo = Repository {
            frontier: "(a@x->2)".parse().unwrap(),
            patches: vec![Patch {
                author: ContributorId::new("a@x").unwrap(),
                revision: 2,
                base: "(a@x->1)".parse().unwrap(),
                message: "second".to_owned(),
                changes: vec![Change::Text {
                    path: "f".to_owned(),
                    edit: EditScript::new(vec![]).unwrap(),
                }],
            }],
        };
        let version: Version = "(a@x->2)".parse().unwrap();
        assert!(validate_version_known(&version, &repo).is_err());
    }

    // ── validate_shared_dots ─────────────────────────────────

    #[test]
    fn shared_dots_identical_patches_ok() {
        use snap::text::{EditOp, EditScript};
        let patch = Patch {
            author: ContributorId::new("a@x").unwrap(),
            revision: 1,
            base: Version::empty(),
            message: "same".to_owned(),
            changes: vec![Change::Text {
                path: "f".to_owned(),
                edit: EditScript::new(vec![EditOp::Insert(vec!["hi\n".to_owned()])]).unwrap(),
            }],
        };
        let local = Repository {
            frontier: "(a@x->1)".parse().unwrap(),
            patches: vec![patch.clone()],
        };
        let remote = Repository {
            frontier: "(a@x->1)".parse().unwrap(),
            patches: vec![patch],
        };
        assert!(validate_shared_dots(&local, &remote).is_ok());
    }

    #[test]
    fn shared_dots_different_patches_fail() {
        use snap::text::{EditOp, EditScript};
        let local = Repository {
            frontier: "(a@x->1)".parse().unwrap(),
            patches: vec![Patch {
                author: ContributorId::new("a@x").unwrap(),
                revision: 1,
                base: Version::empty(),
                message: "local".to_owned(),
                changes: vec![Change::Text {
                    path: "f".to_owned(),
                    edit: EditScript::new(vec![EditOp::Insert(vec!["local\n".to_owned()])])
                        .unwrap(),
                }],
            }],
        };
        let remote = Repository {
            frontier: "(a@x->1)".parse().unwrap(),
            patches: vec![Patch {
                author: ContributorId::new("a@x").unwrap(),
                revision: 1,
                base: Version::empty(),
                message: "remote".to_owned(),
                changes: vec![Change::Text {
                    path: "f".to_owned(),
                    edit: EditScript::new(vec![EditOp::Insert(vec!["remote\n".to_owned()])])
                        .unwrap(),
                }],
            }],
        };
        let err = validate_shared_dots(&local, &remote).unwrap_err();
        match err {
            SnapError::Expected(msg) => assert!(msg.contains("patch collision")),
            SnapError::Internal(_) => panic!("expected Expected error"),
        }
    }

    #[test]
    fn shared_dots_disjoint_patches_ok() {
        use snap::text::EditScript;
        let local = Repository {
            frontier: "(a@x->1)".parse().unwrap(),
            patches: vec![Patch {
                author: ContributorId::new("a@x").unwrap(),
                revision: 1,
                base: Version::empty(),
                message: "local".to_owned(),
                changes: vec![Change::Text {
                    path: "f".to_owned(),
                    edit: EditScript::new(vec![]).unwrap(),
                }],
            }],
        };
        let remote = Repository {
            frontier: "(b@y->1)".parse().unwrap(),
            patches: vec![Patch {
                author: ContributorId::new("b@y").unwrap(),
                revision: 1,
                base: Version::empty(),
                message: "remote".to_owned(),
                changes: vec![Change::Text {
                    path: "g".to_owned(),
                    edit: EditScript::new(vec![]).unwrap(),
                }],
            }],
        };
        assert!(validate_shared_dots(&local, &remote).is_ok());
    }
}
