use std::collections::HashMap;
use std::fs;
use std::path::Path;

use thiserror::Error;

pub type Tree = HashMap<String, Vec<u8>>;

#[derive(Debug, Error)]
pub enum FsError {
    #[error("invalid tracked path: {0}")]
    InvalidPath(String),
    #[error("prefix conflict: {path} and {nested}")]
    PrefixConflict { path: String, nested: String },
    #[error("snap: unsupported working tree entry: {0}")]
    UnsupportedEntry(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Validates a single tracked path per §2.
///
/// # Errors
/// Returns `FsError::InvalidPath` if the path violates any rule.
pub fn validate_path(path: &str) -> Result<(), FsError> {
    let err = || FsError::InvalidPath(path.to_owned());

    if path.is_empty() {
        return Err(err());
    }

    if path.bytes().any(|b| b.is_ascii_control()) {
        return Err(err());
    }

    if path.contains('\\') {
        return Err(err());
    }

    for (i, segment) in path.split('/').enumerate() {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(err());
        }
        if i == 0 && segment == ".snap" {
            return Err(err());
        }
    }

    Ok(())
}

/// Validates that no path in the tree is a prefix of another (by segment).
///
/// # Errors
/// Returns `FsError::PrefixConflict` if a file path is a segment-prefix of another.
pub fn validate_prefix_free(tree: &Tree) -> Result<(), FsError> {
    let mut paths: Vec<&str> = tree.keys().map(String::as_str).collect();
    paths.sort_unstable();

    for pair in paths.windows(2) {
        let a = pair[0];
        let b = pair[1];
        if b.starts_with(a) && b.as_bytes().get(a.len()) == Some(&b'/') {
            return Err(FsError::PrefixConflict {
                path: a.to_owned(),
                nested: b.to_owned(),
            });
        }
    }

    Ok(())
}

/// Recursively scans the working tree, excluding `.snap/`.
///
/// # Errors
/// Returns `FsError::UnsupportedEntry` for symlinks or special files.
/// Returns `FsError::Io` for I/O failures.
pub fn scan_working_tree(root: &Path) -> Result<Tree, FsError> {
    let mut tree = Tree::new();
    scan_dir(root, root, &mut tree)?;
    Ok(tree)
}

fn scan_dir(root: &Path, dir: &Path, tree: &mut Tree) -> Result<(), FsError> {
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);

    for entry in entries {
        let path = entry.path();

        if dir == root && entry.file_name() == ".snap" {
            continue;
        }

        let relative = path.strip_prefix(root).expect("path is under root");
        let key = relative
            .to_str()
            .ok_or_else(|| FsError::UnsupportedEntry(relative.to_string_lossy().into_owned()))?;

        let file_type = entry.file_type()?;

        if file_type.is_symlink() {
            return Err(FsError::UnsupportedEntry(key.to_owned()));
        }

        if file_type.is_dir() {
            scan_dir(root, &path, tree)?;
        } else if file_type.is_file() {
            tree.insert(key.to_owned(), fs::read(&path)?);
        } else {
            return Err(FsError::UnsupportedEntry(key.to_owned()));
        }
    }

    Ok(())
}

/// Returns `true` when the working tree matches the reference exactly.
#[must_use]
pub fn is_clean(working: &Tree, reference: &Tree) -> bool {
    working == reference
}

/// Writes the target tree to disk, replacing all tracked content.
///
/// Clears everything under `root` except `.snap/`, then writes target files.
///
/// # Errors
/// Returns `FsError::Io` on I/O failures.
pub fn materialize(root: &Path, target: &Tree) -> Result<(), FsError> {
    clear_working_tree(root)?;

    for (path, content) in target {
        let full = root.join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&full, content)?;
    }

    Ok(())
}

fn clear_working_tree(root: &Path) -> Result<(), FsError> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_name() == ".snap" {
            continue;
        }
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Path validation ─────────────────────────────────────────────

    #[test]
    fn valid_simple_path() {
        assert!(validate_path("hello.txt").is_ok());
    }

    #[test]
    fn valid_nested_path() {
        assert!(validate_path("src/main.rs").is_ok());
    }

    #[test]
    fn valid_deeply_nested() {
        assert!(validate_path("a/b/c/d/e.txt").is_ok());
    }

    #[test]
    fn valid_dotfile() {
        assert!(validate_path(".gitignore").is_ok());
    }

    #[test]
    fn valid_dot_in_middle() {
        assert!(validate_path("a/.hidden/b").is_ok());
    }

    #[test]
    fn rejects_empty() {
        assert!(matches!(validate_path(""), Err(FsError::InvalidPath(_))));
    }

    #[test]
    fn rejects_control_chars() {
        assert!(matches!(
            validate_path("a\x00b"),
            Err(FsError::InvalidPath(_))
        ));
        assert!(matches!(
            validate_path("a\nb"),
            Err(FsError::InvalidPath(_))
        ));
        assert!(matches!(
            validate_path("a\x7Fb"),
            Err(FsError::InvalidPath(_))
        ));
    }

    #[test]
    fn rejects_backslash() {
        assert!(matches!(
            validate_path("a\\b"),
            Err(FsError::InvalidPath(_))
        ));
    }

    #[test]
    fn rejects_empty_segment() {
        assert!(matches!(
            validate_path("a//b"),
            Err(FsError::InvalidPath(_))
        ));
        assert!(matches!(validate_path("/a"), Err(FsError::InvalidPath(_))));
        assert!(matches!(validate_path("a/"), Err(FsError::InvalidPath(_))));
    }

    #[test]
    fn rejects_dot_segment() {
        assert!(matches!(
            validate_path("a/./b"),
            Err(FsError::InvalidPath(_))
        ));
        assert!(matches!(validate_path("."), Err(FsError::InvalidPath(_))));
    }

    #[test]
    fn rejects_dotdot_segment() {
        assert!(matches!(
            validate_path("a/../b"),
            Err(FsError::InvalidPath(_))
        ));
        assert!(matches!(validate_path(".."), Err(FsError::InvalidPath(_))));
    }

    #[test]
    fn rejects_snap_first_segment() {
        assert!(matches!(
            validate_path(".snap/config.json"),
            Err(FsError::InvalidPath(_))
        ));
        assert!(matches!(
            validate_path(".snap"),
            Err(FsError::InvalidPath(_))
        ));
    }

    #[test]
    fn allows_snap_in_non_first_segment() {
        assert!(validate_path("a/.snap/b").is_ok());
    }

    #[test]
    fn allows_unicode() {
        assert!(validate_path("café/résumé.txt").is_ok());
    }

    // ── Prefix-free validation ──────────────────────────────────────

    #[test]
    fn prefix_free_no_conflict() {
        let tree: Tree = [("a".to_owned(), vec![]), ("b/c".to_owned(), vec![])]
            .into_iter()
            .collect();
        assert!(validate_prefix_free(&tree).is_ok());
    }

    #[test]
    fn prefix_free_detects_conflict() {
        let tree: Tree = [("a".to_owned(), vec![]), ("a/b".to_owned(), vec![])]
            .into_iter()
            .collect();
        assert!(matches!(
            validate_prefix_free(&tree),
            Err(FsError::PrefixConflict { .. })
        ));
    }

    #[test]
    fn prefix_free_detects_nested_conflict() {
        let tree: Tree = [("a/b".to_owned(), vec![]), ("a/b/c".to_owned(), vec![])]
            .into_iter()
            .collect();
        assert!(matches!(
            validate_prefix_free(&tree),
            Err(FsError::PrefixConflict { .. })
        ));
    }

    #[test]
    fn prefix_free_no_false_positive_on_shared_prefix() {
        let tree: Tree = [("abc".to_owned(), vec![]), ("abcd".to_owned(), vec![])]
            .into_iter()
            .collect();
        assert!(validate_prefix_free(&tree).is_ok());
    }

    #[test]
    fn prefix_free_empty_tree() {
        let tree = Tree::new();
        assert!(validate_prefix_free(&tree).is_ok());
    }

    #[test]
    fn prefix_free_single_entry() {
        let tree: Tree = std::iter::once(("a".to_owned(), vec![])).collect();
        assert!(validate_prefix_free(&tree).is_ok());
    }

    // ── Dirty detection ─────────────────────────────────────────────

    #[test]
    fn clean_when_equal() {
        let a: Tree = std::iter::once(("x".to_owned(), vec![1, 2])).collect();
        let b: Tree = std::iter::once(("x".to_owned(), vec![1, 2])).collect();
        assert!(is_clean(&a, &b));
    }

    #[test]
    fn dirty_different_content() {
        let a: Tree = std::iter::once(("x".to_owned(), vec![1])).collect();
        let b: Tree = std::iter::once(("x".to_owned(), vec![2])).collect();
        assert!(!is_clean(&a, &b));
    }

    #[test]
    fn dirty_extra_file() {
        let a: Tree = [("x".to_owned(), vec![]), ("y".to_owned(), vec![])]
            .into_iter()
            .collect();
        let b: Tree = std::iter::once(("x".to_owned(), vec![])).collect();
        assert!(!is_clean(&a, &b));
    }

    #[test]
    fn dirty_missing_file() {
        let a: Tree = std::iter::once(("x".to_owned(), vec![])).collect();
        let b: Tree = [("x".to_owned(), vec![]), ("y".to_owned(), vec![])]
            .into_iter()
            .collect();
        assert!(!is_clean(&a, &b));
    }

    #[test]
    fn clean_both_empty() {
        assert!(is_clean(&Tree::new(), &Tree::new()));
    }
}

#[cfg(all(test, not(miri)))]
mod io_tests {
    use super::*;

    // ── Scanning ────────────────────────────────────────────────────

    #[test]
    fn scan_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let tree = scan_working_tree(dir.path()).unwrap();
        assert!(tree.is_empty());
    }

    #[test]
    fn scan_single_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("hello.txt"), b"world").unwrap();
        let tree = scan_working_tree(dir.path()).unwrap();
        assert_eq!(tree.len(), 1);
        assert_eq!(tree.get("hello.txt").unwrap(), b"world");
    }

    #[test]
    fn scan_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a/b")).unwrap();
        fs::write(dir.path().join("a/b/c.txt"), b"deep").unwrap();
        fs::write(dir.path().join("top.txt"), b"top").unwrap();
        let tree = scan_working_tree(dir.path()).unwrap();
        assert_eq!(tree.len(), 2);
        assert_eq!(tree.get("a/b/c.txt").unwrap(), b"deep");
        assert_eq!(tree.get("top.txt").unwrap(), b"top");
    }

    #[test]
    fn scan_excludes_snap_dir() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".snap")).unwrap();
        fs::write(dir.path().join(".snap/repository.json"), b"{}").unwrap();
        fs::write(dir.path().join("tracked.txt"), b"yes").unwrap();
        let tree = scan_working_tree(dir.path()).unwrap();
        assert_eq!(tree.len(), 1);
        assert!(tree.contains_key("tracked.txt"));
        assert!(!tree.contains_key(".snap/repository.json"));
    }

    #[test]
    fn scan_rejects_symlink() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("real.txt"), b"data").unwrap();
        std::os::unix::fs::symlink("real.txt", dir.path().join("link")).unwrap();
        let err = scan_working_tree(dir.path()).unwrap_err();
        assert!(matches!(err, FsError::UnsupportedEntry(ref p) if p == "link"));
    }

    #[test]
    fn scan_rejects_symlink_in_subdir() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::os::unix::fs::symlink("nowhere", dir.path().join("sub/link")).unwrap();
        let err = scan_working_tree(dir.path()).unwrap_err();
        assert!(matches!(err, FsError::UnsupportedEntry(ref p) if p == "sub/link"));
    }

    #[test]
    fn scan_rejects_fifo() {
        let dir = tempfile::tempdir().unwrap();
        let fifo_path = dir.path().join("pipe");
        std::process::Command::new("mkfifo")
            .arg(&fifo_path)
            .status()
            .unwrap();
        let err = scan_working_tree(dir.path()).unwrap_err();
        assert!(matches!(err, FsError::UnsupportedEntry(ref p) if p == "pipe"));
    }

    #[test]
    fn scan_skips_empty_dirs() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("empty")).unwrap();
        let tree = scan_working_tree(dir.path()).unwrap();
        assert!(tree.is_empty());
    }

    // ── Materialization ─────────────────────────────────────────────

    #[test]
    fn materialize_creates_files() {
        let dir = tempfile::tempdir().unwrap();
        let target: Tree = [
            ("a.txt".to_owned(), b"hello".to_vec()),
            ("b/c.txt".to_owned(), b"world".to_vec()),
        ]
        .into_iter()
        .collect();
        materialize(dir.path(), &target).unwrap();
        assert_eq!(fs::read(dir.path().join("a.txt")).unwrap(), b"hello");
        assert_eq!(fs::read(dir.path().join("b/c.txt")).unwrap(), b"world");
    }

    #[test]
    fn materialize_removes_extra_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("stale.txt"), b"old").unwrap();
        let target: Tree = std::iter::once(("fresh.txt".to_owned(), b"new".to_vec())).collect();
        materialize(dir.path(), &target).unwrap();
        assert!(!dir.path().join("stale.txt").exists());
        assert_eq!(fs::read(dir.path().join("fresh.txt")).unwrap(), b"new");
    }

    #[test]
    fn materialize_preserves_snap_dir() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".snap")).unwrap();
        fs::write(dir.path().join(".snap/repository.json"), b"{}").unwrap();
        let target: Tree = std::iter::once(("a.txt".to_owned(), b"data".to_vec())).collect();
        materialize(dir.path(), &target).unwrap();
        assert!(dir.path().join(".snap/repository.json").exists());
        assert_eq!(
            fs::read(dir.path().join(".snap/repository.json")).unwrap(),
            b"{}"
        );
    }

    #[test]
    fn materialize_replaces_file_with_directory() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a"), b"file").unwrap();
        let target: Tree = std::iter::once(("a/b.txt".to_owned(), b"nested".to_vec())).collect();
        materialize(dir.path(), &target).unwrap();
        assert_eq!(fs::read(dir.path().join("a/b.txt")).unwrap(), b"nested");
    }

    #[test]
    fn materialize_replaces_directory_with_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a/b")).unwrap();
        fs::write(dir.path().join("a/b/c.txt"), b"deep").unwrap();
        let target: Tree = std::iter::once(("a".to_owned(), b"now a file".to_vec())).collect();
        materialize(dir.path(), &target).unwrap();
        assert_eq!(fs::read(dir.path().join("a")).unwrap(), b"now a file");
    }

    #[test]
    fn materialize_empty_target() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("old.txt"), b"data").unwrap();
        materialize(dir.path(), &Tree::new()).unwrap();
        assert!(!dir.path().join("old.txt").exists());
    }

    #[test]
    fn materialize_scan_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let target: Tree = [
            ("a.txt".to_owned(), b"hello".to_vec()),
            ("d/e/f.txt".to_owned(), b"deep".to_vec()),
        ]
        .into_iter()
        .collect();
        materialize(dir.path(), &target).unwrap();
        let scanned = scan_working_tree(dir.path()).unwrap();
        assert_eq!(scanned, target);
    }

    #[test]
    fn materialize_scan_round_trip_with_snap() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".snap")).unwrap();
        fs::write(dir.path().join(".snap/repository.json"), b"{}").unwrap();
        let target: Tree = std::iter::once(("x.txt".to_owned(), b"data".to_vec())).collect();
        materialize(dir.path(), &target).unwrap();
        let scanned = scan_working_tree(dir.path()).unwrap();
        assert_eq!(scanned, target);
    }
}
