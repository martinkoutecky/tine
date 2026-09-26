//! File classes included in graph backup and restore.

use std::path::Path;

use tine_core::model::FileId;

/// Whether a page or journal file identity names Logseq graph text.
pub fn is_graph_text(file: &FileId) -> bool {
    is_graph_text_path(Path::new(file.as_str()))
}

pub(crate) fn is_graph_text_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|part| part.to_str()),
        Some("md" | "org")
    )
}

/// Whether an asset file identity names a Logseq EDN sidecar.
pub fn is_asset_sidecar(file: &FileId) -> bool {
    is_asset_sidecar_path(Path::new(file.as_str()))
}

pub(crate) fn is_asset_sidecar_path(path: &Path) -> bool {
    path.extension().and_then(|part| part.to_str()) == Some("edn")
}
