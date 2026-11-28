//! Develop's page reader as it was before tine-text (Tine `d9dc45177`,
//! `tine-core/src/outline.rs` `headers` + `blocks` and `doc.rs` `document_of`),
//! frozen as the oracle for content equality: every block's `raw`, in source
//! order, with its level. Test code only; nothing in production calls it.

/// Develop's refusal: the whole page is pre-block text.
#[derive(Debug)]
pub struct Refused;

/// The `(level, raw)` of every block in source order, or develop's refusal.
pub fn blocks(text: &str, org: bool) -> Result<Vec<(u32, String)>, Refused> {
    let lf = if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text.to_string()
    };
    let headers = headers(&lf, org)?;
    let body = if org {
        lf.trim_end_matches('\n')
    } else {
        lf.strip_suffix('\n').unwrap_or(&lf)
    };
    let lines: Vec<&str> = if body.is_empty() {
        Vec::new()
    } else {
        body.split('\n').collect()
    };
    let mut out = Vec::new();
    for (n, header) in headers.iter().enumerate() {
        let end = headers.get(n + 1).map_or(lines.len(), |next| next.line);
        let mut raw = lines[header.line][header.prefix_len..].to_string();
        for line in lines.iter().take(end).skip(header.line + 1) {
            raw.push('\n');
            raw.push_str(strip_layout_ws(line, header.content_indent));
        }
        out.push((header.level, raw));
    }
    Ok(out)
}

struct Header {
    line: usize,
    level: u32,
    prefix_len: usize,
    content_indent: usize,
}

fn headers(text: &str, org: bool) -> Result<Vec<Header>, Refused> {
    use lsdoc::OutlineHeaderKind as K;
    let events = tine_text::bound::parse_outline_bounded(text, org).ok_or(Refused)?;
    let mut line_starts = vec![0usize];
    line_starts.extend(text.match_indices('\n').map(|(at, _)| at + 1));
    let mut out: Vec<Header> = Vec::with_capacity(events.len());
    for header in &events {
        let prefix = header.structural_prefix;
        let accepts = matches!(
            (org, header.kind),
            (
                false,
                K::MarkdownUnbulletedAtxHeading | K::MarkdownDashBullet
            ) | (true, K::OrgHeadline)
        );
        let valid = accepts
            && header.level > 0
            && header.line_content.slice(text).is_some()
            && prefix.slice(text).is_some()
            && prefix.start == header.line.start
            && prefix.end <= header.line_content.end
            && header.line_content.start == header.line.start;
        if !valid {
            return Err(Refused);
        }
        let Ok(line) = line_starts.binary_search(&header.line.start) else {
            return Err(Refused);
        };
        if header.header_start != header.line.start || out.last().is_some_and(|h| h.line >= line) {
            return Err(Refused);
        }
        let prefix_len = prefix.end - prefix.start;
        let content_indent = match header.kind {
            K::MarkdownDashBullet if text[..prefix.end].ends_with("- ") => prefix_len,
            K::MarkdownDashBullet => prefix_len + 1,
            _ => 0,
        };
        out.push(Header {
            line,
            level: header.level,
            prefix_len,
            content_indent,
        });
    }
    Ok(out)
}

fn strip_layout_ws(line: &str, n: usize) -> &str {
    let skip = line
        .bytes()
        .take(n)
        .take_while(|b| matches!(b, b' ' | b'\t' | 0x0c))
        .count();
    &line[skip..]
}
