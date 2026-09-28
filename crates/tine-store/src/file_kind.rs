//! File classes included in graph backup and restore.

use std::path::Path;

use tine_core::model::FileId;

/// Whether a page or journal file identity names Logseq graph text.
pub fn is_graph_text(file: &FileId) -> bool {
    is_graph_text_path(Path::new(file.as_str()))
}

/// Whether a path has a supported graph-text extension; does not inspect its area.
pub(crate) fn is_graph_text_path(path: &Path) -> bool {
    path.extension()
        .and_then(|part| part.to_str())
        .is_some_and(|extension| {
            ["md", "markdown", "org"]
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

/// Whether an asset file identity names a Logseq EDN sidecar.
pub fn is_asset_sidecar(file: &FileId) -> bool {
    is_asset_sidecar_path(Path::new(file.as_str()))
}

/// Whether a path has the EDN sidecar extension; does not inspect its area.
pub(crate) fn is_asset_sidecar_path(path: &Path) -> bool {
    path.extension().and_then(|part| part.to_str()) == Some("edn")
}

#[cfg(test)]
#[test]
fn graph_text_and_sidecar_classes_remain_distinct() {
    assert!(is_graph_text_path(Path::new("pages/Note.md")));
    assert!(is_graph_text_path(Path::new("journals/Note.org")));
    assert!(is_graph_text_path(Path::new("pages/Note.MD")));
    assert!(is_graph_text_path(Path::new("archive/Note.MARKDOWN")));
    assert!(is_graph_text_path(Path::new("archive/Note.ORG")));
    assert!(!is_graph_text_path(Path::new("assets/Note.edn")));
    assert!(is_asset_sidecar_path(Path::new("assets/Note.edn")));
    assert!(!is_asset_sidecar_path(Path::new("assets/Note.EDN")));
    assert!(!is_asset_sidecar_path(Path::new("pages/Note.md")));
}
