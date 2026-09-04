use std::io::Write;

pub struct Writer<O: Write, E: Write> {
    stdout: O,
    stderr: E,
}

impl<O: Write, E: Write> Writer<O, E> {
    pub const fn new(stdout: O, stderr: E) -> Self {
        Self { stdout, stderr }
    }

    pub fn stdout(&mut self, msg: &str) {
        let _ = self.stdout.write_all(msg.as_bytes());
    }

    pub fn error(&mut self, detail: &str) {
        let _ = writeln!(self.stderr, "snap: {detail}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdout_writes_to_stdout_stream() {
        let mut out = Vec::new();
        let err = Vec::new();
        let mut w = Writer::new(&mut out, err);
        w.stdout("hello\n");
        assert_eq!(String::from_utf8(out).unwrap(), "hello\n");
    }

    #[test]
    fn error_formats_as_snap_prefix() {
        let out = Vec::new();
        let mut err = Vec::new();
        let mut w = Writer::new(out, &mut err);
        w.error("something went wrong");
        assert_eq!(
            String::from_utf8(err).unwrap(),
            "snap: something went wrong\n"
        );
    }
}
