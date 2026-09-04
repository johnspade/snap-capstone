use std::path::{Path, PathBuf};
use std::process::ExitCode;

use snap::config;
use snap::repository;
use snap::version::ContributorId;
use snap::writer::Writer;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let mut writer = Writer::new(stdout.lock(), stderr.lock());
    match run(&args, &mut writer) {
        Ok(()) => ExitCode::from(0),
        Err(SnapError::Expected(msg)) => {
            writer.error(&msg);
            ExitCode::from(1)
        }
        Err(SnapError::Internal(msg)) => {
            writer.error(&msg);
            ExitCode::from(2)
        }
    }
}

enum SnapError {
    Expected(String),
    Internal(String),
}

fn run<O: std::io::Write, E: std::io::Write>(
    args: &[String],
    writer: &mut Writer<O, E>,
) -> Result<(), SnapError> {
    let args_str: Vec<&str> = args.iter().map(String::as_str).collect();
    match args_str.as_slice() {
        ["--version"] => cmd_version(writer),
        ["init"] => cmd_init(writer, None),
        ["init", path] if !path.starts_with('-') => cmd_init(writer, Some(path)),
        ["config", "--global", "contributor.id", id] => cmd_config(writer, true, id),
        ["config", "contributor.id", id] if !id.starts_with('-') => cmd_config(writer, false, id),
        ["status"] => cmd_require_repo("status"),
        ["log"] => cmd_require_repo("log"),
        ["commit", _msg] => cmd_require_repo("commit"),
        ["revert", arg] if !arg.starts_with('-') => cmd_require_repo("revert"),
        ["merge", arg] if !arg.starts_with('-') => cmd_require_repo("merge"),
        ["diff"] | ["diff", _, _] | ["diff", _, _, "--repo", _] => cmd_require_repo("diff"),
        ["diff", ..] => Err(SnapError::Expected(
            "usage: snap diff [<old> <new> [--repo <repository>]]".to_owned(),
        )),
        ["--serve"] | ["--serve", _] => cmd_require_repo("serve"),
        _ => Err(invalid_command_or_args()),
    }
}

fn invalid_command_or_args() -> SnapError {
    SnapError::Expected("invalid command or arguments".to_owned())
}

fn cmd_require_repo(_cmd: &str) -> Result<(), SnapError> {
    find_repo_from_cwd().ok_or_else(|| SnapError::Expected("not a Snap repository".to_owned()))?;
    Err(SnapError::Expected("not implemented".to_owned()))
}

#[expect(clippy::unnecessary_wraps, reason = "consistent command signature")]
fn cmd_version<O: std::io::Write, E: std::io::Write>(
    writer: &mut Writer<O, E>,
) -> Result<(), SnapError> {
    writer.stdout(&format!("snap {}\n", env!("CARGO_PKG_VERSION")));
    Ok(())
}

fn cmd_init<O: std::io::Write, E: std::io::Write>(
    writer: &mut Writer<O, E>,
    path: Option<&str>,
) -> Result<(), SnapError> {
    let target = match path {
        Some(p) => std::env::current_dir()
            .map_err(|e| SnapError::Internal(e.to_string()))?
            .join(p),
        None => std::env::current_dir().map_err(|e| SnapError::Internal(e.to_string()))?,
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

fn cmd_config<O: std::io::Write, E: std::io::Write>(
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

fn find_repo_from_cwd() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    find_repo_above(&cwd)
}

fn find_repo_above(start: &Path) -> Option<PathBuf> {
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
