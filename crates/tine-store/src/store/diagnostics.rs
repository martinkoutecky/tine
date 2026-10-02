//! The store's diagnostics dump section (GH #623 follow-up): launch phase
//! timings, recent full stat diffs and saves, and graph-shape statistics.
//! Numbers and closed tokens only (I-5); see `model/shape_stats.rs` for the
//! privacy rule and its test.
use super::*;

impl Store {
    /// A JSON object of numbers and closed status tokens for the diagnostics
    /// dump: `launch`, `onDemandBuilds`, `fullDiffs`, `saves` and `shape`. It
    /// holds no page name, path or text. Reads the built page cache without
    /// building it, so `shape` is `{"ready": false}` until the first load
    /// completes; computing it walks every cached block (O(blocks), no file
    /// reads), so call it off the UI thread.
    pub fn diagnostics(&self) -> serde_json::Value {
        let status = match &*self.load.status.lock().unwrap() {
            LoadStatus::Loading => "loading",
            LoadStatus::Ready => "ready",
            LoadStatus::Failed(_) => "failed",
            LoadStatus::Closed => "closed",
        };
        let mut report = self.graph.diag.snapshot(status);
        let began = std::time::Instant::now();
        let shape = match self.graph.peek_pages() {
            Some(pages) => crate::model::shape_stats::shape(
                &pages,
                self.watch.core_for_load().file_facts(),
                self.graph.diag.crlf_at_last_load(),
            ),
            None => serde_json::json!({ "ready": false }),
        };
        report["shape"] = shape;
        report["shapeMs"] = serde_json::json!(began.elapsed().as_secs_f64() * 1e3);
        report
    }
}
