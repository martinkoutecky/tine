use crate::{line_terminator, physical_lines, quote_depth_within, SOURCE_QUOTE_DEPTH_MAX};
use lsdoc::{OutlineHeader, OutlineHeaderKind};

/// The page syntax to pass to lsdoc.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Format {
    Markdown,
    Org,
}

impl Format {
    fn accepts(self, kind: OutlineHeaderKind) -> bool {
        matches!(
            (self, kind),
            (
                Self::Markdown,
                OutlineHeaderKind::MarkdownUnbulletedAtxHeading
                    | OutlineHeaderKind::MarkdownDashBullet
            ) | (Self::Org, OutlineHeaderKind::OrgHeadline)
        )
    }
}

/// One outline header in the document reader's line coordinates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    pub line: usize,
    pub level: u32,
    pub prefix_len: usize,
    pub content_indent: usize,
}

/// Why the compatibility reader shows the page as preamble.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutlineRefusal {
    Unowned,
    NotOnePerLine { event: usize },
    Invalid { event: usize },
}

pub(crate) struct LocatedHeader {
    pub header: Header,
    pub suffix: usize,
    pub missing_separator: bool,
}

/// One lsdoc parse, guarded before its quote recursion starts.
fn parse_outline_bounded(text: &str, format: Format) -> Option<Vec<OutlineHeader>> {
    if !quote_depth_within(text, SOURCE_QUOTE_DEPTH_MAX) {
        return None;
    }
    let format = if format == Format::Org { "org" } else { "md" };
    lsdoc::parse_outline(text, format)
        .ok()
        .map(|outline| outline.headers)
}

/// Whether lsdoc reads the whole input as one unbulleted Markdown heading.
pub fn is_unbulleted_heading_line(line: &str) -> bool {
    !line.contains(['\n', '\r'])
        && parse_outline_bounded(line, Format::Markdown).is_some_and(|events| {
            matches!(events.as_slice(), [only]
                if only.kind == OutlineHeaderKind::MarkdownUnbulletedAtxHeading
                    && only.header_start == 0)
        })
}

pub(crate) fn locate_headers(
    text: &str,
    format: Format,
) -> Result<Vec<LocatedHeader>, OutlineRefusal> {
    let events = parse_outline_bounded(text, format).ok_or(OutlineRefusal::Unowned)?;
    let mut lines = physical_lines(text).enumerate();
    let mut line_end = 0;
    let mut content_end = 0;
    let mut line_start = 0;
    let mut line = 0;
    let mut out: Vec<LocatedHeader> = Vec::with_capacity(events.len());
    for (event, header) in events.into_iter().enumerate() {
        let prefix = header.structural_prefix;
        let valid = format.accepts(header.kind)
            && header.level > 0
            && header.line_content.slice(text).is_some()
            && prefix.slice(text).is_some()
            && prefix.start == header.line.start
            && prefix.end <= header.line_content.end
            && header.line_content.start == header.line.start;
        if !valid {
            return Err(OutlineRefusal::Invalid { event });
        }
        while line_end <= header.line.start {
            let Some((index, physical)) = lines.next() else {
                return Err(OutlineRefusal::Invalid { event });
            };
            line = index;
            line_start = line_end;
            line_end += physical.len();
            content_end = line_end - line_terminator(physical).len();
        }
        if out.last().is_some_and(|h| h.header.line >= line) {
            return Err(OutlineRefusal::NotOnePerLine { event });
        }
        if header.line.start != line_start || prefix.end > content_end {
            return Err(OutlineRefusal::Invalid { event });
        }
        if header.header_start != line_start
            && (!(line_start..content_end).contains(&header.header_start)
                || !text.is_char_boundary(header.header_start))
        {
            return Err(OutlineRefusal::Invalid { event });
        }
        let suffix = header.header_start - line_start;
        let prefix_len = if suffix == 0 {
            prefix.end - prefix.start
        } else {
            0
        };
        let missing_separator = suffix == 0
            && match header.kind {
                OutlineHeaderKind::MarkdownDashBullet => !text[..prefix.end].ends_with("- "),
                OutlineHeaderKind::OrgHeadline => !text[..prefix.end].ends_with(' '),
                OutlineHeaderKind::MarkdownUnbulletedAtxHeading => false,
            };
        let content_indent = if header.kind == OutlineHeaderKind::MarkdownDashBullet {
            prefix_len + usize::from(missing_separator)
        } else {
            0
        };
        out.push(LocatedHeader {
            header: Header {
                line,
                level: header.level,
                prefix_len,
                content_indent,
            },
            suffix,
            missing_separator,
        });
    }
    Ok(out)
}

/// Headers in the existing document projection. Accepts LF or CRLF input.
/// A mid-line event makes the whole outline unrepresentable in this view.
pub fn headers(text: &str, format: Format) -> Result<Vec<Header>, OutlineRefusal> {
    locate_headers(text, format)?
        .into_iter()
        .enumerate()
        .map(|(event, found)| {
            if found.suffix != 0 {
                Err(OutlineRefusal::NotOnePerLine { event })
            } else {
                Ok(found.header)
            }
        })
        .collect()
}
