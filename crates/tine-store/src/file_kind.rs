//! File classes included in graph backup and restore.

use std::path::Path;

/// Whether a page or journal path is Logseq graph text.
pub fn is_graph_text(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|part| part.to_str()),
        Some("md" | "org")
    )
}

/// Whether an asset path is a Logseq EDN sidecar.
pub fn is_asset_sidecar(path: &Path) -> bool {
    path.extension().and_then(|part| part.to_str()) == Some("edn")
}
