use std::fs::File;
use std::io::{self, Seek, SeekFrom};
use std::path::Path;
use tine_core::model::Format;

use super::io_helpers::{content_refusal, failed};
use super::{Content, FileId, Refusal, RenameMap, Store, Why};
use std::collections::HashMap;
use std::sync::Arc;

/// Per rename map, the first pair that renames a page of one identity to
/// another, keyed by both identities, so a moved page's own title is matched
/// in O(1) rather than by a scan of the map (PC-3: a namespace rename moves
/// K pages of one K-entry map). Built once per map in a preflight.
#[derive(Default)]
pub(crate) struct TitleIndexes {
    moves: HashMap<*const RenameMap, HashMap<(String, String), usize>>,
    links: HashMap<*const RenameMap, HashMap<String, String>>,
}

impl TitleIndexes {
    /// `map`'s new name for a page titled `title` moved to `destination`: the
    /// first pair whose old name is `title` and whose new name is
    /// `destination`, up to page identity.
    fn new_name<'m>(
        &mut self,
        map: &'m Arc<RenameMap>,
        title: &str,
        destination: &str,
    ) -> Option<&'m str> {
        let index = self.moves.entry(Arc::as_ptr(map)).or_insert_with(|| {
            #[cfg(feature = "test-faults")]
            crate::cost_counters::rename_title_probes(map.0.len());
            let mut index = HashMap::new();
            for (at, (from, to)) in map.0.iter().enumerate() {
                index
                    .entry((
                        tine_core::refs::page_key(from),
                        tine_core::refs::page_key(to),
                    ))
                    .or_insert(at);
            }
            index
        });
        #[cfg(feature = "test-faults")]
        crate::cost_counters::rename_title_probes(1);
        let at = index.get(&(
            tine_core::refs::page_key(title),
            tine_core::refs::page_key(destination),
        ))?;
        Some(&map.0[*at].1)
    }
}

pub(crate) fn rewrite(
    old: &[u8],
    path: &Path,
    map: &RenameMap,
    name_format: tine_core::config::FileNameFormat,
) -> Result<Vec<u8>, Why> {
    let proposal = rewrite_proposal(old, path, map, name_format)?;
    let is_org = Format::from_path(path) == Format::Org;
    refuse_read_only_org(old, proposal.text.as_bytes(), is_org)?;
    Ok(proposal.text.into_bytes())
}

/// Check the actual title image, including a stationary namespace and reused
/// prepared bytes. Titles and namespace images retain their existing owners.
/// O(affected page bytes), no I/O; journals keep their filename identity.
pub(super) fn title_identity(
    old: &[u8],
    new: &[u8],
    format: Format,
    map: &Arc<RenameMap>,
    titles: &mut TitleIndexes,
) -> Result<(), Why> {
    let old = std::str::from_utf8(old).map_err(|_| Why::Refused(Refusal::Undecodable))?;
    let new = std::str::from_utf8(new).map_err(|_| Why::Refused(Refusal::Undecodable))?;
    if let Some(title) = tine_core::model::page_title_from_preamble(old, format) {
        let keyed = titles.links.entry(Arc::as_ptr(map)).or_insert_with(|| {
            map.0
                .iter()
                .map(|(a, b)| (tine_core::refs::page_key(a), b.clone()))
                .collect()
        });
        let expected = tine_core::refs::renamed_page_name(&title, &keyed);
        let actual = tine_core::model::page_title_from_preamble(new, format);
        if actual.as_deref().is_none_or(|actual| {
            !tine_core::refs::same_page(actual, expected.as_deref().unwrap_or(&title))
        }) {
            return Err(Why::Refused(Refusal::TextRealization(
                tine_core::text_trace::TraceError::TitleIdentity,
            )));
        }
    }
    Ok(())
}

fn rewrite_proposal(
    old: &[u8],
    path: &Path,
    map: &RenameMap,
    name_format: tine_core::config::FileNameFormat,
) -> Result<tine_core::refs::RenameProposal, Why> {
    #[cfg(feature = "test-faults")]
    crate::cost_counters::transaction_rewrite();
    let text = std::str::from_utf8(old).map_err(|_| Why::Refused(Refusal::Undecodable))?;
    let is_org = Format::from_path(path) == Format::Org;
    let keyed = |pairs: &[(String, String)]| {
        pairs
            .iter()
            .map(|(a, b)| (tine_core::refs::page_key(a), b.clone()))
            .collect()
    };
    let proposal =
        tine_core::refs::rename_proposal(text, is_org, &keyed(&map.0), &keyed(&map.1), name_format)
            .map_err(|e| Why::Refused(Refusal::TextRealization(e)))?;
    Ok(proposal)
}

/// Refuse changing an Org file that does not round-trip. Scenario: malformed
/// imported Org content that Tine cannot rewrite without losing bytes.
pub(super) fn refuse_read_only_org(old: &[u8], new: &[u8], is_org: bool) -> Result<(), Why> {
    if is_org && old != new && !std::str::from_utf8(old).is_ok_and(tine_core::org::org_editable) {
        return Err(Why::Refused(Refusal::ReadOnly(
            "org file is read-only (does not round-trip)".into(),
        )));
    }
    Ok(())
}

/// A rename move changes a file's logical identity as well as its path. Keep
/// the own preamble title (Markdown `title::`, Org `#+title:` or `:title:`) in
/// the same guarded transaction when it still names the old identity; a custom
/// title remains untouched. A non-round-tripping Org file is refused rather
/// than rewritten.
pub(crate) fn rewrite_move(
    old: &[u8],
    path: &Path,
    map: &Arc<RenameMap>,
    titles: &mut TitleIndexes,
    name_format: tine_core::config::FileNameFormat,
    proposed: Option<tine_core::refs::RenameProposal>,
) -> Result<Vec<u8>, Why> {
    // `proposed`: the reference rewrite already proposed for these `old`
    // bytes (carried from the caller's planning read and checked by
    // `PreparedRewrite::reuse_proposal`), so the page is not walked again.
    let mut proposal = match proposed {
        Some(proposal) => proposal,
        None => rewrite_proposal(old, path, map, name_format)?,
    };
    refuse_read_only_org(
        old,
        proposal.text.as_bytes(),
        Format::from_path(path) == Format::Org,
    )?;
    let format = tine_core::model::Format::from_path(path);
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return Ok(proposal.text.into_bytes());
    };
    let destination_name = tine_core::model::decode_page_name(stem, name_format);
    let source = std::str::from_utf8(old).map_err(|_| Why::Refused(Refusal::Undecodable))?;
    #[cfg(any(test, feature = "test-faults"))]
    if source.contains(crate::model::TEST_TITLE_READ_PANIC_SENTINEL) {
        panic!("deterministic test sentinel for a rename title-read panic");
    }
    let Some(title) = tine_core::model::page_title_from_preamble(source, format) else {
        return Ok(proposal.text.into_bytes());
    };
    let Some(new_name) = titles.new_name(map, &title, &destination_name) else {
        return Ok(proposal.text.into_bytes());
    };
    proposal
        .rebind_title(format == Format::Org, new_name)
        .map_err(|e| Why::Refused(Refusal::TextRealization(e)))?;
    if format == tine_core::model::Format::Org && !tine_core::org::org_editable(source) {
        return Err(Why::Refused(Refusal::ReadOnly(
            "org file is read-only (does not round-trip)".into(),
        )));
    }
    Ok(proposal.text.into_bytes())
}

pub(super) fn validate_stream(source: &File, max_bytes: u64) -> Result<(), Why> {
    use std::io::Read;
    let mut input = source.try_clone().map_err(failed)?;
    input.seek(SeekFrom::Start(0)).map_err(failed)?;
    let mut total = 0u64;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = input.read(&mut buf).map_err(failed)?;
        if n == 0 {
            break;
        }
        total = total.saturating_add(n as u64);
        if total > max_bytes {
            return Err(failed(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("stream exceeds {max_bytes} byte limit"),
            )));
        }
    }
    Ok(())
}

pub(super) fn validate_config_bytes(store: &Store, bytes: &[u8]) -> Result<(), Why> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        // An undecodable config is recorded as ConfigState::problem. Keep the
        // existing repair path for callers replacing those raw bytes.
        return Ok(());
    };
    let config = tine_core::config::Config::parse(text);
    store
        .graph
        .validate_config_layout(&config)
        .map_err(|error| Why::Refused(Refusal::InvalidTarget(error.to_string())))
}

pub(super) fn validate_config_content(store: &Store, content: &Content) -> Result<(), Why> {
    match content {
        Content::Bytes(bytes) => validate_config_bytes(store, bytes),
        Content::Stream { source, max_bytes } => {
            use std::io::Read;
            let mut input = source.try_clone().map_err(failed)?;
            input.seek(SeekFrom::Start(0)).map_err(failed)?;
            let mut bytes = Vec::new();
            input
                .take((*max_bytes).min(crate::model::PARSE_INPUT_MAX_BYTES) + 1)
                .read_to_end(&mut bytes)
                .map_err(failed)?;
            validate_config_bytes(store, &bytes)
        }
    }
}

pub(super) fn validate_page_content(file: &FileId, content: &Content) -> Result<(), Why> {
    page_content_bytes(file, content).map(drop)
}

/// Stage raw creation bytes with the same bounds and validation on the
/// transaction and planning paths.
/// The page input checks of [`page_content_bytes`] for bytes in memory.
pub(super) fn page_bytes(file: &FileId, bytes: &[u8]) -> Result<(), Why> {
    crate::model::validate_parse_bytes_for_path(bytes, Path::new(file.as_str()))
        .map_err(content_refusal)
}

pub(super) fn page_content_bytes<'a>(
    file: &FileId,
    content: &'a Content,
) -> Result<std::borrow::Cow<'a, [u8]>, Why> {
    match content {
        Content::Bytes(bytes) => {
            crate::model::validate_parse_bytes_for_path(bytes, Path::new(file.as_str()))
                .map_err(content_refusal)?;
            Ok(std::borrow::Cow::Borrowed(bytes))
        }
        Content::Stream { source, max_bytes } => {
            use std::io::Read;
            let mut input = source.try_clone().map_err(failed)?;
            input.seek(SeekFrom::Start(0)).map_err(failed)?;
            let limit = (*max_bytes).min(crate::model::PARSE_INPUT_MAX_BYTES);
            let mut bytes = Vec::new();
            input
                .take(limit + 1)
                .read_to_end(&mut bytes)
                .map_err(failed)?;
            if bytes.len() as u64 > limit {
                return Err(Why::Refused(Refusal::InvalidTarget(format!(
                    "page content exceeds {limit} byte limit"
                ))));
            }
            crate::model::validate_parse_bytes_for_path(&bytes, Path::new(file.as_str()))
                .map_err(content_refusal)?;
            Ok(std::borrow::Cow::Owned(bytes))
        }
    }
}
