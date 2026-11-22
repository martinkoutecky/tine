//! Generation-2 admission for the writer and the browser before graph open.
use crate::publication_source::visit_document_calls;
use crate::render::{is_query_body, parsed_body, whole_begin_query_payload};
use std::{cell::Cell, io, path::Path};
use tine_core::model::Format;

pub(crate) const MAX_QUERIES: usize = 256;
pub(crate) const MAX_BLOCKS: usize = 200_000;
pub(crate) const MAX_STRING_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_PAGES: usize = 201;
const MAX_BLOCK_DEPTH: usize = 253;

fn limit(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

pub(crate) fn check_sizes(
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

/// Check shared source string sizes and count parser-recognized outline
/// blocks and query calls, stopping at the first excess. Cost is bounded by
/// the document byte limit at the envelope boundary. No graph or I/O.
pub fn admit(
    name: &str,
    exported_at: &str,
    files: impl ExactSizeIterator<Item = (impl AsRef<str>, impl AsRef<[u8]>)> + Clone,
) -> io::Result<()> {
    check_sizes(
        name,
        exported_at,
        files
            .clone()
            .map(|(path, text)| (path.as_ref().len(), text.as_ref().len())),
    )?;
    let mut blocks = 0;
    let queries = Cell::new(0);
    let count_query = || {
        queries.set(queries.get() + 1);
        if queries.get() > MAX_QUERIES {
            Err(limit("the shared pages hold too many queries"))
        } else {
            Ok(())
        }
    };
    for (path, bytes) in files {
        let (path, bytes) = (path.as_ref(), bytes.as_ref());
        if path == "logseq/config.edn" {
            continue;
        }
        let text = std::str::from_utf8(bytes).map_err(io::Error::other)?;
        let org = Format::from_path(Path::new(path)) == Format::Org;
        if !tine_store::model::parse_input_depth_within_limit(text)
            || (org
                && !tine_core::org::headline_levels_within_limit(
                    text,
                    tine_core::outline_kernel::OUTLINE_MAX_DEPTH,
                ))
        {
            return Err(limit("the shared pages nest too deeply to share"));
        }
        let document = if org {
            tine_core::org::parse_org(text)
        } else {
            tine_core::doc::parse(text)
        };
        visit_document_calls(
            &document,
            |depth, raw, org| {
                blocks += 1;
                if blocks > MAX_BLOCKS {
                    return Err(limit("the shared pages hold too many blocks"));
                }
                if depth > MAX_BLOCK_DEPTH {
                    return Err(limit("the shared pages nest too deeply to share"));
                }
                if whole_begin_query_payload(raw).is_some() && is_query_body(&parsed_body(raw, org))
                {
                    count_query()?;
                }
                Ok(())
            },
            |name, _, _| {
                if tine_core::query::is_query_macro_name(name) {
                    count_query()?;
                }
                Ok(())
            },
        )?;
    }
    Ok(())
}
