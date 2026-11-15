//! Process output. All command output goes through [`Io`] so that stdout carries only the
//! command's result (one JSON document in `--json` mode) and stderr carries diagnostics. Tests
//! inject in-memory buffers.

use std::io::Write;
use std::sync::{Arc, Mutex};

use serde::Serialize;

/// A shareable, line-oriented writer. Write failures (typically a closed pipe, as in
/// `lampo logs | head`) are reported as `false`, never as a panic.
#[derive(Clone)]
pub struct Sink(Arc<Mutex<Box<dyn Write + Send>>>);

impl Sink {
    pub fn new(writer: impl Write + Send + 'static) -> Self {
        Self(Arc::new(Mutex::new(Box::new(writer))))
    }

    /// Writes `text` plus a newline and flushes. `false` when the reader went away.
    pub fn line(&self, text: &str) -> bool {
        let Ok(mut w) = self.0.lock() else { return false };
        writeln!(w, "{text}").and_then(|_| w.flush()).is_ok()
    }
}

#[derive(Clone)]
pub struct Io {
    pub json: bool,
    out: Sink,
    err: Sink,
}

impl Io {
    pub fn stdio(json: bool) -> Self {
        Self { json, out: Sink::new(std::io::stdout()), err: Sink::new(std::io::stderr()) }
    }

    pub fn with_sinks(json: bool, out: Sink, err: Sink) -> Self {
        Self { json, out, err }
    }

    pub fn err_sink(&self) -> Sink {
        self.err.clone()
    }

    /// The command's result: pretty JSON in `--json` mode, otherwise the human rendering.
    pub fn emit<T: Serialize>(&self, value: &T, human: impl FnOnce(&T) -> String) {
        if self.json {
            match serde_json::to_string_pretty(value) {
                Ok(text) => {
                    self.out.line(&text);
                }
                Err(e) => {
                    self.err.line(&format!("lampo: cannot serialise the result: {e}"));
                }
            }
        } else {
            let text = human(value);
            if !text.is_empty() {
                self.out.line(text.trim_end_matches('\n'));
            }
        }
    }

    /// One compact JSON document on its own line (streaming output). `false` when stdout closed.
    pub fn emit_ndjson<T: Serialize>(&self, value: &T) -> bool {
        match serde_json::to_string(value) {
            Ok(text) => self.out.line(&text),
            Err(_) => true,
        }
    }

    /// A plain line on stdout. `false` when stdout closed.
    pub fn out_line(&self, text: &str) -> bool {
        self.out.line(text)
    }

    /// A diagnostic line on stderr.
    pub fn err_line(&self, text: &str) {
        self.err.line(text);
    }
}

/// In-memory output for tests.
#[derive(Clone, Default)]
pub struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// An [`Io`] writing into buffers, plus the buffers (stdout, stderr).
pub fn captured(json: bool) -> (Io, Capture, Capture) {
    let (out, err) = (Capture::default(), Capture::default());
    (Io::with_sinks(json, Sink::new(out.clone()), Sink::new(err.clone())), out, err)
}
