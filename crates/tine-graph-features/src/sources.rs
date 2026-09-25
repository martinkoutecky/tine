//! Raw graph text for the parser comparison panel. Scans both page areas in
//! O(entries + eligible file bytes); unreadable and oversized files are skipped.

use tine_store::{Area, Store};

/// One UTF-8 source file, with the path shown by the comparison panel.
#[derive(serde::Serialize)]
pub struct GraphSourceFile {
    pub rel: String,
    pub text: String,
    pub format: String,
    pub bytes: u64,
}

/// Collect Markdown and Org sources up to 8 MiB each, in path order. Directory
/// links are not followed. A scan failure is treated as an empty area, as in
/// the prior parser comparison panel.
pub fn graph_source_files(store: &Store, include_journals: bool) -> Vec<GraphSourceFile> {
    const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
    let config = store.config();
    let mut out = Vec::new();
    for (area, prefix) in [
        (Area::Pages, config.pages_dir.as_str()),
        (Area::Journals, config.journals_dir.as_str()),
    ] {
        if area == Area::Journals && !include_journals {
            continue;
        }
        let Ok(listing) = store.scan_area(area, None) else {
            continue;
        };
        for entry in listing.files {
            let format = match entry.rel.rsplit_once('.').map(|(_, ext)| ext) {
                Some("md") => "md",
                Some("org") => "org",
                _ => continue,
            };
            let Some(meta) = entry.meta else { continue };
            if meta.len > MAX_FILE_BYTES {
                continue;
            }
            let Ok((bytes, _)) = store.read(&entry.id, Some(MAX_FILE_BYTES)) else {
                continue;
            };
            let Ok(text) = String::from_utf8(bytes) else {
                continue;
            };
            out.push(GraphSourceFile {
                rel: format!("{prefix}/{}", entry.rel),
                text,
                format: format.to_owned(),
                bytes: meta.len,
            });
        }
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}
