use std::fs::File;
use std::io::{self, Seek, SeekFrom};
use std::path::Path;

use super::io_helpers::{content_refusal, failed};
use super::{Content, FileId, Refusal, RenameMap, Store, Why};

pub(super) fn rewrite(old: &[u8], path: &Path, map: &RenameMap) -> Result<Vec<u8>, Why> {
    let text = std::str::from_utf8(old).map_err(|_| Why::Refused(Refusal::Undecodable))?;
    let is_org = path.extension().and_then(|ext| ext.to_str()) == Some("org");
    let renames: std::collections::HashMap<String, String> = map
        .0
        .iter()
        .map(|(from, to)| (tine_core::refs::normalize(from), to.clone()))
        .collect();
    let rewritten = tine_core::refs::rename_tags_property_multi(
        &tine_core::refs::rename_refs_multi(text, &renames, is_org),
        &renames,
        is_org,
    );
    if is_org && rewritten != text && !tine_core::org::org_editable(text) {
        return Err(Why::Refused(Refusal::ReadOnly(
            "org file is read-only (does not round-trip)".into(),
        )));
    }
    Ok(rewritten.into_bytes())
}

/// A rename move changes a file's logical identity as well as its path. Keep
/// the own Markdown title in the same guarded transaction when it still names
/// the old identity; a custom title remains untouched.
pub(super) fn rewrite_move(
    old: &[u8],
    path: &Path,
    map: &RenameMap,
    name_format: tine_core::config::FileNameFormat,
) -> Result<Vec<u8>, Why> {
    let rewritten = rewrite(old, path, map)?;
    if !path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
    {
        return Ok(rewritten);
    }
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return Ok(rewritten);
    };
    let destination_name = tine_core::model::decode_page_name(stem, name_format);
    let source = std::str::from_utf8(old).map_err(|_| Why::Refused(Refusal::Undecodable))?;
    let Some(title) =
        tine_core::model::page_title_from_preamble(source, tine_core::model::Format::Md)
    else {
        return Ok(rewritten);
    };
    let Some((_, new_name)) = map.0.iter().find(|(from, to)| {
        tine_core::refs::same_page(from, &title)
            && tine_core::refs::same_page(to, &destination_name)
    }) else {
        return Ok(rewritten);
    };
    let text = std::str::from_utf8(&rewritten).map_err(|_| Why::Refused(Refusal::Undecodable))?;
    let mut offset = 0;
    for chunk in text.split_inclusive('\n') {
        let line = chunk.trim_end_matches(['\r', '\n']);
        let trimmed = line.trim_start();
        if trimmed == "-" || trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            break;
        }
        if tine_core::doc::parse_property_line(line)
            .is_some_and(|(key, _)| key.eq_ignore_ascii_case("title"))
        {
            let newline = if chunk.ends_with("\r\n") {
                "\r\n"
            } else if chunk.ends_with('\n') {
                "\n"
            } else {
                ""
            };
            let mut result = String::with_capacity(text.len() + new_name.len());
            result.push_str(&text[..offset]);
            result.push_str("title:: ");
            result.push_str(new_name);
            result.push_str(newline);
            result.push_str(&text[offset + chunk.len()..]);
            return Ok(result.into_bytes());
        }
        offset += chunk.len();
    }
    Ok(rewritten)
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
