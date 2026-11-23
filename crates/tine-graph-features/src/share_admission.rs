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

#[cfg(test)]
mod tests {
    use super::*;
    use tine_store::{cost_counters, WholeGraph};

    #[test]
    fn accepted_share_parses_each_source_once() {
        let files = vec![
            ("pages/Public.md".to_owned(), b"- text\n".repeat(10_000)),
            ("pages/Other.org".to_owned(), b"* text\n".repeat(10_000)),
        ];
        cost_counters::reset();
        let started = std::time::Instant::now();
        check_sizes(
            "Share",
            "",
            files.iter().map(|(path, text)| (path.len(), text.len())),
        )
        .unwrap();
        let (graph, _) = WholeGraph::from_files(files).unwrap();
        check_blocks(
            graph
                .corpus(None, false)
                .pages
                .iter()
                .map(|p| p.document.as_ref()),
        )
        .unwrap();
        let count = cost_counters::snapshot();
        eprintln!(
            "share_open sources=2 blocks=20000 outline_parses={} elapsed={:?}",
            count.parses,
            started.elapsed()
        );
        assert_eq!((count.parses, count.full_reads, count.corpus), (2, 2, 1));
    }
}
