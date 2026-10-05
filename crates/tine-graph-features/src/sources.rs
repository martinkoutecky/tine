//! Raw graph text for the parser comparison panel. Scans both page areas in
//! O(entries + eligible file bytes). A file that cannot be listed, read or
//! decoded, or exceeds the comparison size limit, is skipped and named in
//! [`GraphSources::skipped`]; it never silently shrinks the comparison.

use std::path::Path;
use tine_core::model::Format;
use tine_store::{Area, Store};

/// One UTF-8 source file, with the path shown by the comparison panel.
#[derive(serde::Serialize)]
pub struct GraphSourceFile {
    pub rel: String,
    pub text: String,
    pub format: String,
    pub bytes: u64,
}

/// The comparison input: every eligible file in path order, and `path: reason`
/// for each file left out.
#[derive(serde::Serialize)]
pub struct GraphSources {
    pub files: Vec<GraphSourceFile>,
    pub skipped: Vec<String>,
}

/// Collect Markdown and Org sources up to 8 MiB each, in path order. Directory
/// links are not followed. A failed scan of a whole area is an error (it is
/// not an empty area); a single bad file is skipped and reported.
pub fn graph_source_files(store: &Store, include_journals: bool) -> std::io::Result<GraphSources> {
    const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    for area in [Area::Pages, Area::Journals] {
        if area == Area::Journals && !include_journals {
            continue;
        }
        let (listed, unreadable) = crate::conflicts::area_listing(store, area)?;
        skipped.extend(unreadable);
        for entry in listed {
            if !tine_store::is_graph_text(&entry.id) {
                continue;
            }
            let rel = entry.id.as_str().to_owned();
            // The one format answer the parser uses (`.markdown`/`.MD` are Markdown).
            let format = Format::from_path(Path::new(&entry.rel)).ext();
            let Some(meta) = entry.meta else {
                skipped.push(format!("{rel}: no file metadata"));
                continue;
            };
            if meta.len > MAX_FILE_BYTES {
                skipped.push(format!(
                    "{rel}: over the {MAX_FILE_BYTES}-byte comparison limit"
                ));
                continue;
            }
            let bytes = match store.read(&entry.id, Some(MAX_FILE_BYTES)) {
                Ok((bytes, _)) => bytes,
                Err(tine_store::StoreError::NotFound) => continue,
                Err(error) => {
                    let error = crate::store_error(error);
                    if crate::conflicts::store_failed(&error) {
                        return Err(error);
                    }
                    skipped.push(format!("{rel}: {error}"));
                    continue;
                }
            };
            let Ok(text) = String::from_utf8(bytes) else {
                skipped.push(format!("{rel}: not valid UTF-8"));
                continue;
            };
            files.push(GraphSourceFile {
                rel,
                text,
                format: format.to_owned(),
                bytes: meta.len,
            });
        }
    }
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    skipped.sort();
    Ok(GraphSources { files, skipped })
}
