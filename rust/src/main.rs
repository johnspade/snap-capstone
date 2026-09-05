use std::io::IsTerminal;
use std::process::ExitCode;

use snap::writer::{ColorMode, Writer, resolve_color_modes};

mod commands;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let snap_color = std::env::var("SNAP_COLOR").ok();
    let no_color_present = std::env::var_os("NO_COLOR").is_some();

    let stdout = std::io::stdout();
    let stderr = std::io::stderr();

    let (stdout_mode, stderr_mode) = match resolve_color_modes(
        snap_color.as_deref(),
        no_color_present,
        stdout.is_terminal(),
        stderr.is_terminal(),
    ) {
        Ok(modes) => modes,
        Err(msg) => {
            let mut writer = Writer::new(
                stdout.lock(),
                stderr.lock(),
                ColorMode::Plain,
                ColorMode::Plain,
            );
            writer.error(&msg);
            return ExitCode::from(1);
        }
    };

    let mut writer = Writer::new(stdout.lock(), stderr.lock(), stdout_mode, stderr_mode);
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

#[derive(Debug)]
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
        ["status"] => commands::status(writer),
        ["log"] => commands::log(writer),
        ["commit", msg] => commands::commit(writer, msg),
        ["revert", arg] if !arg.starts_with('-') => commands::revert(writer, arg),
        ["merge", arg] if !arg.starts_with('-') => commands::merge(writer, arg),
        ["diff"] => commands::diff_working(writer),
        ["diff", old, new] => commands::diff_versions(writer, old, new),
        ["diff", old, new, "--repo", repo] => commands::diff_cross_repo(writer, old, new, repo),
        ["diff", ..] => Err(SnapError::Expected(
            "usage: snap diff [<old> <new> [--repo <repository>]]".to_owned(),
        )),
        ["--serve"] => commands::serve(writer, None),
        ["--serve", port] => commands::serve(writer, Some(port)),
        _ => Err(invalid_command_or_args()),
    }
}

fn invalid_command_or_args() -> SnapError {
    SnapError::Expected("invalid command or arguments".to_owned())
}
