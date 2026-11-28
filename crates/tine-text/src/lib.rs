//! Physical Markdown and Org page text. lsdoc decides which headers open an
//! outline slot. This crate assigns file ranges, projects each slot into LF
//! content coordinates and applies splices to those ranges in memory.
//!
//! Reading takes one outline parse plus linear file-local work. The file is
//! borrowed and lines carry offsets, so a text string is created only for a
//! requested content projection. Content/file lookups take O(log slot lines).

mod admission;
mod boundary;
mod splice;

pub use admission::{
    line_quote_depth, parse_input_depth_within_limit, quote_depth_within, PAGE_MAX_BYTES,
    SOURCE_DEPTH_MAX, SOURCE_QUOTE_DEPTH_MAX,
};
pub use boundary::{headers, is_unbulleted_heading_line, Format, Header, OutlineRefusal};
pub use splice::{apply, Splice, SpliceError};

use std::{borrow::Cow, num::NonZeroU32, ops::Range, str::Utf8Error};

/// Failure at file admission. A parser outline fallback is carried by
/// [`PageText::outline_refusal`] because the existing file reader can show it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadError {
    TooLarge { len: u64 },
    Encoding(Utf8Error),
    SourceDepth,
    OrgHeadlineDepth,
}

/// A physical break, or the end of a piece that ends within a line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BreakKind {
    Lf,
    CrLf,
    Cr,
    Eof,
    Boundary,
}

/// The byte pieces of one file line or a fragment at a mid-line header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Line {
    start: usize,
    text_start: usize,
    text_end: usize,
    end: usize,
    content_at: usize,
}

impl Line {
    fn new(start: usize, text_end: usize, end: usize) -> Self {
        Self {
            start,
            text_start: start,
            text_end,
            end,
            content_at: 0,
        }
    }

    pub fn layout(self) -> Range<usize> {
        self.start..self.text_start
    }

    pub fn text(self) -> Range<usize> {
        self.text_start..self.text_end
    }

    pub fn line_break(self) -> Range<usize> {
        self.text_end..self.end
    }

    pub fn break_kind(self, page: &PageText<'_>) -> BreakKind {
        match &page.source[self.line_break()] {
            "\n" => BreakKind::Lf,
            "\r\n" => BreakKind::CrLf,
            "\r" => BreakKind::Cr,
            _ if self.end == page.source.len() => BreakKind::Eof,
            _ => BreakKind::Boundary,
        }
    }

    pub fn content_offset(self) -> usize {
        self.content_at
    }
}

struct Slot {
    start: usize,
    content_end: usize,
    end: usize,
    content_indent: usize,
    level: Option<NonZeroU32>,
    missing_separator: bool,
}

/// One block's physical lines and content coordinates. An opaque slot
/// preserves bytes as preamble and offers only its trailer.
#[derive(Clone, Copy)]
pub struct SlotText<'p> {
    source: &'p str,
    lines: &'p [Line],
    trailer: &'p [Line],
    slot: &'p Slot,
}

impl<'p> SlotText<'p> {
    pub fn level(self) -> Option<u32> {
        self.slot.level.map(NonZeroU32::get)
    }

    pub fn is_opaque(self) -> bool {
        self.slot.level.is_none()
    }

    pub fn lines(self) -> &'p [Line] {
        self.lines
    }

    pub fn trailer(self) -> &'p [Line] {
        self.trailer
    }

    pub fn content_len(self) -> usize {
        self.lines
            .last()
            .map_or(0, |line| line.content_at + line.text().len())
    }

    pub fn content(self) -> String {
        join_content(
            self.lines.iter().map(|line| &self.source[line.text()]),
            self.content_len(),
        )
    }

    /// The native coordinate, including at content end. An internal LF maps
    /// to the first byte of its native break. The empty opaque projection maps
    /// offset zero to the start of its preserved region.
    pub fn file_offset(self, content: usize) -> Option<usize> {
        if self.is_opaque() {
            return (content == 0).then(|| self.trailer[0].start);
        }
        self.line_at_content(content)
            .map(|line| line.text_start + content - line.content_at)
    }

    /// The content coordinate of a native position in the slot. Layout maps
    /// to the start of its text, break bytes to its end, and trailer to EOF.
    pub fn content_offset(self, native: usize) -> Option<usize> {
        if self.is_opaque() {
            return (self.trailer[0].start..=self.trailer.last()?.end)
                .contains(&native)
                .then_some(0);
        }
        let first = self.lines.first()?;
        let last = self.trailer.last().or_else(|| self.lines.last())?;
        if native < first.start || native > last.end {
            return None;
        }
        let index = self.lines.partition_point(|line| line.start <= native) - 1;
        let line = self.lines[index];
        Some(line.content_at + native.clamp(line.text_start, line.text_end) - line.text_start)
    }

    fn line_at_content(self, at: usize) -> Option<Line> {
        if self.lines.is_empty() || at > self.content_len() {
            return None;
        }
        Some(self.lines[self.lines.partition_point(|line| line.content_at <= at) - 1])
    }
}

/// Project blocks from already-located headers and LF line slices at the
/// existing document boundary. `removed` records each block line's layout
/// for the native source map. O(bytes visited), with no additional parse.
pub fn block_contents<'a>(
    lines: &'a [&'a str],
    headers: &'a [Header],
    mut removed: Option<&'a mut [usize]>,
) -> impl Iterator<Item = (u32, String)> + 'a {
    headers.iter().enumerate().map(move |(index, header)| {
        let start = header.line;
        let end = headers.get(index + 1).map_or(lines.len(), |next| next.line);
        let content = lines[start..end].iter().enumerate().map(|(i, line)| {
            let layout = if i == 0 {
                header.prefix_len
            } else {
                continuation_layout_len(line, header.content_indent)
            };
            if let Some(removed) = removed.as_deref_mut() {
                removed[start + i] = layout;
            }
            &line[layout..]
        });
        (header.level, join_content(content, 0))
    })
}

fn continuation_layout_len(line: &str, indent: usize) -> usize {
    line.len() - strip_layout_ws(line, indent).len()
}

fn join_content<'a>(mut lines: impl Iterator<Item = &'a str>, capacity: usize) -> String {
    let Some(first) = lines.next() else {
        return String::new();
    };
    let mut raw = if capacity > first.len() {
        let mut raw = String::with_capacity(capacity);
        raw.push_str(first);
        raw
    } else {
        first.to_owned()
    };
    for line in lines {
        raw.push('\n');
        raw.push_str(line);
    }
    raw
}

/// Borrowed source and its physical piece table. Each slot owns a consecutive
/// range of pieces. Compatibility methods expose the current document reader's
/// projection of the same source and parser events.
pub struct PageText<'a> {
    source: &'a str,
    lines: Vec<Line>,
    slots: Vec<Slot>,
    preamble_end: usize,
    refusal: Option<OutlineRefusal>,
}

impl<'a> PageText<'a> {
    /// Read a file under Tine's existing size, UTF-8 and source-depth limits.
    pub fn read(bytes: &'a [u8], format: Format) -> Result<Self, ReadError> {
        if bytes.len() as u64 > PAGE_MAX_BYTES {
            return Err(ReadError::TooLarge {
                len: bytes.len() as u64,
            });
        }
        let source = std::str::from_utf8(bytes).map_err(ReadError::Encoding)?;
        if !parse_input_depth_within_limit(source) {
            return Err(ReadError::SourceDepth);
        }
        let page = Self::from_text(source, format);
        if format == Format::Org && page.headers().any(|h| h.level as usize > SOURCE_DEPTH_MAX) {
            return Err(ReadError::OrgHeadlineDepth);
        }
        Ok(page)
    }

    fn from_text(source: &'a str, format: Format) -> Self {
        let outline = boundary::locate_headers(&normalize_line_endings(source), format);
        let (located, mut refusal, opaque) = match outline {
            Ok(headers) => (headers, None, false),
            Err(reason) => (
                Vec::new(),
                Some(reason),
                matches!(reason, OutlineRefusal::NotOnePerLine { .. }),
            ),
        };
        let mut lines = Vec::with_capacity(located.len());
        let mut slots = Vec::with_capacity(located.len());
        let mut headers = located.into_iter().enumerate().peekable();
        let mut at = 0;
        for (i, physical) in physical_lines(source).enumerate() {
            let end = at + physical.len();
            let text_end = end - line_terminator(physical).len();
            let mut prefix_len = 0;
            if let Some((event, found)) = headers.next_if(|(_, h)| h.header.line == i) {
                if found.suffix > 0 {
                    refusal.get_or_insert(OutlineRefusal::NotOnePerLine { event });
                    lines.push(Line::new(at, at + found.suffix, at + found.suffix));
                    at += found.suffix;
                }
                prefix_len = found.header.prefix_len;
                slots.push(Slot {
                    start: lines.len(),
                    content_end: 0,
                    end: 0,
                    content_indent: found.header.content_indent,
                    level: NonZeroU32::new(found.header.level),
                    missing_separator: found.missing_separator,
                });
            }
            let mut line = Line::new(at, text_end, end);
            line.text_start += prefix_len;
            lines.push(line);
            at = end;
        }
        let read_lines = match format {
            Format::Org => lines
                .iter()
                .rposition(|line| line.start != line.text_end)
                .map_or(0, |i| i + 1),
            Format::Markdown => lines.len(),
        };
        if opaque {
            slots.push(Slot {
                start: 0,
                content_end: 0,
                end: lines.len(),
                content_indent: 0,
                level: None,
                missing_separator: false,
            });
        }
        let preamble_end = slots.first().map_or(lines.len(), |slot| slot.start);
        let mut end = lines.len();
        for slot in slots.iter_mut().rev() {
            slot.end = end;
            if slot.level.is_some() {
                slot.content_end = end.min(read_lines);
                let (first, rest) = lines[slot.start..slot.content_end]
                    .split_first_mut()
                    .expect("slot includes its header");
                let mut offset = first.text().len() + 1;
                for line in rest {
                    let text = &source[line.text()];
                    line.text_start += continuation_layout_len(text, slot.content_indent);
                    line.content_at = offset;
                    offset += line.text().len() + 1;
                }
            }
            end = slot.start;
        }
        Self {
            source,
            lines,
            slots,
            preamble_end,
            refusal,
        }
    }

    pub fn source(&self) -> &'a str {
        self.source
    }

    pub fn preamble(&self) -> &[Line] {
        &self.lines[..self.preamble_end]
    }

    pub fn slots(&self) -> impl ExactSizeIterator<Item = SlotText<'_>> {
        self.slots.iter().map(|slot| self.slot_text(slot))
    }

    fn slot_text<'p>(&'p self, slot: &'p Slot) -> SlotText<'p> {
        SlotText {
            source: self.source,
            lines: &self.lines[slot.start..slot.content_end],
            trailer: &self.lines[slot.content_end..slot.end],
            slot,
        }
    }

    pub fn outline_refusal(&self) -> Option<OutlineRefusal> {
        self.refusal
    }

    /// Headers that can be projected into the current line-based document.
    pub fn headers(&self) -> impl Iterator<Item = Header> + '_ {
        let slots = if self.refusal.is_none() {
            &self.slots[..]
        } else {
            &[]
        };
        slots.iter().map(|slot| Header {
            line: slot.start,
            level: slot.level.expect("representable outline slot").get(),
            prefix_len: self.lines[slot.start].layout().len(),
            content_indent: slot.content_indent,
        })
    }
}

/// Physical lines including their terminators. This mirrors lsdoc's source
/// scanner: CRLF is one break, LF and bare CR are each one, and only remaining
/// unterminated bytes form an EOF line.
pub fn physical_lines(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        let bytes = rest.as_bytes();
        let end = memchr::memchr2(b'\r', b'\n', bytes).map_or(bytes.len(), |at| {
            at + 1 + usize::from(bytes[at] == b'\r' && bytes.get(at + 1) == Some(&b'\n'))
        });
        let (line, tail) = rest.split_at(end);
        rest = tail;
        (!line.is_empty()).then_some(line)
    })
}

/// The terminator at the end of a physical line.
pub fn line_terminator(line: &str) -> &'static str {
    match line.as_bytes() {
        [.., b'\r', b'\n'] => "\r\n",
        [.., b'\n'] => "\n",
        [.., b'\r'] => "\r",
        _ => "",
    }
}

/// The LF text used by Tine's document reader and its outline parser.
pub fn normalize_line_endings(source: &str) -> Cow<'_, str> {
    if !source.contains('\r') {
        return Cow::Borrowed(source);
    }
    let mut lf = String::with_capacity(source.len());
    for line in physical_lines(source) {
        let terminator = line_terminator(line);
        lf.push_str(&line[..line.len() - terminator.len()]);
        if !terminator.is_empty() {
            lf.push('\n');
        }
    }
    Cow::Owned(lf)
}

/// Remove up to `count` bytes of mldoc layout whitespace from a continuation.
pub fn strip_layout_ws(text: &str, count: usize) -> &str {
    let skip = text
        .bytes()
        .take(count)
        .take_while(|b| matches!(b, b' ' | b'\t' | 0x0c))
        .count();
    &text[skip..]
}
