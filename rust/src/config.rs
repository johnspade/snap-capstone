use std::collections::HashSet;
use std::path::Path;

use thiserror::Error;

use crate::version::ContributorId;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid JSON in configuration file")]
    InvalidJson(String),
    #[error("duplicate JSON key: {0}")]
    DuplicateKey(String),
    #[error("invalid contributor id: {0}")]
    InvalidContributorId(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Read contributor ID from a config file.
///
/// Returns `Ok(None)` if the file doesn't exist.
/// Returns `Err` if the file exists but is malformed, has unknown fields,
/// duplicate keys, or an invalid contributor ID.
///
/// # Errors
/// Returns `ConfigError` on malformed config or I/O failure.
pub fn read_config_file(path: &Path) -> Result<Option<ContributorId>, ConfigError> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(ConfigError::Io(e)),
    };
    check_config_duplicate_keys(&content)?;
    let raw: RawConfig =
        serde_json::from_str(&content).map_err(|e| ConfigError::InvalidJson(e.to_string()))?;
    let id = ContributorId::new(&raw.contributor.id)
        .map_err(|_| ConfigError::InvalidContributorId(raw.contributor.id))?;
    Ok(Some(id))
}

/// Write a config file with the given contributor ID.
///
/// Overwrites the file completely — no unknown fields are preserved.
///
/// # Errors
/// Returns `ConfigError::Io` on I/O failure.
pub fn write_config_file(path: &Path, id: &ContributorId) -> Result<(), ConfigError> {
    let json = format!("{{\"contributor\":{{\"id\":\"{}\"}}}}\n", id.as_str());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, json)?;
    Ok(())
}

/// Resolve contributor ID using local-over-global precedence.
///
/// 1. Read local `.snap/config.json` if `repo_root` is `Some`.
///    If it provides an ID, return it (skip global).
/// 2. Otherwise read `$HOME/.snapconfig.json`.
///
/// # Errors
/// Returns `ConfigError` on malformed config files or I/O failure.
pub fn resolve_contributor(repo_root: Option<&Path>) -> Result<Option<ContributorId>, ConfigError> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    resolve_contributor_with_home(repo_root, home.as_deref())
}

/// Like [`resolve_contributor`] but accepts an explicit home directory.
///
/// # Errors
/// Returns `ConfigError` on malformed config files or I/O failure.
pub fn resolve_contributor_with_home(
    repo_root: Option<&Path>,
    home: Option<&Path>,
) -> Result<Option<ContributorId>, ConfigError> {
    if let Some(root) = repo_root {
        let local_path = root.join(".snap/config.json");
        match read_config_file(&local_path) {
            Ok(Some(id)) => return Ok(Some(id)),
            Ok(None) => {}
            Err(e) => return Err(e),
        }
    }

    let Some(home) = home else {
        return Ok(None);
    };
    let global_path = home.join(".snapconfig.json");
    read_config_file(&global_path)
}

fn check_config_duplicate_keys(json: &str) -> Result<(), ConfigError> {
    let bytes = json.as_bytes();
    let mut in_string = false;
    let mut escape = false;
    let mut object_keys_stack: Vec<HashSet<String>> = Vec::new();
    let mut awaiting_key = false;
    let mut current_key_start: Option<usize> = None;

    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_string = false;
                if let Some(start) = current_key_start.take() {
                    let key_json = &bytes[start..=i];
                    let key: String = serde_json::from_slice(key_json)
                        .map_err(|e| ConfigError::InvalidJson(e.to_string()))?;
                    if let Some(keys) = object_keys_stack.last_mut() {
                        if !keys.insert(key.clone()) {
                            return Err(ConfigError::DuplicateKey(key));
                        }
                    }
                    awaiting_key = false;
                }
            }
        } else {
            match b {
                b'"' => {
                    in_string = true;
                    if awaiting_key {
                        current_key_start = Some(i);
                    }
                }
                b'{' => {
                    object_keys_stack.push(HashSet::new());
                    awaiting_key = true;
                }
                b'}' => {
                    object_keys_stack.pop();
                    awaiting_key = false;
                }
                b',' if !object_keys_stack.is_empty() => {
                    awaiting_key = true;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    contributor: RawContributor,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContributor {
    id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_missing_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.json");
        assert!(read_config_file(&path).unwrap().is_none());
    }

    #[test]
    fn read_valid_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"contributor":{"id":"a@x"}}"#).unwrap();
        let id = read_config_file(&path).unwrap().unwrap();
        assert_eq!(id.as_str(), "a@x");
    }

    #[test]
    fn read_rejects_malformed_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "not json").unwrap();
        assert!(matches!(
            read_config_file(&path),
            Err(ConfigError::InvalidJson(_))
        ));
    }

    #[test]
    fn read_rejects_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"contributor":{"id":"a@x"},"unknown":true}"#).unwrap();
        assert!(matches!(
            read_config_file(&path),
            Err(ConfigError::InvalidJson(_))
        ));
    }

    #[test]
    fn read_rejects_unknown_field_in_contributor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"contributor":{"id":"a@x","extra":1}}"#).unwrap();
        assert!(matches!(
            read_config_file(&path),
            Err(ConfigError::InvalidJson(_))
        ));
    }

    #[test]
    fn read_rejects_invalid_contributor_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"contributor":{"id":"no-at-sign"}}"#).unwrap();
        assert!(matches!(
            read_config_file(&path),
            Err(ConfigError::InvalidContributorId(_))
        ));
    }

    #[test]
    fn read_rejects_duplicate_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"contributor":{"id":"a@x","id":"b@x"}}"#).unwrap();
        assert!(matches!(
            read_config_file(&path),
            Err(ConfigError::DuplicateKey(_))
        ));
    }

    #[test]
    fn write_creates_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let id = ContributorId::new("a@x").unwrap();
        write_config_file(&path, &id).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["contributor"]["id"], "a@x");
    }

    #[test]
    fn write_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/dir/config.json");
        let id = ContributorId::new("a@x").unwrap();
        write_config_file(&path, &id).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn write_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"contributor":{"id":"old@x"},"unknown":true}"#).unwrap();
        let id = ContributorId::new("new@x").unwrap();
        write_config_file(&path, &id).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["contributor"]["id"], "new@x");
        assert!(parsed.get("unknown").is_none());
    }

    #[test]
    fn resolve_local_takes_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join(".snap")).unwrap();
        std::fs::write(
            repo.join(".snap/config.json"),
            r#"{"contributor":{"id":"local@x"}}"#,
        )
        .unwrap();

        let id = resolve_contributor(Some(&repo)).unwrap().unwrap();
        assert_eq!(id.as_str(), "local@x");
    }

    #[test]
    fn resolve_none_without_local_or_home() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join(".snap")).unwrap();

        let id = resolve_contributor_with_home(Some(&repo), None).unwrap();
        assert!(id.is_none());
    }

    #[test]
    fn resolve_global_when_no_local() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join(".snap")).unwrap();

        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join(".snapconfig.json"),
            r#"{"contributor":{"id":"global@x"}}"#,
        )
        .unwrap();

        let id = resolve_contributor_with_home(Some(&repo), Some(&home))
            .unwrap()
            .unwrap();
        assert_eq!(id.as_str(), "global@x");
    }

    #[test]
    fn resolve_local_blocks_global_even_if_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join(".snap")).unwrap();
        std::fs::write(
            repo.join(".snap/config.json"),
            r#"{"contributor":{"id":"not-an-id"}}"#,
        )
        .unwrap();

        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join(".snapconfig.json"),
            r#"{"contributor":{"id":"global@x"}}"#,
        )
        .unwrap();

        assert!(resolve_contributor_with_home(Some(&repo), Some(&home)).is_err());
    }
}
