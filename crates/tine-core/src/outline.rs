//! The page-outline authority: which physical lines of a Markdown or Org page
//! open a block, and at which structural level.
//!
//! The physical layer of the page is `tine_text::PageText` (lsdoc's outline
//! read once, under the bound of `tine_text::bound`): which bytes are which
//! block's layout, text and breaks. This module adds develop's reading on top:
//! a page whose outline does not give each block its own line has no blocks
//! (the whole text is the pre-block), and it builds the block forest and the
//! reader's [`SourceMap`] from the slots. `doc::parse` and `org::parse_org`
//! build every [`Document`](crate::doc::Document) from [`page`]; nothing else
//! decides where a block starts (I-12).
//!
//! Cost: one lsdoc outline parse of the page, O(page bytes), plus O(lines).

use crate::doc::DocBlock;
use tine_text::{Format, Owner, PageText};

#[cfg(feature = "test-counters")]
thread_local! {
    pub(crate) static READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OutlineFormat {
    Markdown,
    Org,
}

impl OutlineFormat {
    pub(crate) fn text(self) -> Format {
        match self {
            Self::Markdown => Format::Markdown,
            Self::Org => Format::Org,
        }
    }
}

/// One block-opening line of a page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Header {
    /// Index of the header's physical line in `text.split('\n')`.
    pub(crate) line: usize,
    /// lsdoc's structural level: a later header nests under the nearest
    /// earlier one of a smaller level.
    pub(crate) level: u32,
    /// Leading bytes of the header line that are structure, not block text:
    /// indentation plus `-` (and one space) for a dash bullet, the star run
    /// (and one space) for an Org headline, nothing for an unbulleted heading.
    pub(crate) prefix_len: usize,
    /// Leading tabs/spaces the block's continuation lines are dedented by:
    /// the bullet's content column for a Markdown dash bullet, else 0.
    pub(crate) content_indent: usize,
}

/// Where the reader took its answer from (`doc::parse_outlined`): the
/// physical layer it read ([`tine_text::Layout`]) and how many of its lines
/// are the pre-block. Every offset is the layer's own answer; this adds only
/// the reader's names for it. LF, CRLF and lone-CR lines share line indices,
/// so offsets in the reader's LF text map to native bytes by line.
#[derive(Debug)]
pub struct SourceMap {
    page: tine_text::Layout,
    /// Lines of the pre-block.
    pre_block: usize,
}

impl SourceMap {
    /// The map of `page` as the reader read it, with `pre_block` lines of
    /// pre-block. O(1).
    pub(crate) fn new(page: PageText<'_>, pre_block: usize) -> Self {
        Self {
            page: page.into_layout(),
            pre_block,
        }
    }

    /// Physical line `i`, terminator included.
    pub fn line(&self, i: usize) -> std::ops::Range<usize> {
        self.page.line(i).0
    }
    /// The physical lines, in order.
    pub fn lines(&self) -> std::ops::Range<usize> {
        0..self.page.line_count()
    }
    /// The line holding native byte `at` (the line count for the text end).
    pub fn line_at(&self, at: usize) -> usize {
        self.page.line_at(at)
    }
    /// Native end of physical line `i` before its terminator.
    pub fn end(&self, i: usize) -> usize {
        self.page.line(i).1
    }
    /// The bytes of line `i` the reader kept: after what it removed, before
    /// the terminator (a line it did not read is all kept).
    pub fn payload(&self, i: usize) -> std::ops::Range<usize> {
        self.line(i).start + self.removed(i)..self.end(i)
    }
    /// Bytes the reader removed in front of line `i`.
    pub fn removed(&self, i: usize) -> usize {
        self.page.line_layout(i)
    }
    /// Offset of read line `i` in its owner's text.
    pub fn offset(&self, i: usize) -> usize {
        self.page.line_offset(i)
    }
    /// End of the last line the reader read, its terminator included (Org
    /// does not read a file's final blank lines: the last block's trailer).
    pub fn read_end(&self) -> usize {
        self.page.read_end()
    }
    /// The lines of the pre-block.
    pub fn pre_block(&self) -> std::ops::Range<usize> {
        0..self.pre_block
    }
    /// The read lines of block `n` (preorder).
    pub fn block(&self, n: usize) -> std::ops::Range<usize> {
        let line = self.page.slots()[n].line();
        let end = (self.page.slots().get(n + 1)).map_or(self.page.read_lines(), |s| s.line());
        line..end
    }
    /// Native start of block `n`'s header line, if there is such a block.
    pub fn block_start(&self, n: usize) -> Option<usize> {
        let slot = self.page.slots().get(n)?;
        Some(self.line(slot.line()).start)
    }
    /// Leading layout bytes (indentation) of block `n`'s header line.
    pub fn indent(&self, n: usize) -> usize {
        self.page.slots()[n].indent()
    }
    /// The native byte of offset `at` in the reader's text of the pre-block
    /// (`block` none) or of block `block`'s `raw`. O(log lines).
    pub fn native(&self, block: Option<usize>, at: usize) -> usize {
        let owner = block.map_or(Owner::Preamble, Owner::Slot);
        self.page.content_map(owner).to_file(at)
    }
}

/// Why the page has no representable outline. Each makes `doc::parse` /
/// `org::parse_org` show the whole page as its unparsed preamble text, which a
/// save writes back verbatim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Refusal {
    /// The physical layer refused the text: lsdoc did not take ownership of
    /// it, its quote staircase is deeper than the bound admits, or an event is
    /// outside the parser's contract (malformed imported content).
    Read(tine_text::Refusal),
    /// A slot does not start its own physical line, or two outline headers
    /// share bytes (malformed imported content). Accepting it would give two
    /// blocks the same line, and a save would duplicate or drop text.
    NotOnePerLine,
}

/// The physical layer of `text` with one block per header line, or why the
/// page has none. One bounded lsdoc outline parse, O(page bytes).
pub(crate) fn page(text: &str, format: OutlineFormat) -> Result<PageText<'_>, Refusal> {
    page_lf(text, &tine_text::lf_form(text), format)
}

/// [`page`] for a caller holding `lf`, the LF form of `text`.
fn page_lf<'a>(text: &'a str, lf: &str, format: OutlineFormat) -> Result<PageText<'a>, Refusal> {
    #[cfg(feature = "test-counters")]
    READS.with(|n| n.set(n.get() + 1));
    let page = PageText::read_lf(text, lf, format.text()).map_err(Refusal::Read)?;
    let one_per_line =
        (0..page.slots().len()).all(|n| page.opens_line(n) && !page.slots()[n].opaque());
    one_per_line.then_some(page).ok_or(Refusal::NotOnePerLine)
}

/// [`page`] of `text` (whose LF form is `lf`), or (with a diagnostic) the
/// whole text as the pre-block when the outline is not representable.
pub(crate) fn page_or_preamble<'a>(text: &'a str, lf: &str, format: OutlineFormat) -> PageText<'a> {
    page_lf(text, lf, format).unwrap_or_else(|refusal| {
        crate::diag_line::diagnostic_line(match refusal {
            Refusal::Read(tine_text::Refusal::Invalid { .. }) => {
                "page outline events invalid; shown as page text"
            }
            Refusal::Read(_) => "page outline not owned by lsdoc; shown as page text",
            Refusal::NotOnePerLine => "page outline has two blocks on one line; shown as page text",
        });
        PageText::preamble_only(text, format.text())
    })
}

/// The block-opening lines of `page`, in source order. O(slots).
pub(crate) fn headers_of(page: &PageText<'_>) -> Vec<Header> {
    let slots = page.slots().iter();
    slots
        .map(|slot| Header {
            line: slot.line(),
            level: slot.level(),
            prefix_len: page.line_layout(slot.line()),
            content_indent: slot.dedent(),
        })
        .collect()
}

/// The block-opening lines of `text`, in source order. `Err` names why no
/// outline is representable.
pub(crate) fn headers(text: &str, format: OutlineFormat) -> Result<Vec<Header>, Refusal> {
    page(text, format).map(|page| headers_of(&page))
}

/// Byte offset of the first block-opening line of `text`, or `None` when
/// there is none or the outline is not representable: `doc::parse`/
/// `org::parse_org` then read the whole text as the preamble. Silent (the
/// page parse reports refusals).
pub(crate) fn first_header_start(text: &str, format: OutlineFormat) -> Option<usize> {
    Some(page(text, format).ok()?.slots().first()?.start())
}

/// Headers, or none (with a diagnostic) when the outline is not representable.
pub(crate) fn headers_or_none(text: &str, format: OutlineFormat) -> Vec<Header> {
    headers_of(&page_or_preamble(text, &tine_text::lf_form(text), format))
}

/// The blocks of `page`, one per slot: its content, nested by level.
pub(crate) fn blocks(page: &PageText<'_>, is_org: bool) -> Vec<DocBlock> {
    let flat = (0..page.slots().len()).map(|n| {
        let mut block = DocBlock::new(page.content(n));
        block.is_org = is_org;
        (page.slots()[n].level(), block)
    });
    build_tree(flat.collect())
}

/// The text of `page`'s pre-block lines (`..lines`), joined by `"\n"`.
pub(crate) fn pre_block(page: &PageText<'_>, lines: usize) -> String {
    let text = page.text();
    let pieces = page.pieces(Owner::Preamble).take(lines);
    let texts: Vec<&str> = pieces.map(|piece| &text[piece.text]).collect();
    texts.join("\n")
}

/// Nest a source-order `(level, block)` list: each block goes under the
/// nearest earlier block of a smaller level. Iterative, O(blocks).
fn build_tree(flat: Vec<(u32, DocBlock)>) -> Vec<DocBlock> {
    fn attach(stack: &mut [(u32, DocBlock)], roots: &mut Vec<DocBlock>, done: DocBlock) {
        match stack.last_mut() {
            Some((_, parent)) => parent.children.push(done),
            None => roots.push(done),
        }
    }
    let mut roots = Vec::new();
    let mut stack: Vec<(u32, DocBlock)> = Vec::new();
    for (level, block) in flat {
        while stack.last().is_some_and(|(open, _)| *open >= level) {
            let (_, done) = stack.pop().expect("checked nonempty");
            attach(&mut stack, &mut roots, done);
        }
        stack.push((level, block));
    }
    while let Some((_, done)) = stack.pop() {
        attach(&mut stack, &mut roots, done);
    }
    roots
}

/// Whether lsdoc reads `line`, alone, as an unbulleted Markdown heading.
pub(crate) fn is_unbulleted_heading_line(line: &str) -> bool {
    !line.contains(['\n', '\r'])
        && tine_text::bound::parse_outline_bounded(line, false).is_some_and(|events| {
            matches!(events.as_slice(), [only]
                if only.kind == lsdoc::OutlineHeaderKind::MarkdownUnbulletedAtxHeading
                    && only.header_start == 0)
        })
}
