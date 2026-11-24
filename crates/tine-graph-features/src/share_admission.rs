//! Generation-2 limits at the source and opened graph boundaries.
use std::io;
use tine_core::doc::Document;

pub(crate) const MAX_BLOCKS: usize = 200_000;
pub(crate) const MAX_STRING_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_PAGES: usize = 201;

fn limit(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

/// Check the number of files and UTF-8 string lengths before graph open.
pub fn check_sizes(
    name: &str,
    exported_at: &str,
    mut files: impl ExactSizeIterator<Item = (usize, usize)>,
) -> io::Result<()> {
    if files.len() > MAX_PAGES + 1 {
        return Err(limit("the shared sources hold too many files"));
    }
    if name.len().max(exported_at.len()) > MAX_STRING_BYTES
        || files.any(|(path, text)| path.max(text) > MAX_STRING_BYTES)
    {
        return Err(limit("a shared source exceeds the 1 MiB string limit"));
    }
    Ok(())
}

/// Admit the outlines that graph loading checked and parsed. Visits at most
/// one block past the limit, with no source reads or reparsing.
pub fn check_blocks<'a>(documents: impl IntoIterator<Item = &'a Document>) -> io::Result<()> {
    let mut remaining = MAX_BLOCKS;
    let mut levels = Vec::new();
    for document in documents {
        levels.push(document.roots.iter());
        while let Some(level) = levels.last_mut() {
            let Some(block) = level.next() else {
                levels.pop();
                continue;
            };
            if remaining == 0 {
                return Err(limit("the shared pages hold too many blocks"));
            }
            remaining -= 1;
            if !block.children.is_empty() {
                levels.push(block.children.iter());
            }
        }
    }
    Ok(())
}

// The cost test (each accepted source parsed once) reads the process-global
// cost counters: it lives in `tests/share_cost.rs` under the exclusive lock.
