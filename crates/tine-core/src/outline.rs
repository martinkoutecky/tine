//! Project tine-text's block content into the existing document tree and map.
//! One outline parse and O(page bytes).

use crate::doc::DocBlock;
use tine_text::OutlineRefusal;
pub(crate) use tine_text::{strip_layout_ws, Format as OutlineFormat, Header};

#[cfg(feature = "test-counters")]
thread_local! {
    pub(crate) static READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Where the reader took its answer from (`doc::parse_outlined`): the native
/// byte range of every physical line, and for each line it read the bytes it
/// removed in front (a block's structural prefix or a continuation dedent) and
/// the line's offset in its owner's text (the pre-block, or a block's `raw`).
/// LF, CRLF and lone-CR lines share line indices, so offsets in the reader's
/// LF text map to native bytes by line. Built in the reader's pass, O(lines).
#[derive(Debug, Default)]
pub struct SourceMap {
    /// Native start of every physical line, then the text length; and the
    /// native end of every physical line before its terminator.
    starts: Vec<usize>,
    ends: Vec<usize>,
    /// Per read line: the bytes the reader removed in front, and the line's
    /// offset in its owner's text.
    removed: Vec<usize>,
    at: Vec<usize>,
    /// Per block, preorder: its header line and that line's leading layout.
    blocks: Vec<(usize, usize)>,
    /// Lines of the pre-block.
    pre_block: usize,
}

impl SourceMap {
    pub(crate) fn new(
        native: &str,
        lines: &[&str],
        removed: Vec<usize>,
        headers: &[Header],
        pre_block: usize,
    ) -> Self {
        let (mut starts, mut ends) = (vec![0], Vec::new());
        for line in crate::doc::physical_lines(native) {
            let start = *starts.last().unwrap();
            ends.push(start + line.len() - crate::doc::line_terminator(line).len());
            starts.push(start + line.len());
        }
        let (mut at, mut next) = (Vec::new(), 0);
        let mut headers_at = headers.iter().map(|h| h.line).peekable();
        for (i, line) in lines.iter().enumerate() {
            if headers_at.next_if_eq(&i).is_some() {
                next = 0;
            }
            at.push(next);
            next += line.len() - removed[i] + 1;
        }
        let blocks = headers
            .iter()
            .map(|h| {
                let line = lines.get(h.line).copied().unwrap_or("");
                (h.line, line.len() - strip_layout_ws(line, usize::MAX).len())
            })
            .collect();
        Self {
            starts,
            ends,
            removed,
            at,
            blocks,
            pre_block,
        }
    }

    /// Physical line `i`, terminator included.
    pub fn line(&self, i: usize) -> std::ops::Range<usize> {
        self.starts[i]..self.starts[i + 1]
    }
    /// The physical lines, in order.
    pub fn lines(&self) -> std::ops::Range<usize> {
        0..self.starts.len() - 1
    }
    /// The line holding native byte `at` (the last line for the text end).
    pub fn line_at(&self, at: usize) -> usize {
        self.starts.partition_point(|s| *s <= at).saturating_sub(1)
    }
    /// Native end of physical line `i` before its terminator.
    pub fn end(&self, i: usize) -> usize {
        self.ends[i]
    }
    /// The bytes of line `i` the reader kept: after what it removed, before
    /// the terminator (a line it did not read is all kept).
    pub fn payload(&self, i: usize) -> std::ops::Range<usize> {
        self.starts[i] + self.removed(i)..self.ends[i]
    }
    /// Bytes the reader removed in front of line `i`.
    pub fn removed(&self, i: usize) -> usize {
        self.removed.get(i).copied().unwrap_or(0)
    }
    /// Offset of read line `i` in its owner's text.
    pub fn offset(&self, i: usize) -> usize {
        self.at[i]
    }
    /// End of the last line the reader read, its terminator included (Org
    /// does not read a file's final blank lines).
    pub fn read_end(&self) -> usize {
        self.starts[self.removed.len()]
    }
    /// The lines of the pre-block.
    pub fn pre_block(&self) -> std::ops::Range<usize> {
        0..self.pre_block
    }
    /// The read lines of block `n` (preorder).
    pub fn block(&self, n: usize) -> std::ops::Range<usize> {
        let end = self.blocks.get(n + 1).map_or(self.removed.len(), |b| b.0);
        self.blocks[n].0..end
    }
    /// Native start of block `n`'s header line, if there is such a block.
    pub fn block_start(&self, n: usize) -> Option<usize> {
        self.blocks.get(n).map(|b| self.starts[b.0])
    }
    /// Leading layout bytes (indentation) of block `n`'s header line.
    pub fn indent(&self, n: usize) -> usize {
        self.blocks[n].1
    }
    /// The native byte of offset `at` in the reader's text of the pre-block
    /// (`block` none) or of block `block`'s `raw`. O(log lines).
    pub fn native(&self, block: Option<usize>, at: usize) -> usize {
        let lines = block.map_or(self.pre_block(), |n| self.block(n));
        let i = lines.start + self.at[lines].partition_point(|s| *s <= at) - 1;
        self.starts[i] + self.removed[i] + at - self.at[i]
    }
}

/// The block-opening lines of `text`, in source order. `text` must use `\n`
/// line breaks (a CRLF's `\r` may stay in the line; a lone `\r` must already
/// be a `\n`). `Err` names why no outline is representable.
pub(crate) fn headers(text: &str, format: OutlineFormat) -> Result<Vec<Header>, OutlineRefusal> {
    #[cfg(feature = "test-counters")]
    READS.with(|n| n.set(n.get() + 1));
    tine_text::headers(text, format)
}

/// Byte offset of the first block-opening line of `text` (same line-break
/// contract as [`headers`]), or `None` when there is none or the outline is
/// not representable: `doc::parse`/`org::parse_org` then read the whole text
/// as the preamble. Silent (the page parse reports refusals).
pub(crate) fn first_header_start(text: &str, format: OutlineFormat) -> Option<usize> {
    let line = headers(text, format).ok()?.first()?.line;
    Some(text.split_inclusive('\n').take(line).map(str::len).sum())
}

/// Headers, or none (with a diagnostic) when the outline is not representable.
pub(crate) fn headers_or_none(text: &str, format: OutlineFormat) -> Vec<Header> {
    headers(text, format).unwrap_or_else(|refusal| {
        crate::diag_line::diagnostic_line(match refusal {
            OutlineRefusal::Unowned => "page outline not owned by lsdoc; shown as page text",
            OutlineRefusal::NotOnePerLine { .. } => {
                "page outline has two blocks on one line; shown as page text"
            }
            OutlineRefusal::Invalid { .. } => "page outline events invalid; shown as page text",
        });
        Vec::new()
    })
}

/// The blocks of `lines` (the page's physical lines), one per header: the
/// header line after its prefix, then each following line up to the next
/// header, dedented by the header's `content_indent`, nested by level.
/// `removed`, when given (one entry per line), records the bytes removed in
/// front of each block line, for the reader's [`SourceMap`].
pub(crate) fn blocks(
    lines: &[&str],
    headers: &[Header],
    is_org: bool,
    removed: Option<&mut [usize]>,
) -> Vec<DocBlock> {
    build_tree(
        tine_text::block_contents(lines, headers, removed).map(|(level, raw)| {
            let mut block = DocBlock::new(raw);
            block.is_org = is_org;
            (level, block)
        }),
    )
}

/// Nest a source-order `(level, block)` list: each block goes under the
/// nearest earlier block of a smaller level. Iterative, O(blocks).
fn build_tree(flat: impl Iterator<Item = (u32, DocBlock)>) -> Vec<DocBlock> {
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
