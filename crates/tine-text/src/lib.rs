//! Block text, the physical layer (area 1, `specs/areas/block-text.md`): which
//! bytes of a page file belong to which block, as layout (structural prefix or
//! removed indent), text or line break, so that the pieces concatenate to the
//! file (R0) and a write replaces only named byte ranges (R3).
//!
//! ```text
//! page  = preamble-piece* slot* tail
//! slot  = piece+          with a level; the first piece carries the block's structural prefix
//! piece = layout text break
//! ```
//!
//! lsdoc decides where a slot starts ([`bound::parse_outline_bounded`]); this
//! crate owns no Markdown or Org grammar. Lines end where lsdoc's source
//! scanner ends them (`lsdoc/src/v2/source.rs`, `SourceScanner::scan`: LF, CRLF
//! and every other CR). A slot's content is its text pieces joined by `"\n"`
//! ([`PageText::content`]), with a total map to file bytes ([`ContentMap`]).
//!
//! Cost of [`PageText::read`]: one outline parse of the page plus O(bytes);
//! one record per physical line and per slot, no text copied (the outline is
//! parsed from an LF copy only when the file has a CR).

pub mod bound;
mod splice;

use std::ops::Range;

pub use splice::{apply, Splice, SpliceError};

/// Which outline grammar the page is read with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Format {
    Markdown,
    Org,
}

/// Why a file has no physical layer: the page reader shows such a file as its
/// unparsed text (tine-core `doc::parse`), as develop does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// The bytes are not UTF-8 (the store's `NotUtf8`).
    NotUtf8,
    /// lsdoc did not take ownership of the text, or its quote staircase is
    /// deeper than [`bound::SOURCE_QUOTE_DEPTH_MAX`].
    Unowned,
    /// Outline event `event` has ranges, kind or level outside the parser's
    /// contract.
    Invalid { event: usize },
}

/// How a piece ends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Break {
    Lf,
    CrLf,
    /// A CR not followed by LF: lsdoc's scanner ends a line there.
    Cr,
    /// The end of the file.
    Eof,
    /// The physical line goes on in the next slot, whose outline header starts
    /// inside it (an empty break).
    Continues,
}

impl Break {
    /// The bytes of this break; empty for [`Break::Eof`] and [`Break::Continues`].
    pub fn as_str(self) -> &'static str {
        match self {
            Break::Lf => "\n",
            Break::CrLf => "\r\n",
            Break::Cr => "\r",
            Break::Eof | Break::Continues => "",
        }
    }
}

/// One line piece: three adjacent byte ranges of the file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Piece {
    /// Bytes in front of the text that are structure, not content: the slot's
    /// structural prefix on its first piece, the removed indent on the others.
    pub layout: Range<usize>,
    pub text: Range<usize>,
    pub brk: Range<usize>,
    pub kind: Break,
}

impl Piece {
    /// All bytes of the piece.
    pub fn range(&self) -> Range<usize> {
        self.layout.start..self.brk.end
    }
}

/// The outline header that opens a slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeaderKind {
    /// `# Heading`, the whole line is content.
    MarkdownHeading,
    /// `- text`, continuation lines dedent to the bullet's content column.
    MarkdownBullet,
    /// `** text`.
    OrgHeadline,
}

/// One block's bytes: from the first byte of its outline header to the next
/// slot (or the tail).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Slot {
    start: usize,
    text_start: usize,
    line: usize,
    level: u32,
    kind: HeaderKind,
    dedent: usize,
    opaque: bool,
}

impl Slot {
    /// lsdoc's structural level: a slot nests under the nearest earlier slot
    /// of a smaller level.
    pub fn level(&self) -> u32 {
        self.level
    }
    pub fn kind(&self) -> HeaderKind {
        self.kind
    }
    /// The file byte the slot starts at (its outline header).
    pub fn start(&self) -> usize {
        self.start
    }
    /// Index of the physical line the slot starts on.
    pub fn line(&self) -> usize {
        self.line
    }
    /// Leading layout bytes (tabs, spaces, form feeds) removed from each of the
    /// slot's later lines, at most: the bullet's content column, else 0.
    pub fn dedent(&self) -> usize {
        self.dedent
    }
    /// Whether two outline headers fell into this slot because lsdoc reported
    /// them without disjoint byte ranges; its content is text only.
    pub fn opaque(&self) -> bool {
        self.opaque
    }
}

/// Who owns a run of pieces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Owner {
    Preamble,
    Slot(usize),
}

/// One physical line: `start..layout_end` is the layout of whoever owns the
/// line's first byte, `text_end..end` its break.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Line {
    start: usize,
    layout_end: usize,
    text_end: usize,
    end: usize,
}

/// The physical layer of one page file.
#[derive(Clone, Debug)]
pub struct PageText<'a> {
    text: &'a str,
    format: Format,
    lines: Vec<Line>,
    slots: Vec<Slot>,
    /// First physical line of the tail: lines the reader does not read (Org's
    /// final empty lines; a Markdown file that is one empty line).
    tail: usize,
}

/// `text` with every break (CRLF, LF, CR) written as one `"\n"`: what lsdoc
/// reads the outline from, and what a slot's content joins lines with. Line
/// indices and columns are the same in both. O(bytes), a copy only when
/// `text` has a CR.
pub fn lf_form(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('\r') {
        std::borrow::Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

impl<'a> PageText<'a> {
    /// Read `bytes` as a page in `format`. One bounded outline parse, O(bytes).
    pub fn read(bytes: &'a [u8], format: Format) -> Result<Self, Refusal> {
        let text = std::str::from_utf8(bytes).map_err(|_| Refusal::NotUtf8)?;
        Self::read_text(text, format)
    }

    /// [`Self::read`] of text already known to be UTF-8.
    pub fn read_text(text: &'a str, format: Format) -> Result<Self, Refusal> {
        Self::read_lf(text, &lf_form(text), format)
    }

    /// [`Self::read_text`] for a caller that holds `lf`, the [`lf_form`] of
    /// `text`: the outline is read from it, as develop's reader reads it.
    pub fn read_lf(text: &'a str, lf: &str, format: Format) -> Result<Self, Refusal> {
        let events =
            bound::parse_outline_bounded(lf, format == Format::Org).ok_or(Refusal::Unowned)?;
        Self::outlined(text, format, lf, &events)
    }

    /// The physical layer of `text` from lsdoc's outline `events` of its LF
    /// form `lf`. O(bytes + events).
    fn outlined(
        text: &'a str,
        format: Format,
        lf: &str,
        events: &[lsdoc::OutlineHeader],
    ) -> Result<Self, Refusal> {
        let mut page = Self::preamble_only(text, format);
        // Events come in source order: walk the lines once, with each line's
        // start in `lf`.
        let (mut line, mut lf_start) = (0, 0);
        for (event, header) in events.iter().enumerate() {
            while line < page.lines.len() && lf_start < header.line.start {
                let l = page.lines[line];
                lf_start += l.text_end - l.start + usize::from(l.end > l.text_end);
                line += 1;
            }
            let slot = (line < page.lines.len() && lf_start == header.line.start)
                .then(|| page.slot_of(lf, line, header))
                .flatten()
                .ok_or(Refusal::Invalid { event })?;
            match page.slots.last_mut() {
                // A header inside the previous slot's structural prefix (or
                // before it) cannot have its own bytes.
                Some(last) if slot.start < last.text_start => last.opaque = true,
                _ => page.slots.push(slot),
            }
        }
        page.lay_out();
        Ok(page)
    }

    /// `text` with no slots: every line is preamble (develop's reading of a
    /// page whose outline it refuses). O(bytes).
    pub fn preamble_only(text: &'a str, format: Format) -> Self {
        let mut lines = Vec::new();
        let bytes = text.as_bytes();
        let mut start = 0;
        while start < bytes.len() {
            let text_end =
                memchr::memchr2(b'\n', b'\r', &bytes[start..]).map_or(bytes.len(), |at| start + at);
            let end = match bytes.get(text_end) {
                None => text_end,
                Some(b'\r') if bytes.get(text_end + 1) == Some(&b'\n') => text_end + 2,
                Some(_) => text_end + 1,
            };
            lines.push(Line {
                start,
                layout_end: start,
                text_end,
                end,
            });
            start = end;
        }
        let empty = |line: &Line| line.text_end == line.start;
        let tail = match format {
            Format::Org => lines.len() - lines.iter().rev().take_while(|l| empty(l)).count(),
            Format::Markdown if lines.len() == 1 && empty(&lines[0]) => 0,
            Format::Markdown => lines.len(),
        };
        Self {
            text,
            format,
            lines,
            slots: Vec::new(),
            tail,
        }
    }

    /// The slot an outline event on physical line `line` opens, or `None`
    /// when the event is outside lsdoc's outline contract.
    fn slot_of(&self, lf: &str, line: usize, header: &lsdoc::OutlineHeader) -> Option<Slot> {
        let prefix = header.structural_prefix;
        let kind = match (self.format, header.kind) {
            (Format::Markdown, lsdoc::OutlineHeaderKind::MarkdownUnbulletedAtxHeading) => {
                HeaderKind::MarkdownHeading
            }
            (Format::Markdown, lsdoc::OutlineHeaderKind::MarkdownDashBullet) => {
                HeaderKind::MarkdownBullet
            }
            (Format::Org, lsdoc::OutlineHeaderKind::OrgHeadline) => HeaderKind::OrgHeadline,
            _ => return None,
        };
        let line_start = header.line.start;
        let valid = header.level > 0
            && header.line_content.slice(lf).is_some()
            && prefix.slice(lf).is_some()
            && prefix.start == line_start
            && prefix.end <= header.line_content.end
            && header.line_content.start == line_start
            && (line_start..=header.line_content.end).contains(&header.header_start);
        if !valid {
            return None;
        }
        let native = self.lines[line].start;
        let prefix_end = native + prefix.end - line_start;
        let start = native + header.header_start - line_start;
        let dedent = match kind {
            HeaderKind::MarkdownBullet if lf[..prefix.end].ends_with("- ") => {
                prefix.end - prefix.start
            }
            HeaderKind::MarkdownBullet => prefix.end - prefix.start + 1,
            _ => 0,
        };
        Some(Slot {
            start,
            text_start: start.max(prefix_end),
            line,
            level: header.level,
            kind,
            dedent,
            opaque: false,
        })
    }

    /// Fill each line's layout for the owner of its first byte. O(bytes).
    fn lay_out(&mut self) {
        for n in 0..self.slots.len() {
            let slot = &self.slots[n];
            let (first, dedent) = (slot.line, slot.dedent);
            if slot.start == self.lines[first].start {
                self.lines[first].layout_end = slot.text_start;
            }
            let end = self.slots.get(n + 1).map_or(self.tail, |next| {
                next.line + usize::from(next.start > self.lines[next.line].start)
            });
            for line in &mut self.lines[(first + 1).min(end)..end] {
                let ws = self.text.as_bytes()[line.start..line.text_end]
                    .iter()
                    .take(dedent)
                    .take_while(|b| matches!(b, b' ' | b'\t' | 0x0c))
                    .count();
                line.layout_end = line.start + ws;
            }
        }
    }

    /// The file's text.
    pub fn text(&self) -> &'a str {
        self.text
    }
    pub fn format(&self) -> Format {
        self.format
    }
    /// The slots in source order.
    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }
    /// Whether slot `n` starts at the start of a physical line.
    pub fn opens_line(&self, n: usize) -> bool {
        let slot = &self.slots[n];
        slot.start == self.lines[slot.line].start
    }
    /// Number of physical lines.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }
    /// Physical line `i`: its bytes, terminator included, and where its break
    /// starts.
    pub fn line(&self, i: usize) -> (Range<usize>, usize) {
        let line = self.lines[i];
        (line.start..line.end, line.text_end)
    }
    /// Layout bytes in front of physical line `i` (for the owner of its first byte).
    pub fn line_layout(&self, i: usize) -> usize {
        self.lines[i].layout_end - self.lines[i].start
    }
    /// The physical lines the reader reads (before the tail).
    pub fn read_lines(&self) -> usize {
        self.tail
    }
    /// The tail's bytes: lines the reader does not read.
    pub fn tail(&self) -> Range<usize> {
        self.lines
            .get(self.tail)
            .map_or(self.text.len(), |l| l.start)..self.text.len()
    }

    /// File bytes owned by `owner`.
    pub fn range(&self, owner: Owner) -> Range<usize> {
        let start = |n: usize| self.slots.get(n).map(|s| s.start);
        let end = self.tail().start;
        match owner {
            Owner::Preamble => 0..start(0).unwrap_or(end),
            Owner::Slot(n) => self.slots[n].start..start(n + 1).unwrap_or(end),
        }
    }

    /// The pieces of `owner`, in order. O(its lines) to walk, no allocation.
    pub fn pieces(&self, owner: Owner) -> impl Iterator<Item = Piece> + '_ {
        let range = self.range(owner);
        let first = match owner {
            Owner::Preamble => 0,
            Owner::Slot(n) => self.slots[n].line,
        };
        let text_start = match owner {
            Owner::Slot(n) => self.slots[n].text_start,
            Owner::Preamble => 0,
        };
        let lines = self.lines[first..self.tail].iter();
        lines
            .take_while(move |line| line.start < range.end)
            .enumerate()
            .map(move |(i, line)| {
                let start = line.start.max(range.start);
                let layout_end = if i == 0 && owner != Owner::Preamble {
                    text_start
                } else {
                    line.layout_end.min(range.end)
                };
                let (text_end, brk_end, kind) = if line.end > range.end {
                    (range.end, range.end, Break::Continues)
                } else {
                    let kind = match &self.text.as_bytes()[line.text_end..line.end] {
                        b"\n" => Break::Lf,
                        b"\r\n" => Break::CrLf,
                        b"\r" => Break::Cr,
                        _ => Break::Eof,
                    };
                    (line.text_end, line.end, kind)
                };
                Piece {
                    layout: start..layout_end,
                    text: layout_end..text_end,
                    brk: text_end..brk_end,
                    kind,
                }
            })
    }

    /// Slot `n`'s content: its text pieces joined by `"\n"`. O(its bytes).
    pub fn content(&self, n: usize) -> String {
        let mut out = String::with_capacity(self.range(Owner::Slot(n)).len());
        for (i, piece) in self.pieces(Owner::Slot(n)).enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&self.text[piece.text]);
        }
        out
    }

    /// The map between slot `n`'s content offsets and file bytes. O(its lines).
    pub fn content_map(&self, n: usize) -> ContentMap {
        let mut lines = Vec::new();
        let mut at = 0;
        for piece in self.pieces(Owner::Slot(n)) {
            let len = piece.text.len();
            lines.push((at, piece));
            at += len + 1;
        }
        ContentMap { lines }
    }
}

/// A total map between a slot's content offsets and file bytes: every content
/// offset has one file byte; a file byte of the slot that is layout maps to
/// the start of the following text, one that is break to the end of the
/// preceding text.
#[derive(Clone, Debug)]
pub struct ContentMap {
    /// Per piece: its content offset, and the piece.
    lines: Vec<(usize, Piece)>,
}

impl ContentMap {
    /// Content length.
    pub fn len(&self) -> usize {
        self.lines
            .last()
            .map_or(0, |(at, piece)| at + piece.text.len())
    }
    /// Whether the content is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The pieces, each with its content offset.
    pub fn pieces(&self) -> &[(usize, Piece)] {
        &self.lines
    }
    /// The piece holding content offset `at` (a separator belongs to the
    /// piece it ends). O(log lines).
    pub fn piece_at(&self, at: usize) -> usize {
        self.lines
            .partition_point(|(start, _)| *start <= at)
            .saturating_sub(1)
    }
    /// File byte of content offset `at` (`at <= len`). O(log lines).
    pub fn to_file(&self, at: usize) -> usize {
        let (start, piece) = &self.lines[self.piece_at(at)];
        piece.text.start + (at - start).min(piece.text.len())
    }
    /// Content offset of file byte `at` inside the slot (or its end). O(log lines).
    pub fn to_content(&self, at: usize) -> usize {
        let i = self
            .lines
            .partition_point(|(_, piece)| piece.layout.start <= at)
            .saturating_sub(1);
        let (start, piece) = &self.lines[i];
        start + at.clamp(piece.text.start, piece.text.end) - piece.text.start
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsdoc::{OutlineHeader, OutlineHeaderKind, SourceRange};

    fn header(level: u32, header_start: usize, prefix: Range<usize>) -> OutlineHeader {
        OutlineHeader {
            kind: OutlineHeaderKind::MarkdownDashBullet,
            level,
            header_start,
            structural_prefix: SourceRange {
                start: prefix.start,
                end: prefix.end,
            },
            line_content: SourceRange { start: 0, end: 5 },
            line: SourceRange { start: 0, end: 6 },
        }
    }

    /// lsdoc v0.5.13 gives every header disjoint bytes; a header reported
    /// inside an earlier one's structural prefix makes that slot opaque, and
    /// the pieces still concatenate to the file.
    #[test]
    fn a_header_inside_the_previous_prefix_makes_one_opaque_slot() {
        let text = "- - a\nb";
        let events = [header(1, 0, 0..2), header(2, 1, 0..4)];
        let page = PageText::outlined(text, Format::Markdown, text, &events).unwrap();
        assert_eq!(page.slots().len(), 1);
        assert!(page.slots()[0].opaque());
        assert_eq!(page.content(0), "- a\nb");
        let all: String = page
            .pieces(Owner::Slot(0))
            .map(|p| &text[p.range()])
            .collect();
        assert_eq!(all, text);
    }

    #[test]
    fn an_event_off_its_line_is_invalid() {
        let text = "- a\n";
        let mut event = header(1, 0, 0..2);
        event.line = SourceRange { start: 1, end: 4 };
        let refusal = PageText::outlined(text, Format::Markdown, text, &[event]).unwrap_err();
        assert_eq!(refusal, Refusal::Invalid { event: 0 });
    }
}
