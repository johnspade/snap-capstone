use std::path::{Path, PathBuf};

use snap::config;
use snap::repository;
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

pub fn require_repo(_cmd: &str) -> Result<(), SnapError> {
    find_repo_from_cwd().ok_or_else(|| SnapError::Expected("not a Snap repository".to_owned()))?;
    Err(SnapError::Expected("not implemented".to_owned()))
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
