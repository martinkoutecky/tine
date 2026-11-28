//! Frozen page projection from develop d9dc45177 (`outline.rs`, `doc.rs`).
//! Keep this independent of `PageText`, including its normalization and
//! physical line handling. The pinned parser remains the outline oracle.

use lsdoc::OutlineHeaderKind::*;
use tine_text::{line_quote_depth, Format};

pub fn read(source: &str, format: Format) -> Vec<(u32, String)> {
    let text = source.replace("\r\n", "\n").replace('\r', "\n");
    let org = format == Format::Org;
    let body = if org {
        text.trim_end_matches('\n')
    } else {
        text.strip_suffix('\n').unwrap_or(&text)
    };
    let lines: Vec<_> = if body.is_empty() {
        Vec::new()
    } else {
        body.split('\n').collect()
    };
    let starts: Vec<_> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    let headers = headers(&text, format, &starts).unwrap_or_default();
    headers
        .iter()
        .enumerate()
        .map(|(n, &(line, level, prefix, indent))| {
            let end = headers.get(n + 1).map_or(lines.len(), |next| next.0);
            let mut raw = lines[line][prefix..].to_owned();
            for line in &lines[line + 1..end] {
                let left = line
                    .bytes()
                    .take(indent)
                    .take_while(|b| matches!(b, b' ' | b'\t' | 0x0c))
                    .count();
                raw.push('\n');
                raw.push_str(&line[left..]);
            }
            (level, raw)
        })
        .collect()
}

fn headers(
    text: &str,
    format: Format,
    starts: &[usize],
) -> Option<Vec<(usize, u32, usize, usize)>> {
    if text
        .split(['\n', '\r'])
        .any(|line| line_quote_depth(line) > 128)
    {
        return None;
    }
    let events = lsdoc::parse_outline(text, if format == Format::Org { "org" } else { "md" })
        .ok()?
        .headers;
    let mut result: Vec<(usize, u32, usize, usize)> = Vec::with_capacity(events.len());
    for header in events {
        let prefix = header.structural_prefix;
        let valid = matches!(
            (format, header.kind),
            (Format::Org, OrgHeadline)
                | (
                    Format::Markdown,
                    MarkdownDashBullet | MarkdownUnbulletedAtxHeading
                )
        ) && header.level > 0
            && header.line_content.slice(text).is_some()
            && prefix.slice(text).is_some()
            && prefix.start == header.line.start
            && prefix.end <= header.line_content.end
            && header.line_content.start == header.line.start;
        let line = starts.binary_search(&header.line.start).ok()?;
        if !valid
            || header.header_start != header.line.start
            || result.last().is_some_and(|h| h.0 >= line)
        {
            return None;
        }
        let prefix_len = prefix.end - prefix.start;
        let indent = match header.kind {
            MarkdownDashBullet if text[..prefix.end].ends_with("- ") => prefix_len,
            MarkdownDashBullet => prefix_len + 1,
            _ => 0,
        };
        result.push((line, header.level, prefix_len, indent));
    }
    Some(result)
}
