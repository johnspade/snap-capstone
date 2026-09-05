use std::io::Write;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorMode {
    Plain,
    Terminal,
}

pub struct Writer<O: Write, E: Write> {
    stdout: O,
    stderr: E,
    stdout_mode: ColorMode,
    stderr_mode: ColorMode,
}

impl<O: Write, E: Write> Writer<O, E> {
    pub const fn new(stdout: O, stderr: E, stdout_mode: ColorMode, stderr_mode: ColorMode) -> Self {
        Self {
            stdout,
            stderr,
            stdout_mode,
            stderr_mode,
        }
    }

    pub fn stdout(&mut self, msg: &str) {
        let _ = self.stdout.write_all(msg.as_bytes());
    }

    pub fn stderr(&mut self, msg: &str) {
        let _ = self.stderr.write_all(msg.as_bytes());
    }

    pub const fn stdout_mode(&self) -> ColorMode {
        self.stdout_mode
    }

    pub fn style_out(&self, code: u8, text: &str) -> String {
        style(self.stdout_mode, code, text)
    }

    pub fn style_err(&self, code: u8, text: &str) -> String {
        style(self.stderr_mode, code, text)
    }

    pub fn error(&mut self, detail: &str) {
        match self.stderr_mode {
            ColorMode::Plain => {
                let _ = writeln!(self.stderr, "snap: {detail}");
            }
            ColorMode::Terminal => {
                let styled = style(ColorMode::Terminal, 31, &format!("✗ snap: {detail}"));
                let _ = writeln!(self.stderr, "{styled}");
            }
        }
    }

    pub const fn stdout_mut(&mut self) -> &mut O {
        &mut self.stdout
    }

    pub const fn stderr_mut(&mut self) -> &mut E {
        &mut self.stderr
    }

    pub fn warning(&mut self, detail: &str) {
        match self.stderr_mode {
            ColorMode::Plain => {
                let _ = writeln!(self.stderr, "warning: {detail}");
            }
            ColorMode::Terminal => {
                let icon = style(ColorMode::Terminal, 33, "⚠");
                let msg = style(ColorMode::Terminal, 33, detail);
                let _ = writeln!(self.stderr, "{icon} {msg}");
            }
        }
    }
}

fn style(mode: ColorMode, code: u8, text: &str) -> String {
    match mode {
        ColorMode::Plain => text.to_owned(),
        ColorMode::Terminal => format!("\x1b[{code}m{text}\x1b[0m"),
    }
}

/// # Errors
///
/// Returns an error message if `snap_color` is an unrecognized value.
pub fn resolve_color_modes(
    snap_color: Option<&str>,
    no_color_present: bool,
    stdout_is_tty: bool,
    stderr_is_tty: bool,
) -> Result<(ColorMode, ColorMode), String> {
    match snap_color {
        None | Some("auto") => {
            if no_color_present {
                Ok((ColorMode::Plain, ColorMode::Plain))
            } else {
                let out_mode = if stdout_is_tty {
                    ColorMode::Terminal
                } else {
                    ColorMode::Plain
                };
                let err_mode = if stderr_is_tty {
                    ColorMode::Terminal
                } else {
                    ColorMode::Plain
                };
                Ok((out_mode, err_mode))
            }
        }
        Some("always") => Ok((ColorMode::Terminal, ColorMode::Terminal)),
        Some("never") => Ok((ColorMode::Plain, ColorMode::Plain)),
        Some(_) => Err("SNAP_COLOR must be auto, always, or never".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdout_writes_to_stdout_stream() {
        let mut out = Vec::new();
        let err = Vec::new();
        let mut w = Writer::new(&mut out, err, ColorMode::Plain, ColorMode::Plain);
        w.stdout("hello\n");
        assert_eq!(String::from_utf8(out).unwrap(), "hello\n");
    }

    #[test]
    fn error_formats_as_snap_prefix_plain() {
        let out = Vec::new();
        let mut err = Vec::new();
        let mut w = Writer::new(out, &mut err, ColorMode::Plain, ColorMode::Plain);
        w.error("something went wrong");
        assert_eq!(
            String::from_utf8(err).unwrap(),
            "snap: something went wrong\n"
        );
    }

    #[test]
    fn error_formats_with_ansi_in_terminal_mode() {
        let out = Vec::new();
        let mut err = Vec::new();
        let mut w = Writer::new(out, &mut err, ColorMode::Plain, ColorMode::Terminal);
        w.error("bad input");
        assert_eq!(
            String::from_utf8(err).unwrap(),
            "\x1b[31m✗ snap: bad input\x1b[0m\n"
        );
    }

    #[test]
    fn warning_formats_plain() {
        let out = Vec::new();
        let mut err = Vec::new();
        let mut w = Writer::new(out, &mut err, ColorMode::Plain, ColorMode::Plain);
        w.warning("auto-resolved f: later-create-wins");
        assert_eq!(
            String::from_utf8(err).unwrap(),
            "warning: auto-resolved f: later-create-wins\n"
        );
    }

    #[test]
    fn warning_formats_with_ansi_in_terminal_mode() {
        let out = Vec::new();
        let mut err = Vec::new();
        let mut w = Writer::new(out, &mut err, ColorMode::Plain, ColorMode::Terminal);
        w.warning("auto-resolved f: later-create-wins");
        assert_eq!(
            String::from_utf8(err).unwrap(),
            "\x1b[33m⚠\x1b[0m \x1b[33mauto-resolved f: later-create-wins\x1b[0m\n"
        );
    }

    #[test]
    fn style_out_wraps_in_terminal_mode() {
        let w = Writer::new(
            Vec::<u8>::new(),
            Vec::<u8>::new(),
            ColorMode::Terminal,
            ColorMode::Plain,
        );
        assert_eq!(w.style_out(1, "bold"), "\x1b[1mbold\x1b[0m");
    }

    #[test]
    fn style_out_passthrough_in_plain_mode() {
        let w = Writer::new(
            Vec::<u8>::new(),
            Vec::<u8>::new(),
            ColorMode::Plain,
            ColorMode::Plain,
        );
        assert_eq!(w.style_out(1, "bold"), "bold");
    }

    #[test]
    fn style_err_wraps_in_terminal_mode() {
        let w = Writer::new(
            Vec::<u8>::new(),
            Vec::<u8>::new(),
            ColorMode::Plain,
            ColorMode::Terminal,
        );
        assert_eq!(w.style_err(31, "red"), "\x1b[31mred\x1b[0m");
    }

    // ── resolve_color_modes ──────────────────────────────────

    #[test]
    fn auto_no_color_absent_tty_both() {
        let (out, err) = resolve_color_modes(Some("auto"), false, true, true).unwrap();
        assert_eq!(out, ColorMode::Terminal);
        assert_eq!(err, ColorMode::Terminal);
    }

    #[test]
    fn auto_no_color_absent_not_tty() {
        let (out, err) = resolve_color_modes(Some("auto"), false, false, false).unwrap();
        assert_eq!(out, ColorMode::Plain);
        assert_eq!(err, ColorMode::Plain);
    }

    #[test]
    fn auto_no_color_absent_stdout_tty_only() {
        let (out, err) = resolve_color_modes(Some("auto"), false, true, false).unwrap();
        assert_eq!(out, ColorMode::Terminal);
        assert_eq!(err, ColorMode::Plain);
    }

    #[test]
    fn auto_no_color_absent_stderr_tty_only() {
        let (out, err) = resolve_color_modes(Some("auto"), false, false, true).unwrap();
        assert_eq!(out, ColorMode::Plain);
        assert_eq!(err, ColorMode::Terminal);
    }

    #[test]
    fn auto_no_color_present_overrides_tty() {
        let (out, err) = resolve_color_modes(Some("auto"), true, true, true).unwrap();
        assert_eq!(out, ColorMode::Plain);
        assert_eq!(err, ColorMode::Plain);
    }

    #[test]
    fn unset_no_color_absent_not_tty() {
        let (out, err) = resolve_color_modes(None, false, false, false).unwrap();
        assert_eq!(out, ColorMode::Plain);
        assert_eq!(err, ColorMode::Plain);
    }

    #[test]
    fn unset_no_color_present_forces_plain() {
        let (out, err) = resolve_color_modes(None, true, true, true).unwrap();
        assert_eq!(out, ColorMode::Plain);
        assert_eq!(err, ColorMode::Plain);
    }

    #[test]
    fn always_overrides_no_color() {
        let (out, err) = resolve_color_modes(Some("always"), true, false, false).unwrap();
        assert_eq!(out, ColorMode::Terminal);
        assert_eq!(err, ColorMode::Terminal);
    }

    #[test]
    fn always_without_no_color() {
        let (out, err) = resolve_color_modes(Some("always"), false, false, false).unwrap();
        assert_eq!(out, ColorMode::Terminal);
        assert_eq!(err, ColorMode::Terminal);
    }

    #[test]
    fn never_forces_plain() {
        let (out, err) = resolve_color_modes(Some("never"), false, true, true).unwrap();
        assert_eq!(out, ColorMode::Plain);
        assert_eq!(err, ColorMode::Plain);
    }

    #[test]
    fn invalid_value_errors() {
        let err = resolve_color_modes(Some("sometimes"), false, false, false).unwrap_err();
        assert_eq!(err, "SNAP_COLOR must be auto, always, or never");
    }
}
