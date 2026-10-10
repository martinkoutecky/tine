use std::fs::File;
use std::io::{self, Seek, SeekFrom};
use std::path::Path;

use super::io_helpers::{content_refusal, failed};
use super::{Content, FileId, Refusal, RenameMap, Store, Why};

pub(crate) fn rewrite(
    old: &[u8],
    path: &Path,
    map: &RenameMap,
    name_format: tine_core::config::FileNameFormat,
) -> Result<Vec<u8>, Why> {
    #[cfg(feature = "test-faults")]
    crate::cost_counters::transaction_rewrite();
    let text = std::str::from_utf8(old).map_err(|_| Why::Refused(Refusal::Undecodable))?;
    let is_org = path.extension().and_then(|ext| ext.to_str()) == Some("org");
    let rewritten = tine_core::refs::rename_rewrite(text, is_org, &map.0, name_format);
    refuse_read_only_org(text.as_bytes(), rewritten.as_bytes(), is_org)?;
    Ok(rewritten.into_bytes())
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
    map: &RenameMap,
    name_format: tine_core::config::FileNameFormat,
) -> Result<Vec<u8>, Why> {
    let rewritten = rewrite(old, path, map, name_format)?;
    let format = tine_core::model::Format::from_path(path);
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return Ok(rewritten);
    };
    let destination_name = tine_core::model::decode_page_name(stem, name_format);
    let source = std::str::from_utf8(old).map_err(|_| Why::Refused(Refusal::Undecodable))?;
    let Some(title) = tine_core::model::page_title_from_preamble(source, format) else {
        return Ok(rewritten);
    };
    let Some((from, new_name)) = map.0.iter().find(|(from, to)| {
        tine_core::refs::same_page(from, &title)
            && tine_core::refs::same_page(to, &destination_name)
    }) else {
        return Ok(rewritten);
    };
    let text = std::str::from_utf8(&rewritten).map_err(|_| Why::Refused(Refusal::Undecodable))?;
    let Some(rebound) = tine_core::model::rebind_page_title(text, format, from, new_name) else {
        return Ok(rewritten);
    };
    if format == tine_core::model::Format::Org && !tine_core::org::org_editable(source) {
        return Err(Why::Refused(Refusal::ReadOnly(
            "org file is read-only (does not round-trip)".into(),
        )));
    }
    Ok(rebound.into_bytes())
}

pub(super) fn valid_utf8_file(path: &Path) -> io::Result<bool> {
    use std::io::Read;
    let mut file = File::open(path)?;
    let mut carry = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(carry.is_empty());
        }
        carry.extend_from_slice(&buf[..n]);
        match std::str::from_utf8(&carry) {
            Ok(_) => carry.clear(),
            Err(error) if error.error_len().is_none() => {
                let valid = error.valid_up_to();
                carry.drain(..valid);
            }
            Err(_) => return Ok(false),
        }
    }
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
    match content {
        Content::Bytes(bytes) => {
            crate::model::validate_parse_bytes_for_path(bytes, Path::new(file.as_str()))
                .map_err(content_refusal)
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
                .map_err(content_refusal)
        }
    }
}
