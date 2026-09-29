//! The one way crate production code writes a line for a person to read.
//!
//! **Operation.** [`diagnostic_line`] prints a fixed, source-owned line to
//! stderr and hands it to the host's sink, which the desktop app points at its
//! opt-in `--debug` log file ([`set_diagnostic_line_sink`]). stderr alone
//! reaches nobody on the Windows release app, which has no console (GH #594).
//!
//! **Privacy (I-5).** The line is `&'static str`: page names, paths and
//! content cannot be formatted into it. Private detail belongs to the app's
//! debug-only `diag_private`.
//!
//! **Cost.** O(line length) plus the sink's own write; never fails. The first
//! installed sink wins; with none installed the line goes to stderr only.
//! `crates/tine-store/tests/i05_log_sinks.rs` keeps this the only production
//! stderr writer in `tine-core`, `tine-store` and `tine-graph-features`.

type DiagnosticLineSink = Box<dyn Fn(&str) + Send + Sync>;
static DIAGNOSTIC_LINE_SINK: std::sync::OnceLock<DiagnosticLineSink> = std::sync::OnceLock::new();

/// Install the host's copy of every [`diagnostic_line`]. The first install
/// wins; later calls are ignored.
pub fn set_diagnostic_line_sink(sink: impl Fn(&str) + Send + Sync + 'static) {
    let _ = DIAGNOSTIC_LINE_SINK.set(Box::new(sink));
}

/// Print `line` to stderr and hand it to the installed sink, if any.
pub fn diagnostic_line(line: &'static str) {
    eprintln!("{line}");
    if let Some(sink) = DIAGNOSTIC_LINE_SINK.get() {
        sink(line);
    }
}

#[cfg(test)]
mod tests {
    /// GH #594: the host's sink is handed every line, so the app can put it in
    /// the `--debug` log a reporter sends.
    #[test]
    fn a_diagnostic_line_reaches_the_host_sink() {
        static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
        super::set_diagnostic_line_sink(|line| SEEN.lock().unwrap().push(line.to_owned()));
        super::diagnostic_line("tine probe 594");
        assert!(SEEN
            .lock()
            .unwrap()
            .iter()
            .any(|line| line == "tine probe 594"));
    }
}
