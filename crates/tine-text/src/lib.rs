//! Block text, the physical layer: which bytes of a page file belong to which
//! block, as layout (structural prefix or removed indent), text or line break,
//! so that the pieces concatenate to the file (R0) and a write replaces only
//! named byte ranges (R3).
//!
//! ```text
//! page  = preamble slot*                      the owners, in file order
//! owner = piece* trailer-piece*               only the last owner has a trailer
//! piece = layout text break
//! ```
//!
//! lsdoc decides where a slot starts ([`bound::parse_outline_bounded`]); this
//! crate owns no Markdown or Org grammar. Lines end where lsdoc's source
//! scanner ends them ([`lines`]: LF, CRLF and every other CR). A slot's content
//! is its text pieces joined by `"\n"` ([`PageText::content`]), with a map to
//! file bytes ([`ContentMap`]). The physical lines after the last owner's
//! content (an Org file's final empty lines; a Markdown file that is one empty
//! line) are that owner's trailer: its bytes, not its content.
//!
//! Cost of [`PageText::read`]: one outline parse of the page plus O(bytes);
//! one record per physical line and per slot, no text copied (the outline is
//! parsed from an LF copy only when the file has a CR).

pub mod bound;
mod splice;

use std::ops::Range;

pub use splice::{apply, check_planned_rows, Splice, SpliceError};

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

/// The physical lines of `text`, in order: each line's bytes before its
/// break, and the break ([`Break::Eof`] for an unterminated last line). An
/// empty text has no line and a final break opens none, so the lines and
/// their breaks concatenate to `text`. The one place a line's end is decided.
/// O(bytes), no copy.
pub fn lines(text: &str) -> impl Iterator<Item = (Range<usize>, Break)> + '_ {
    let bytes = text.as_bytes();
    let mut start = 0;
    std::iter::from_fn(move || {
        if start >= bytes.len() {
            return None;
        }
        let text_end =
            memchr::memchr2(b'\n', b'\r', &bytes[start..]).map_or(bytes.len(), |at| start + at);
        let (end, kind) = match bytes.get(text_end) {
            None => (text_end, Break::Eof),
            Some(b'\r') if bytes.get(text_end + 1) == Some(&b'\n') => (text_end + 2, Break::CrLf),
            Some(b'\r') => (text_end + 1, Break::Cr),
            Some(_) => (text_end + 1, Break::Lf),
        };
        let line = start..text_end;
        start = end;
        Some((line, kind))
    })
}

/// How many leading bytes of `line` the reader removes as layout when it
/// dedents by up to `dedent` bytes: spaces, tabs and form feeds (mldoc's
/// layout whitespace). The one answer for a continuation line's removed
/// indent, a header line's indentation (`usize::MAX`) and the header layout a
/// new line reuses. O(min(dedent, line)).
pub fn removed_indent(line: &[u8], dedent: usize) -> usize {
    line.iter()
        .take(dedent)
        .take_while(|b| matches!(b, b' ' | b'\t' | 0x0c))
        .count()
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
/// slot (or the end of the file).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Slot {
    start: usize,
    text_start: usize,
    line: usize,
    level: u32,
    kind: HeaderKind,
    dedent: usize,
    indent: usize,
    lead: usize,
    bare: bool,
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
    /// Leading layout bytes of the slot's header line ([`removed_indent`]
    /// with no bound).
    pub fn indent(&self) -> usize {
        self.indent
    }
    /// Whether the header's structural prefix ends at its marker (`-`, `**`,
    /// `-<tab>Title`, `-# Heading`): lsdoc's `OutlineHeader::structural_prefix`
    /// (`src/outline.rs`) takes one ASCII space after the marker when there is
    /// one, so a bare header's content starts right after the marker. An
    /// unbulleted Markdown heading has an empty prefix and is never bare.
    pub fn bare(&self) -> bool {
        self.bare
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
/// line's first byte, `text_end..end` its break; `at` is the line's offset in
/// that owner's content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Line {
    start: usize,
    layout_end: usize,
    text_end: usize,
    end: usize,
    at: usize,
    kind: Break,
}

/// The physical layer of one page file without its text: every answer about
/// which byte is what, by offset. [`PageText`] pairs it with the text; a
/// caller that keeps the answers after the text is gone keeps this.
#[derive(Clone, Debug)]
pub struct Layout {
    format: Format,
    len: usize,
    lines: Vec<Line>,
    slots: Vec<Slot>,
    /// First trailer line: the last owner's lines after its content.
    tail: usize,
}

/// The physical layer of one page file and its text.
#[derive(Clone, Debug)]
pub struct PageText<'a> {
    text: &'a str,
    layout: Layout,
}

impl std::ops::Deref for PageText<'_> {
    type Target = Layout;
    fn deref(&self) -> &Layout {
        &self.layout
    }
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
            match page.layout.slots.last_mut() {
                // A header inside the previous slot's structural prefix (or
                // before it) cannot have its own bytes.
                Some(last) if slot.start < last.text_start => last.opaque = true,
                _ => page.layout.slots.push(slot),
            }
        }
        page.lay_out();
        Ok(page)
    }

    /// `text` with no slots: every line is preamble (develop's reading of a
    /// page whose outline it refuses). O(bytes).
    pub fn preamble_only(text: &'a str, format: Format) -> Self {
        let lines: Vec<Line> = lines(text)
            .map(|(line, kind)| Line {
                start: line.start,
                layout_end: line.start,
                text_end: line.end,
                end: line.end + kind.as_str().len(),
                at: 0,
                kind,
            })
            .collect();
        let empty = |line: &Line| line.text_end == line.start;
        let tail = match format {
            Format::Org => lines.len() - lines.iter().rev().take_while(|l| empty(l)).count(),
            Format::Markdown if lines.len() == 1 && empty(&lines[0]) => 0,
            Format::Markdown => lines.len(),
        };
        let mut page = Self {
            text,
            layout: Layout {
                format,
                len: text.len(),
                lines,
                slots: Vec::new(),
                tail,
            },
        };
        page.layout.number(Owner::Preamble);
        page
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
        let physical = self.lines[line];
        let native = physical.start;
        let prefix_end = native + prefix.end - line_start;
        let start = native + header.header_start - line_start;
        let text_start = start.max(prefix_end);
        let dedent = match kind {
            HeaderKind::MarkdownBullet if lf[..prefix.end].ends_with("- ") => {
                prefix.end - prefix.start
            }
            HeaderKind::MarkdownBullet => prefix.end - prefix.start + 1,
            _ => 0,
        };
        let bytes = self.text.as_bytes();
        Some(Slot {
            start,
            text_start,
            line,
            level: header.level,
            kind,
            dedent,
            indent: removed_indent(&bytes[native..physical.text_end], usize::MAX),
            lead: removed_indent(&bytes[start..text_start], dedent),
            bare: kind != HeaderKind::MarkdownHeading
                && start < text_start
                && bytes[text_start - 1] != b' ',
            opaque: false,
        })
    }

    /// Fill each line's layout for the owner of its first byte, then number
    /// every owner's lines. O(bytes).
    fn lay_out(&mut self) {
        let bytes = self.text.as_bytes();
        let layout = &mut self.layout;
        for n in 0..layout.slots.len() {
            let slot = layout.slots[n];
            if slot.start == layout.lines[slot.line].start {
                layout.lines[slot.line].layout_end = slot.text_start;
            }
            for i in layout.content_lines(Owner::Slot(n)).skip(1) {
                let line = &mut layout.lines[i];
                line.layout_end =
                    line.start + removed_indent(&bytes[line.start..line.text_end], slot.dedent);
            }
            layout.number(Owner::Slot(n));
        }
    }

    /// The file's text.
    pub fn text(&self) -> &'a str {
        self.text
    }

    /// The physical layer without the text.
    pub fn into_layout(self) -> Layout {
        self.layout
    }

    /// The layout in front of a line a write adds to slot `n`: the header's
    /// leading layout, then spaces up to the slot's dedent (`"  "` under
    /// `- a`, nothing under `* a`). O(that layout).
    pub fn continuation(&self, n: usize) -> String {
        let slot = &self.slots[n];
        let mut out = String::with_capacity(slot.dedent);
        out.push_str(&self.text[slot.start..slot.start + slot.lead]);
        out.extend(std::iter::repeat_n(' ', slot.dedent - slot.lead));
        out
    }

    /// Whether lsdoc reads slot `n`'s header layout followed by `text` (one
    /// line, no break) as a header of the slot's kind and level whose
    /// structural prefix is that layout ([`bound::head_reads`]). One parse of
    /// one line.
    pub fn head_reads(&self, n: usize, text: &str) -> bool {
        let slot = &self.slots[n];
        let layout = &self.text[slot.start..slot.text_start];
        bound::head_reads(layout, text, slot.kind, slot.level)
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
}

impl Layout {
    /// Give each of `owner`'s content lines its offset in the owner's content.
    /// A slot's first line keeps the offset of the owner before it when the
    /// slot starts inside that line. O(its lines).
    fn number(&mut self, owner: Owner) {
        let lines = self.content_lines(owner);
        let shared = matches!(owner, Owner::Slot(n) if !self.opens_line(n));
        let mut at = 0;
        for i in lines.clone() {
            let len = self.piece(owner, i).text.len();
            if !(shared && i == lines.start) {
                self.lines[i].at = at;
            }
            at += len + 1;
        }
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
    /// The line holding file byte `at`; the line count for the end of the
    /// file. O(log lines).
    pub fn line_at(&self, at: usize) -> usize {
        let before = self.lines.partition_point(|l| l.start <= at);
        (before + usize::from(at >= self.len)).saturating_sub(1)
    }
    /// Layout bytes in front of physical line `i` (for the owner of its first byte).
    pub fn line_layout(&self, i: usize) -> usize {
        self.lines[i].layout_end - self.lines[i].start
    }
    /// Offset of physical line `i`'s text in the content of the owner of its
    /// first byte (0 for a trailer line).
    pub fn line_offset(&self, i: usize) -> usize {
        self.lines[i].at
    }
    /// The physical lines that are some owner's content (before the trailer).
    pub fn read_lines(&self) -> usize {
        self.tail
    }
    /// The first byte of the trailer (the file's length when there is none).
    pub fn read_end(&self) -> usize {
        self.lines.get(self.tail).map_or(self.len, |l| l.start)
    }

    /// File bytes owned by `owner`, its trailer included.
    pub fn range(&self, owner: Owner) -> Range<usize> {
        let start = |n: usize| self.slots.get(n).map(|s| s.start);
        match owner {
            Owner::Preamble => 0..start(0).unwrap_or(self.len),
            Owner::Slot(n) => self.slots[n].start..start(n + 1).unwrap_or(self.len),
        }
    }

    /// The physical lines holding `owner`'s content pieces. O(1).
    fn content_lines(&self, owner: Owner) -> Range<usize> {
        let (first, next) = match owner {
            Owner::Preamble => (0, 0),
            Owner::Slot(n) => (self.slots[n].line, n + 1),
        };
        let end = self.slots.get(next).map_or(self.tail, |next| {
            next.line + usize::from(next.start > self.lines[next.line].start)
        });
        first..end.max(first)
    }

    /// The piece of `owner` on physical line `i`.
    fn piece(&self, owner: Owner, i: usize) -> Piece {
        let line = self.lines[i];
        let range = self.range(owner);
        let start = line.start.max(range.start);
        let layout_end = match owner {
            Owner::Slot(n) if i == self.slots[n].line => self.slots[n].text_start,
            _ => line.layout_end.min(range.end),
        };
        let (text_end, brk_end, kind) = if line.end > range.end {
            (range.end, range.end, Break::Continues)
        } else {
            (line.text_end, line.end, line.kind)
        };
        Piece {
            layout: start..layout_end,
            text: layout_end..text_end,
            brk: text_end..brk_end,
            kind,
        }
    }

    /// The content pieces of `owner`, in order. O(its lines) to walk, no
    /// allocation.
    pub fn pieces(&self, owner: Owner) -> impl Iterator<Item = Piece> + '_ {
        self.content_lines(owner).map(move |i| self.piece(owner, i))
    }

    /// The trailer pieces of `owner`: the physical lines after the last
    /// content line of the file, written back untouched, not content. Only the
    /// last owner (the last slot, else the preamble) has any; their layout is
    /// empty.
    pub fn trailer(&self, owner: Owner) -> impl Iterator<Item = Piece> + '_ {
        let last = match self.slots.len() {
            0 => Owner::Preamble,
            n => Owner::Slot(n - 1),
        };
        let lines = if owner == last {
            self.tail..self.lines.len()
        } else {
            0..0
        };
        self.lines[lines].iter().map(|line| Piece {
            layout: line.start..line.start,
            text: line.start..line.text_end,
            brk: line.text_end..line.end,
            kind: line.kind,
        })
    }

    /// The map between `owner`'s content offsets and file bytes. O(1).
    pub fn content_map(&self, owner: Owner) -> ContentMap<'_> {
        ContentMap {
            layout: self,
            owner,
            lines: self.content_lines(owner),
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Line records [`ContentMap`] has looked at.
    pub(crate) static VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Lines [`bound::head_reads`] has asked lsdoc about.
    pub(crate) static HEAD_QUERIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The map between an owner's content offsets and file bytes: every content
/// offset has one file byte; a file byte of the owner that is layout maps to
/// the start of the following text, one that is break to the end of the
/// preceding text. A view of the [`Layout`]; nothing is built.
#[derive(Clone, Debug)]
pub struct ContentMap<'p> {
    layout: &'p Layout,
    owner: Owner,
    lines: Range<usize>,
}

impl ContentMap<'_> {
    /// Offset of the piece on physical line `i` in the content.
    fn offset(&self, i: usize) -> usize {
        if i == self.lines.start {
            0
        } else {
            self.layout.lines[i].at
        }
    }
    /// The physical line whose piece holds content offset `at` (a separator
    /// belongs to the piece it ends). O(log lines).
    fn line_of(&self, at: usize) -> usize {
        let later = &self.layout.lines[(self.lines.start + 1).min(self.lines.end)..self.lines.end];
        self.lines.start
            + later.partition_point(|line| {
                #[cfg(test)]
                VISITS.with(|n| n.set(n.get() + 1));
                line.at <= at
            })
    }
    /// Content length. O(1).
    pub fn len(&self) -> usize {
        match self.lines.clone().last() {
            Some(i) => self.offset(i) + self.layout.piece(self.owner, i).text.len(),
            None => 0,
        }
    }
    /// Whether the content is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The pieces, each with its content offset.
    pub fn pieces(&self) -> impl Iterator<Item = (usize, Piece)> + '_ {
        let lines = self.lines.clone();
        lines.map(|i| (self.offset(i), self.layout.piece(self.owner, i)))
    }
    /// File byte of content offset `at` (`at <= len`). O(log lines).
    pub fn to_file(&self, at: usize) -> usize {
        if self.lines.is_empty() {
            return self.layout.range(self.owner).start;
        }
        let i = self.line_of(at);
        let piece = self.layout.piece(self.owner, i);
        piece.text.start + (at - self.offset(i)).min(piece.text.len())
    }
    /// Content offset of the end of the text of the line holding content
    /// offset `at` (`at` itself when it is that line's separator). O(log
    /// lines).
    pub(crate) fn line_end(&self, at: usize) -> usize {
        if self.lines.is_empty() {
            return 0;
        }
        let i = self.line_of(at);
        self.offset(i) + self.layout.piece(self.owner, i).text.len()
    }
    /// How the piece holding content offset `at` ends. O(log lines).
    pub fn break_at(&self, at: usize) -> Break {
        if self.lines.is_empty() {
            return Break::Eof;
        }
        self.layout.piece(self.owner, self.line_of(at)).kind
    }
    /// How the last content piece ends: [`Break::Eof`] or
    /// [`Break::Continues`] when the owner's slice ends without a break.
    /// O(1).
    pub fn final_break(&self) -> Break {
        match self.lines.clone().last() {
            Some(i) => self.layout.piece(self.owner, i).kind,
            None => Break::Eof,
        }
    }
    /// Content offset of file byte `at` inside the owner (or its end).
    /// O(log lines).
    pub fn to_content(&self, at: usize) -> usize {
        if self.lines.is_empty() {
            return 0;
        }
        let later = &self.layout.lines[self.lines.start + 1..self.lines.end];
        let i = self.lines.start + later.partition_point(|line| line.start <= at);
        let piece = self.layout.piece(self.owner, i);
        self.offset(i) + at.clamp(piece.text.start, piece.text.end) - piece.text.start
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
