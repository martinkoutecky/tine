//! File classes and spelling candidates shared by graph clients.

use std::path::Path;

use tine_core::model::FileId;

/// Whether two file identities are candidates for a case-only entry move: the same
/// parent and different leaves equal under the store's Unicode fold.
/// This proves neither alias identity nor destination absence. Publication
/// must still check those and use the guarded no-replace move recipe.
pub fn case_only(src: &FileId, dst: &FileId) -> bool {
    case_only_paths(Path::new(src.as_str()), Path::new(dst.as_str()))
}

pub(crate) fn case_only_paths(src: &Path, dst: &Path) -> bool {
    src != dst
        && src.parent() == dst.parent()
        && src
            .file_name()
            .zip(dst.file_name())
            .is_some_and(|(from, to)| {
                crate::model::entry_identity::fold_leaf(from)
                    == crate::model::entry_identity::fold_leaf(to)
            })
}

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
