use std::process::ExitCode;

use snap::writer::Writer;

mod commands;

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
        ["--version"] => commands::version(writer),
        ["init"] => commands::init(writer, None),
        ["init", path] if !path.starts_with('-') => commands::init(writer, Some(path)),
        ["config", "--global", "contributor.id", id] => commands::config(writer, true, id),
        ["config", "contributor.id", id] if !id.starts_with('-') => {
            commands::config(writer, false, id)
        }
        ["status"] => commands::require_repo("status"),
        ["log"] => commands::require_repo("log"),
        ["commit", _msg] => commands::require_repo("commit"),
        ["revert", arg] if !arg.starts_with('-') => commands::require_repo("revert"),
        ["merge", arg] if !arg.starts_with('-') => commands::require_repo("merge"),
        ["diff"] | ["diff", _, _] | ["diff", _, _, "--repo", _] => commands::require_repo("diff"),
        ["diff", ..] => Err(SnapError::Expected(
            "usage: snap diff [<old> <new> [--repo <repository>]]".to_owned(),
        )),
        ["--serve"] | ["--serve", _] => commands::require_repo("serve"),
        _ => Err(invalid_command_or_args()),
    }
}

fn invalid_command_or_args() -> SnapError {
    SnapError::Expected("invalid command or arguments".to_owned())
}
