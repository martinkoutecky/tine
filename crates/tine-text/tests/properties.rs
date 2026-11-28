//! The physical layer's properties as tests, on the repository's page
//! corpora, on the model's witnesses and on generated pages. Each test named
//! after a theorem of the block-text model
//! (`docs/block-text-model/lean/BlockText.lean`) checks that theorem's
//! statement on this crate's reader and writer:
//!
//! - `r0`: the pieces, trailers included, concatenate to the file.
//! - `r1`: [`apply`] with no splices returns the file.
//! - `gap_total`, `gap_monotone`, `gap_strict`: a content offset maps to a file
//!   byte inside its slot; the map is monotone, strictly so on the content.
//! - `text_address_inverse`, `content_newline`: a text byte maps to itself at
//!   one file byte; a `"\n"` maps to its line's break and the next line's
//!   removed indent.
//! - `checked_write_exact`: [`apply`] accepts an edit exactly when the model's
//!   planned rows read back (`model::checked_write`), with the same bytes.
//! - `checked_write_commute`: any order of a separate batch gives the same
//!   bytes, or the same refusal.
//! - `r3`, `r3_patch_start`, `r3_separator_patch`: the new file is the old
//!   file's kept runs, in order, with each splice's patch at its mapped gap;
//!   a patch is its replacement, with breaks, continuation layout, the
//!   separator a bare header earns and the CR that keeps a bare CR apart from
//!   an LF; nothing else is written.
//! - `repaired_page_byte_joins`: the lexer reads the whole new file as the
//!   planned rows of every slice, so no break is joined across a cut.
//! - `lexical_check_iff`: [`check_planned_rows`] holds exactly when the lexer
//!   reads the rows back.
//!
//! Content equality: every slot's content is develop's `DocBlock.raw`
//! (`develop_reader`, the frozen reader). Read-back, which the model does not
//! prove: an accepted edit that adds no header text re-reads, with lsdoc, as
//! the same slots, the edited ones with the splices applied to their content.

mod develop_reader;
mod model;

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tine_text::{
    apply, check_planned_rows, Break, Format, HeaderKind, Owner, PageText, Splice, SpliceError,
};

/// What one page check covered.
#[derive(Default, Debug)]
struct Seen {
    pages: usize,
    slots: usize,
    splices: usize,
    refused: usize,
    mid_line: usize,
    refused_edits: usize,
    read_back: usize,
    strict: usize,
    placements: std::collections::HashMap<Placement, usize>,
    examples: std::collections::HashMap<Placement, String>,
}

fn format_of(path: &Path) -> Format {
    match path.extension().and_then(|e| e.to_str()) {
        Some("org") => Format::Org,
        _ => Format::Markdown,
    }
}

/// The owners of `page`, in file order.
fn owners(page: &PageText<'_>) -> impl Iterator<Item = Owner> {
    std::iter::once(Owner::Preamble).chain((0..page.slots().len()).map(Owner::Slot))
}

/// R0, R1, the map theorems and content equality, for one page.
fn check_read<'a>(text: &'a str, format: Format, seen: &mut Seen) -> Option<PageText<'a>> {
    seen.pages += 1;
    let org = format == Format::Org;
    let page = match PageText::read(text.as_bytes(), format) {
        Ok(page) => page,
        Err(refusal) => {
            seen.refused += 1;
            assert!(
                develop_reader::blocks(text, org).is_err(),
                "{refusal:?} where develop reads {text:?}"
            );
            return None;
        }
    };
    r0_holds(&page);
    assert_eq!(apply(&page, &[]).unwrap(), text);
    // Content equality with develop, where develop gives the page an outline.
    let develop = develop_reader::blocks(text, org);
    let representable =
        (0..page.slots().len()).all(|n| page.opens_line(n) && !page.slots()[n].opaque());
    if representable {
        let ours: Vec<(u32, String)> = (0..page.slots().len())
            .map(|n| (page.slots()[n].level(), page.content(n)))
            .collect();
        assert_eq!(ours, develop.expect("develop reads it"), "{text:?}");
    } else {
        seen.mid_line += 1;
        assert!(develop.is_err(), "develop reads {text:?}");
    }
    seen.slots += page.slots().len();
    map_theorems_hold(&page);
    check_breaks_against_lsdoc(&page);
    Some(page)
}

/// R0: each owner's pieces and then its trailer pieces start where the
/// previous ended, and together they are the file. Only the last owner has a
/// trailer, and a trailer piece is an empty line.
fn r0_holds(page: &PageText<'_>) {
    let text = page.text();
    let last = match page.slots().len() {
        0 => Owner::Preamble,
        n => Owner::Slot(n - 1),
    };
    let mut at = 0;
    for owner in owners(page) {
        assert_eq!(page.range(owner).start, at, "{owner:?} of {text:?}");
        for piece in page.pieces(owner) {
            assert_eq!(piece.layout.start, at, "{text:?}");
            assert_eq!(piece.layout.end, piece.text.start);
            assert_eq!(piece.text.end, piece.brk.start);
            assert_eq!(&text[piece.brk.clone()], piece.kind.as_str(), "{text:?}");
            assert!(!text[piece.text.clone()].contains(['\n', '\r']), "{text:?}");
            at = piece.brk.end;
        }
        if owner == last {
            assert_eq!(page.read_end(), at, "{text:?}");
        }
        for piece in page.trailer(owner) {
            assert_eq!(owner, last, "{text:?}");
            assert_eq!(piece.layout.start, at, "{text:?}");
            assert!(piece.layout.is_empty() && piece.text.is_empty(), "{text:?}");
            assert_eq!(&text[piece.brk.clone()], piece.kind.as_str(), "{text:?}");
            at = piece.brk.end;
        }
        assert_eq!(page.range(owner).end, at, "{owner:?} of {text:?}");
    }
    assert_eq!(at, text.len(), "{text:?}");
}

/// Slot `n`'s gaps: the file byte of every content offset, and two past the
/// end (which clamp to the end).
fn gaps(page: &PageText<'_>, n: usize) -> Vec<usize> {
    let map = page.content_map(Owner::Slot(n));
    (0..=map.len() + 2).map(|at| map.to_file(at)).collect()
}

/// `gap_total`: every gap of a slot is inside the slot's bytes.
fn gap_total_holds(page: &PageText<'_>) {
    for n in 0..page.slots().len() {
        let range = page.range(Owner::Slot(n));
        for gap in gaps(page, n) {
            assert!(range.start <= gap && gap <= range.end, "{:?}", page.text());
        }
    }
}

/// `gap_monotone`: a later offset never maps to an earlier byte.
fn gap_monotone_holds(page: &PageText<'_>) {
    for n in 0..page.slots().len() {
        assert!(
            gaps(page, n).windows(2).all(|w| w[0] <= w[1]),
            "{:?}",
            page.text()
        );
    }
}

/// `gap_strict`: on the content's offsets the map is strictly increasing. Its
/// premise, that the slot is what the lexer reads, holds for every slot read
/// from bytes, which is the only way to make one.
fn gap_strict_holds(page: &PageText<'_>) {
    for n in 0..page.slots().len() {
        let len = page.content_map(Owner::Slot(n)).len();
        let gaps = gaps(page, n);
        assert!(
            gaps[..=len].windows(2).all(|w| w[0] < w[1]),
            "{:?}",
            page.text()
        );
    }
}

/// `text_address_inverse`: a text byte of the content is the file byte at its
/// gap, and no other offset has that gap.
fn text_address_inverse_holds(page: &PageText<'_>) {
    let text = page.text().as_bytes();
    for n in 0..page.slots().len() {
        let content = page.content(n);
        let map = page.content_map(Owner::Slot(n));
        let gaps = gaps(page, n);
        for (i, byte) in content.bytes().enumerate().filter(|(_, b)| *b != b'\n') {
            assert_eq!(text[gaps[i]], byte, "{:?}", page.text());
            assert_eq!(map.to_content(gaps[i]), i);
            assert_eq!(
                gaps[..=content.len()]
                    .iter()
                    .filter(|g| **g == gaps[i])
                    .count(),
                1
            );
        }
        // From the file side: every byte of the slot has an offset, and a text
        // byte's offset maps back to it.
        let pieces: Vec<_> = map.pieces().collect();
        for file in page.range(Owner::Slot(n)) {
            let at = map.to_content(file);
            assert!(at <= content.len());
            if pieces.iter().any(|(_, p)| p.text.contains(&file)) {
                assert_eq!(map.to_file(at), file, "{:?}", page.text());
            }
        }
    }
}

/// `content_newline`: a `"\n"` of the content spans exactly its line's
/// break and the next line's removed indent, and that break is a real one.
fn content_newline_holds(page: &PageText<'_>) {
    let text = page.text();
    for n in 0..page.slots().len() {
        let map = page.content_map(Owner::Slot(n));
        let pieces: Vec<_> = map.pieces().collect();
        for pair in pieces.windows(2) {
            let ((at, piece), (_, next)) = (&pair[0], &pair[1]);
            let i = at + piece.text.len();
            assert_eq!(page.content(n).as_bytes()[i], b'\n');
            assert!(matches!(piece.kind, Break::Lf | Break::CrLf | Break::Cr));
            let expected = format!("{}{}", &text[piece.brk.clone()], &text[next.layout.clone()]);
            assert_eq!(
                &text[map.to_file(i)..map.to_file(i + 1)],
                expected,
                "{text:?}"
            );
        }
    }
}

/// Every map theorem, for one page.
fn map_theorems_hold(page: &PageText<'_>) {
    gap_total_holds(page);
    gap_monotone_holds(page);
    gap_strict_holds(page);
    text_address_inverse_holds(page);
    content_newline_holds(page);
}

/// lsdoc's own scan of the file (not its LF form) ends each header line where
/// this crate ends that physical line: the break kinds agree.
fn check_breaks_against_lsdoc(page: &PageText<'_>) {
    let org = page.format() == Format::Org;
    use tine_text::bound::{quote_depth_within, SOURCE_QUOTE_DEPTH_MAX};
    if !quote_depth_within(page.text(), SOURCE_QUOTE_DEPTH_MAX) {
        return;
    }
    let Ok(outline) = lsdoc::parse_outline(page.text(), if org { "org" } else { "md" }) else {
        return;
    };
    let lines: Vec<_> = (0..page.line_count()).map(|i| page.line(i).0).collect();
    let crlf: String = page
        .text()
        .split("\r\n")
        .map(|part| part.replace('\r', "\n"))
        .collect::<Vec<_>>()
        .join("\r\n");
    if let Ok(crlf) = lsdoc::parse_outline(&crlf, if org { "org" } else { "md" }) {
        let native: Vec<_> = crlf
            .headers
            .iter()
            .map(|h| (h.header_start, h.level))
            .collect();
        let ours: Vec<_> = page
            .slots()
            .iter()
            .map(|s| (s.start(), s.level()))
            .collect();
        assert_eq!(native, ours, "outline of the CRLF text {crlf:?}");
    }
    for header in outline.headers {
        let i = lines.partition_point(|l| l.start <= header.line.start) - 1;
        assert_eq!(
            lines[i],
            header.line.start..header.line.end,
            "{:?}",
            page.text()
        );
    }
}

/// Deterministic xorshift64*.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
    fn word(&mut self) -> String {
        let len = 1 + self.below(6);
        (0..len)
            .map(|_| (b'a' + self.below(26) as u8) as char)
            .collect()
    }
}

/// Replacements the generated edits draw from: line breaks, layout bytes
/// (the separators a bare header's rule takes), a leading byte that is not a
/// separator, and multibyte text. None holds header text of its own.
const REPLACEMENTS: &[&str] = &[
    "", "x", "yz", "\n", "a\nb", "\n\n", " é ", "q\n  r", "\t", " ", "\x0c", "x\n", "\nx", "#",
];

/// A range of slot `n`'s content (`content`): anywhere, at its start, a
/// head of it, one whole line with a separator next to it, or its end.
fn some_range(page: &PageText<'_>, n: usize, content: &str, rng: &mut Rng) -> Range<usize> {
    let len = content.len();
    match rng.below(6) {
        0 | 1 => {
            let (a, b) = (boundary(content, rng), boundary(content, rng));
            a.min(b)..a.max(b)
        }
        2 => 0..0,
        3 => 0..boundary(content, rng),
        4 => {
            let pieces: Vec<_> = page.content_map(Owner::Slot(n)).pieces().collect();
            let k = rng.below(pieces.len());
            let (at, piece) = &pieces[k];
            let end = at + piece.text.len();
            if k + 1 < pieces.len() {
                *at..end + 1
            } else {
                at.saturating_sub(1)..end
            }
        }
        _ => len..len,
    }
}

/// The model's `separate`: half-open ranges that do not overlap, and not
/// both insertions at one gap.
fn separate(a: &Range<usize>, b: &Range<usize>) -> bool {
    a != b && (a.end <= b.start || b.end <= a.start)
}

/// Up to `count` splices of one snapshot, in bounds and pairwise separate,
/// with replacements from `replacements`.
fn batch(
    page: &PageText<'_>,
    rng: &mut Rng,
    count: usize,
    replacements: &[&'static str],
) -> Vec<Splice<'static>> {
    let mut chosen: Vec<Splice<'static>> = Vec::new();
    for _ in 0..count {
        let n = rng.below(page.slots().len());
        let range = some_range(page, n, &page.content(n), rng);
        if chosen
            .iter()
            .all(|s| s.slot != n || separate(&s.range, &range))
        {
            let text = replacements[rng.below(replacements.len())];
            chosen.push(Splice {
                slot: n,
                range,
                text,
            });
        }
    }
    chosen
}

/// `content` with `splices` (of one slot, separate) applied: the model's
/// `applyContent`.
fn apply_content(content: &str, splices: &[&Splice<'_>]) -> String {
    let mut sorted = splices.to_vec();
    sorted.sort_by_key(|s| (s.range.start, s.range.end));
    let mut out = String::new();
    let mut at = 0;
    for s in sorted {
        out.push_str(&content[at..s.range.start]);
        out.push_str(s.text);
        at = s.range.end;
    }
    out + &content[at..]
}

/// The level of the outline slot one line opens when read alone, if it
/// opens one: header text.
fn header_level(line: &str, format: Format) -> Option<u32> {
    tine_text::bound::parse_outline_bounded(line, format == Format::Org)
        .and_then(|events| events.first().map(|e| e.level))
}

/// Whether an edit of slot `n` giving it `new` content adds no header text:
/// no line after the first opens a slot when read alone, and the first line
/// of an unbulleted heading (which is content) opens one of the same level.
/// Such an edit should re-read as the same slots.
fn adds_no_header_text(page: &PageText<'_>, n: usize, new: &str) -> bool {
    let format = page.format();
    let mut lines = new.split('\n');
    let first = lines.next().unwrap_or("");
    let heading = page.slots()[n].kind() == HeaderKind::MarkdownHeading;
    let level = || {
        let old = page.content(n);
        header_level(old.split('\n').next().unwrap_or(""), format)
    };
    (!heading || header_level(first, format) == level())
        && lines.all(|line| header_level(line, format).is_none())
}

/// Where lsdoc does not place an accepted edit's slots as the write planned
/// them: a question for the parser or the model, not a byte the write got
/// wrong. Each is printed with one input's bytes and reported.
#[derive(Debug, Eq, PartialEq, Hash, Clone, Copy)]
enum Placement {
    /// The edit adds, removes or rewrites a line lsdoc may read as block
    /// structure (a fence, a `#+` directive, a drawer or property line
    /// such as `:END:` or `:id: x`, a list item, a quote, a table row, a
    /// rule), so lsdoc reads the lines around it as other structure, other
    /// slots included.
    BlockLineEdited,
    /// The edit changes a header line's text, the written line still opens a
    /// slot of its level when read alone, but lsdoc reads it as part of the
    /// structure before it (a quote's lazy continuation): fewer slots.
    HeaderAbsorbedByContext,
    /// The edit changes a slot that starts inside a line, or whose last line
    /// goes on into the next slot (a header lsdoc found inside that line):
    /// whether lsdoc finds the header there depends on the whole line, which
    /// the edit changed.
    MidLineCutEdited,
    /// A question for the model: a bare dash whose content starts with an
    /// ATX run (`-# H`) is a header because lsdoc takes `-`, the `#` run and
    /// a separator after it as a header; the model's `separator` looks at
    /// the first byte only, so an edit that keeps the first `#` but not the
    /// separator after the run writes `-#x`, which is no header.
    BareDashAtxRun,
    /// Org reads the file's final empty lines as trailer, not content: an
    /// edit that ends the last slot's content with empty lines loses them.
    OrgFinalEmptyLines,
    /// A bare header whose new first byte is an ASCII space: the model takes
    /// the space as the separator (no space is added), lsdoc then takes it
    /// as the header's prefix, so the content loses its first byte.
    BareHeaderLeadingSpace,
    /// Layout bytes (space, tab, form feed) put at the start of a kept line
    /// whose own layout is shorter than the bullet's content column: lsdoc
    /// removes them as that line's indent.
    ContinuationLayoutRemoved,
}

/// Whether a content line may open or close a block-level structure: its
/// first byte after layout is a block marker, or a colon (a drawer line or
/// a drawer's property line).
fn block_line(line: &str) -> bool {
    let t = line.trim_start_matches([' ', '\t', '\x0c']);
    let marker = t.starts_with(['`', '$', '#', '>', '*', '+', '-', '|', '=', ':'])
        || t.split_once('.')
            .is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    marker
}

/// Whether `old` and `new` content differ in their block lines.
fn block_lines_edited(old: &str, new: &str) -> bool {
    let of = |s: &str| {
        s.split('\n')
            .filter(|l| block_line(l))
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    of(old) != of(new)
}

/// The content placement that explains `got` where `want` was planned for
/// slot `n`, if one does.
fn content_placement(page: &PageText<'_>, n: usize, want: &str, got: &str) -> Option<Placement> {
    let last = n + 1 == page.slots().len();
    if page.format() == Format::Org && last && want.trim_end_matches('\n') == got {
        return Some(Placement::OrgFinalEmptyLines);
    }
    if page.slots()[n].bare() && want.strip_prefix(' ') == Some(got) {
        return Some(Placement::BareHeaderLeadingSpace);
    }
    let (wants, gots): (Vec<_>, Vec<_>) = (want.split('\n').collect(), got.split('\n').collect());
    let layout_removed = |w: &str, g: &str| {
        w.len() > g.len()
            && w.ends_with(g)
            && w[..w.len() - g.len()]
                .bytes()
                .all(|b| matches!(b, b' ' | b'\t' | 0x0c))
    };
    let dedent = page.slots()[n].dedent() > 0;
    if dedent
        && wants.len() == gots.len()
        && wants
            .iter()
            .zip(&gots)
            .skip(1)
            .all(|(w, g)| w == g || layout_removed(w, g))
        && wants[0] == gots[0]
    {
        return Some(Placement::ContinuationLayoutRemoved);
    }
    None
}

/// The placement question an edit raises for the structure around it, from
/// its input alone: `want` is each slot's planned content.
fn structure_placement(
    page: &PageText<'_>,
    splices: &[Splice<'_>],
    want: &[String],
) -> Option<Placement> {
    let edited: Vec<usize> = (0..want.len())
        .filter(|m| splices.iter().any(|s| s.slot == *m))
        .collect();
    let cut = |m: usize| {
        !page.opens_line(m) || page.content_map(Owner::Slot(m)).final_break() == Break::Continues
    };
    let first = |s: &str| s.split('\n').next().unwrap_or("").to_string();
    let content = |m: usize| page.content(m);
    // The model question first: it is the most specific.
    let bare_atx = |m: usize| {
        page.slots()[m].bare()
            && content(m).starts_with('#')
            && want[m].starts_with('#')
            && first(&content(m)) != first(&want[m])
    };
    if edited.iter().any(|m| bare_atx(*m)) {
        Some(Placement::BareDashAtxRun)
    } else if edited
        .iter()
        .any(|m| block_lines_edited(&content(*m), &want[*m]))
    {
        Some(Placement::BlockLineEdited)
    } else if edited.iter().any(|m| cut(*m)) {
        Some(Placement::MidLineCutEdited)
    } else {
        None
    }
}

/// Whether slot `m`'s header line, as written, still opens a slot of its
/// level when read alone, though its text changed: the bytes in front of the
/// slot on its line, then its planned first row.
fn header_line_kept(
    page: &PageText<'_>,
    splices: &[Splice<'_>],
    want: &[String],
    m: usize,
) -> bool {
    let first = |s: &str| s.split('\n').next().unwrap_or("").to_string();
    let plan = model::plan_page(page, splices);
    let slot = &page.slots()[m];
    let line_start = page.line(slot.line()).0.start;
    let written = [
        &page.text().as_bytes()[line_start..slot.start()],
        &plan.slots[m].rows[0].line().0[..],
    ]
    .concat();
    let line = String::from_utf8(written).unwrap();
    first(&page.content(m)) != first(&want[m])
        && header_level(&line, page.format()) == Some(slot.level())
}

/// R3 for splices inside plain lines (lowercase letters and spaces), whose
/// replacement cannot open or close a structure: a letter run is replaced by
/// letters, possibly with one `"\n"` between letters, or the separator of two
/// plain lines is removed. These read back exactly, with no [`Placement`]
/// allowed.
fn check_splices(page: &PageText<'_>, rng: &mut Rng, seen: &mut Seen) {
    let plain = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b == b' ');
    let slots = page.slots().len();
    if slots == 0 || !(0..slots).all(|n| page.opens_line(n)) {
        return;
    }
    let text = page.text();
    for _ in 0..3 {
        let n = rng.below(slots);
        let map = page.content_map(Owner::Slot(n));
        let pieces: Vec<_> = map.pieces().collect();
        let k = rng.below(pieces.len());
        let (at, piece) = &pieces[k];
        if !plain(&text[piece.text.clone()]) {
            continue;
        }
        let line = &text[piece.text.clone()];
        let (range, replacement) = if rng.below(4) == 0 && k + 1 < pieces.len() {
            if !plain(&text[pieces[k + 1].1.text.clone()]) {
                continue;
            }
            let end = at + line.len();
            (end..end + 1, String::new())
        } else {
            let letters: Vec<usize> = (0..line.len())
                .filter(|i| line.as_bytes()[*i].is_ascii_lowercase())
                .collect();
            if letters.is_empty() {
                continue;
            }
            let start = letters[rng.below(letters.len())];
            let mut end = start;
            while end < line.len() && line.as_bytes()[end].is_ascii_lowercase() && rng.below(3) > 0
            {
                end += 1;
            }
            let run_start = start == 0 || !line.as_bytes()[start - 1].is_ascii_lowercase();
            let mut replacement = String::new();
            if run_start || rng.below(2) == 0 {
                replacement.push_str(&rng.word());
                if rng.below(3) == 0 {
                    replacement.push('\n');
                    replacement.push_str(&rng.word());
                }
            }
            (at + start..at + end, replacement)
        };
        let content = page.content(n);
        let expected = format!(
            "{}{}{}",
            &content[..range.start],
            replacement,
            &content[range.end..]
        );
        let file = map.to_file(range.start)..map.to_file(range.end);
        let splice = Splice {
            slot: n,
            range,
            text: &replacement,
        };
        let out = apply(page, &[splice]).unwrap();
        seen.strict += 1;
        assert_eq!(&out[..file.start], &text[..file.start]);
        assert_eq!(
            &out[out.len() - (text.len() - file.end)..],
            &text[file.end..]
        );
        let again = PageText::read(out.as_bytes(), page.format()).unwrap();
        assert_eq!(again.slots().len(), slots, "{text:?} -> {out:?}");
        for m in 0..slots {
            assert_eq!(again.slots()[m].level(), page.slots()[m].level());
            if m == n {
                assert_eq!(again.content(m), expected, "{text:?} -> {out:?}");
            } else {
                assert_eq!(
                    &out[again.range(Owner::Slot(m))],
                    &text[page.range(Owner::Slot(m))]
                );
                assert_eq!(again.content(m), page.content(m));
            }
        }
    }
}

/// Read-back with lsdoc: an accepted batch on `page` whose edits add no
/// header text re-reads as the same slots and levels, the untouched ones
/// byte for byte with their content, the edited ones with their content
/// spliced. A failure a [`Placement`] explains is counted and one example
/// kept; any other fails the test.
fn read_back_holds(page: &PageText<'_>, rng: &mut Rng, seen: &mut Seen) {
    let slots = page.slots().len();
    if slots == 0 {
        return;
    }
    let text = page.text();
    let count = 1 + rng.below(3);
    let splices = batch(page, rng, count, REPLACEMENTS);
    let edited = |m: usize| splices.iter().any(|s| s.slot == m);
    let want: Vec<String> = (0..slots)
        .map(|m| {
            let mine: Vec<_> = splices.iter().filter(|s| s.slot == m).collect();
            apply_content(&page.content(m), &mine)
        })
        .collect();
    if !(0..slots).all(|m| !edited(m) || adds_no_header_text(page, m, &want[m])) {
        return;
    }
    let Ok(out) = apply(page, &splices) else {
        seen.refused_edits += 1;
        return;
    };
    seen.splices += splices.len();
    let mut record = |why: Option<Placement>, read: String| {
        let Some(why) = why else {
            panic!("read-back: {text:?} {splices:?} -> {out:?}\nplanned {want:?}\nread    {read}");
        };
        // Every such input, with its bytes (shown with `--nocapture`).
        eprintln!("read-back {why:?}: {text:?} {splices:?} -> {out:?} read {read}");
        *seen.placements.entry(why).or_default() += 1;
        seen.examples
            .entry(why)
            .or_insert_with(|| format!("{text:?} {splices:?} -> {out:?}"));
    };
    let again = match PageText::read(out.as_bytes(), page.format()) {
        Ok(again) => again,
        Err(refusal) => {
            return record(
                structure_placement(page, &splices, &want),
                format!("{refusal:?}"),
            );
        }
    };
    let read: Vec<(u32, String)> = (0..again.slots().len())
        .map(|m| (again.slots()[m].level(), again.content(m)))
        .collect();
    let same_slots =
        read.len() == slots && (0..slots).all(|m| read[m].0 == page.slots()[m].level());
    let untouched = |m: usize| {
        out[again.range(Owner::Slot(m))] == text[page.range(Owner::Slot(m))]
            && read[m].1 == page.content(m)
    };
    let why = if !same_slots || (0..slots).any(|m| !edited(m) && !untouched(m)) {
        structure_placement(page, &splices, &want).or_else(|| {
            let absorbed = |m: usize| edited(m) && header_line_kept(page, &splices, &want, m);
            (read.len() < slots && (0..slots).any(absorbed))
                .then_some(Placement::HeaderAbsorbedByContext)
        })
    } else {
        let wrong: Vec<usize> = (0..slots)
            .filter(|m| edited(*m) && read[*m].1 != want[*m])
            .collect();
        if wrong.is_empty() {
            seen.read_back += 1;
            return;
        }
        let explain = |m: usize| {
            content_placement(page, m, &want[m], &read[m].1)
                .or_else(|| structure_placement(page, &splices, &want))
        };
        let reasons: Option<Vec<Placement>> = wrong.into_iter().map(explain).collect();
        reasons.map(|r| r[0])
    };
    record(why, format!("{read:?}"));
}

/// A generated page: random nesting, LF/CRLF/CR breaks, tabs and spaces,
/// blank and whitespace-only lines, headers mid-line, a missing final break.
fn generate(rng: &mut Rng) -> (String, Format) {
    let format = if rng.below(2) == 0 {
        Format::Markdown
    } else {
        Format::Org
    };
    let page_break = rng.pick(&["\n", "\r\n", "\r", ""]);
    let mut out = String::new();
    let lines = 1 + rng.below(12);
    for i in 0..lines {
        let indent: String = (0..rng.below(4))
            .map(|_| rng.pick(&["  ", "\t", " "]))
            .collect();
        let words = format!("{} {}", rng.word(), rng.word());
        let line = match rng.below(17) {
            0..=3 => format!("{indent}{}{words}", rng.pick(&["- ", "-", "-\t", "-  "])),
            4 => format!("{}{words}", rng.pick(&["# ", "## ", "  # "])),
            5 | 6 => format!("{} {words}", rng.pick(&["*", "**", "***"])),
            7 => format!("{indent}{words}"),
            8 => rng.pick(&["", " ", "\t ", "  "]).to_string(),
            9 => format!(
                "{indent}{}",
                rng.pick(&["```", "```js", "#+BEGIN_QUOTE", "#+END_QUOTE"])
            ),
            10 => format!(
                "{indent}{}",
                rng.pick(&["a:: b", ":PROPERTIES:", ":END:", ":id: x", "id:: y"])
            ),
            11 => format!(
                "{}",
                rng.pick(&[
                    ":END: # Kept",
                    "- $$x$$ # #+BEGIN_NOTE",
                    "x # y",
                    "#+END_NOTE"
                ])
            ),
            12 => format!("{indent}> {words}"),
            13 => format!("{indent}- TODO {words}"),
            14 => format!(
                "{indent}{}",
                rng.pick(&["$$", "#+BEGIN_SRC", "#+END_SRC", "---"])
            ),
            // Bare headers: the marker, then content or nothing.
            15 => format!(
                "{indent}{}",
                rng.pick(&["-", "-\t", "-# H", "-## H", "*", "**", "*\x0cT", "-\tT"])
            ),
            _ => format!("{indent}{} {words}", rng.pick(&["1.", "+", "|"])),
        };
        out.push_str(&line);
        let brk = match page_break {
            "" => rng.pick(&["\n", "\r\n", "\r"]),
            brk => brk,
        };
        if i + 1 < lines || rng.below(3) > 0 {
            out.push_str(brk);
        }
    }
    (out, format)
}

#[test]
fn generated_pages() {
    let started = std::time::Instant::now();
    let mut rng = Rng(0x7469_6e65_7465_7874);
    // The strict plain-line check draws from its own stream.
    let mut strict = Rng(0x7374_7269_6374);
    let mut seen = Seen::default();
    for _ in 0..20_000 {
        let (text, format) = generate(&mut rng);
        if let Some(page) = check_read(&text, format, &mut seen) {
            check_splices(&page, &mut strict, &mut seen);
            read_back_holds(&page, &mut rng, &mut seen);
        }
    }
    println!("generated: {seen:?} in {:?}", started.elapsed());
    assert_eq!(seen.pages, 20_000);
    assert!(seen.splices > 10_000 && seen.mid_line > 100, "{seen:?}");
    assert!(seen.strict > 10_000, "{seen:?}");
}

/// The repository's page corpora: the kitchen-sink and mock graphs, the Org
/// graph (`kalem/logseq-org`), the Guide, and the tine-core, tine-store and
/// tine-graph-features fixtures.
fn corpus_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in [
        "src/fixtures",
        "src/mock",
        "samples",
        "scripts/fixtures",
        "crates/tine-guide/src/templates",
        "crates/tine-core/src/query/fixtures",
        "crates/tine-core/tests",
        "crates/tine-store/tests",
        "crates/tine-graph-features/tests",
    ] {
        walk(&root.join(dir), &mut files);
    }
    files.sort();
    files
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("md" | "org")
        ) {
            out.push(path);
        }
    }
}

#[test]
fn repository_corpora() {
    let mut rng = Rng(0x636f_7270_7573);
    // The strict plain-line check draws from its own stream.
    let mut strict = Rng(0x7374_7269_6374_32);
    let mut seen = Seen::default();
    let files = corpus_files();
    let org = files.iter().filter(|f| format_of(f) == Format::Org).count();
    assert!(
        files.len() > 200 && org > 15,
        "{} files, {org} org",
        files.len()
    );
    let mut not_utf8 = 0;
    for path in &files {
        let bytes = std::fs::read(path).unwrap();
        let Ok(text) = std::str::from_utf8(&bytes) else {
            let refusal = PageText::read(&bytes, format_of(path)).unwrap_err();
            assert_eq!(refusal, tine_text::Refusal::NotUtf8);
            not_utf8 += 1;
            continue;
        };
        if let Some(page) = check_read(text, format_of(path), &mut seen) {
            for _ in 0..8 {
                check_splices(&page, &mut strict, &mut seen);
                read_back_holds(&page, &mut rng, &mut seen);
            }
        }
    }
    println!(
        "corpora: {} files, {not_utf8} not UTF-8, {seen:?}",
        files.len()
    );
    assert!(seen.slots > 2_000 && seen.splices > 1_000, "{seen:?}");
    assert!(seen.strict > 1_000, "{seen:?}");
}

/// A private corpus: R0, R1 and content equality over every page
/// under `TINE_TEXT_CORPUS`. Never run on a lane.
#[test]
#[ignore = "needs TINE_TEXT_CORPUS"]
fn private_corpus() {
    let dir = std::env::var("TINE_TEXT_CORPUS").expect("TINE_TEXT_CORPUS");
    let mut files = Vec::new();
    walk(Path::new(&dir), &mut files);
    let mut seen = Seen::default();
    for path in &files {
        if let Ok(text) = std::fs::read_to_string(path) {
            check_read(&text, format_of(path), &mut seen);
        }
    }
    println!("private corpus: {} files, {seen:?}", files.len());
    assert!(!files.is_empty());
}

/// Mixed line endings: two files whose first slot has the same content but different
/// internal breaks stay distinct, and a splice in one line keeps every break.
#[test]
fn h1_mixed_line_endings_round_trip() {
    let a = "- T\r\n  old\n  tail\r\n- Next\n";
    let b = "- T\n  old\r\n  tail\r\n- Next\n";
    for text in [a, b] {
        let page = check_read(text, Format::Markdown, &mut Seen::default()).unwrap();
        assert_eq!(page.content(0), "T\nold\ntail");
        let splice = Splice {
            slot: 0,
            range: 2..5,
            text: "new",
        };
        assert_eq!(apply(&page, &[splice]).unwrap(), text.replace("old", "new"));
    }
    let kinds = |text| {
        let page = PageText::read(text, Format::Markdown).unwrap();
        page.pieces(Owner::Slot(0))
            .map(|p| p.kind)
            .collect::<Vec<_>>()
    };
    assert_eq!(kinds(a.as_bytes()), [Break::CrLf, Break::Lf, Break::CrLf]);
    assert_eq!(kinds(b.as_bytes()), [Break::Lf, Break::CrLf, Break::CrLf]);
}

/// A `"\n"` in a replacement is written as the break of the line it is
/// inserted in, then the slot's continuation indent.
#[test]
fn a_new_line_takes_the_lines_break_and_the_slots_indent() {
    let text = "- a\n\t- bc\r\n\t  d\r";
    let page = PageText::read(text.as_bytes(), Format::Markdown).unwrap();
    assert_eq!(page.content(1), "bc\nd");
    let split = Splice {
        slot: 1,
        range: 1..1,
        text: "x\ny",
    };
    assert_eq!(
        apply(&page, &[split]).unwrap(),
        "- a\n\t- bx\r\n\t  yc\r\n\t  d\r"
    );
    let end = Splice {
        slot: 0,
        range: 1..1,
        text: "\n",
    };
    let eof = PageText::read(b"* a", Format::Org).unwrap();
    assert_eq!(apply(&page, &[end]).unwrap(), "- a\n  \n\t- bc\r\n\t  d\r");
    let end = Splice {
        slot: 0,
        range: 0..1,
        text: "b\nc",
    };
    assert_eq!(apply(&eof, &[end]).unwrap(), "* b\nc");
}

/// Mid-line headers: an outline header that starts inside a physical line opens a
/// slot there; the line's first bytes stay with the owner before it. Develop
/// refuses both pages (shown as page text).
#[test]
fn h2_headers_mid_line_open_slots_inside_the_line() {
    let text = ":PROPERTIES:\n:key: value\n:END: # Kept\nnext:: property\n";
    let page = check_read(text, Format::Markdown, &mut Seen::default()).unwrap();
    assert_eq!(page.slots().len(), 1);
    assert!(!page.opens_line(0));
    assert_eq!(page.content(0), " # Kept\nnext:: property");
    let last = page.pieces(Owner::Preamble).last().unwrap();
    assert_eq!((&text[last.text], last.kind), (":END:", Break::Continues));

    let text = "- $$x$$ # #+BEGIN_NOTE\r\nx\r\n#+END_NOTE";
    let page = check_read(text, Format::Markdown, &mut Seen::default()).unwrap();
    let slots: Vec<_> = (0..2)
        .map(|n| (page.slots()[n].level(), page.content(n)))
        .collect();
    assert_eq!(
        slots,
        [
            (1, "$$x$$".to_string()),
            (2, " # #+BEGIN_NOTE\nx\n#+END_NOTE".to_string())
        ]
    );
    assert!(develop_reader::blocks(text, false).is_err());
}

/// Lines end at LF, CRLF and every other CR, and the LF form has the same
/// lines (tine-core's `physical_lines` specimens, which this replaces).
#[test]
fn lines_end_where_the_lf_form_ends_them() {
    for text in [
        "",
        "a",
        "a\n",
        "a\r",
        "a\r\n",
        "a\rb\nc\r\nd",
        "\r\r\n",
        "\n\r",
        "\r\n\r\n",
        "a\r\rb",
        "- x\r  y\r- z",
    ] {
        let page = PageText::preamble_only(text, Format::Markdown);
        let lf = tine_text::lf_form(text);
        let lf_lines: Vec<&str> = lf.split_inclusive('\n').collect();
        assert_eq!(page.line_count(), lf_lines.len(), "{text:?}");
        for (i, lf_line) in lf_lines.iter().enumerate() {
            let (range, brk) = page.line(i);
            assert_eq!(&text[range.start..brk], lf_line.trim_end_matches('\n'));
            assert_eq!(range.end == brk, !lf_line.ends_with('\n'), "{text:?}");
        }
    }
}

/// Org does not read a file's final empty lines; Markdown does not read a
/// file that is one empty line. Those lines are the last owner's trailer.
#[test]
fn the_trailer_is_what_the_reader_does_not_read() {
    let trailer = |text: &'static [u8], format| {
        let page = PageText::read(text, format).unwrap();
        let last = page
            .slots()
            .len()
            .checked_sub(1)
            .map_or(Owner::Preamble, Owner::Slot);
        let pieces: Vec<_> = page.trailer(last).map(|p| (p.range(), p.kind)).collect();
        (page.read_end(), pieces)
    };
    let page = PageText::read(b"* a\nb\n\r\n\n", Format::Org).unwrap();
    assert_eq!(page.content(0), "a\nb");
    assert_eq!(page.range(Owner::Slot(0)), 0..9);
    assert_eq!(
        trailer(b"* a\nb\n\r\n\n", Format::Org),
        (6, vec![(6..8, Break::CrLf), (8..9, Break::Lf)])
    );
    let page = PageText::read(b"- a\nb\n\n", Format::Markdown).unwrap();
    assert_eq!(page.content(0), "a\nb\n");
    assert_eq!(trailer(b"- a\nb\n\n", Format::Markdown), (7, vec![]));
    assert_eq!(
        trailer(b"\r\n", Format::Markdown),
        (0, vec![(0..2, Break::CrLf)])
    );
    // Final breaks of mixed kinds are all the last slot's.
    assert_eq!(
        trailer(b"* T\r\n\n\r", Format::Org),
        (5, vec![(5..6, Break::Lf), (6..7, Break::Cr)])
    );
}

#[test]
fn develops_refusals_are_kept() {
    let deep = format!("- {}x\n", "> ".repeat(200));
    assert_eq!(
        PageText::read(deep.as_bytes(), Format::Markdown).unwrap_err(),
        tine_text::Refusal::Unowned
    );
    assert_eq!(
        PageText::read(b"- \xff\n", Format::Markdown).unwrap_err(),
        tine_text::Refusal::NotUtf8
    );
}

/// The pages every theorem test runs on: generated pages, then every UTF-8
/// page of the repository's corpora. Read once per test process.
fn cases() -> &'static [(String, Format)] {
    static CASES: OnceLock<Vec<(String, Format)>> = OnceLock::new();
    CASES.get_or_init(|| {
        let mut rng = Rng(0x6c65_616e_2d62_7478);
        let mut cases: Vec<_> = (0..4_000).map(|_| generate(&mut rng)).collect();
        for path in corpus_files() {
            if let Ok(text) = std::fs::read_to_string(&path) {
                cases.push((text, format_of(&path)));
            }
        }
        cases
    })
}

/// Run `check` on every page of [`cases`] that has a physical layer, with a
/// generator; returns how many pages it ran on.
fn on_cases(mut check: impl FnMut(&PageText<'_>, &mut Rng)) -> usize {
    let mut rng = Rng(0x7468_656f_7265_6d73);
    let mut pages = 0;
    for (text, format) in cases() {
        if let Ok(page) = PageText::read_text(text, *format) {
            pages += 1;
            check(&page, &mut rng);
        }
    }
    assert!(pages > 3_000, "{pages} pages");
    pages
}

#[test]
fn r0() {
    on_cases(|page, _| r0_holds(page));
}

#[test]
fn r1() {
    on_cases(|page, _| assert_eq!(apply(page, &[]).unwrap(), page.text()));
}

#[test]
fn gap_total() {
    on_cases(|page, _| gap_total_holds(page));
}

#[test]
fn gap_monotone() {
    on_cases(|page, _| gap_monotone_holds(page));
}

#[test]
fn gap_strict() {
    on_cases(|page, _| gap_strict_holds(page));
}

#[test]
fn text_address_inverse() {
    on_cases(|page, _| text_address_inverse_holds(page));
}

#[test]
fn content_newline() {
    on_cases(|page, _| content_newline_holds(page));
}

/// A content offset of slot `n` on a character boundary.
fn boundary(content: &str, rng: &mut Rng) -> usize {
    let mut at = rng.below(content.len() + 1);
    while !content.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// A batch for the writer's theorems: in bounds and separate; one batch in
/// eight has a replacement with a CR.
fn writer_batch(page: &PageText<'_>, rng: &mut Rng) -> Vec<Splice<'static>> {
    const WITH_CR: &[&str] = &["\r", "a\rb", "\r\n"];
    let count = 1 + rng.below(4);
    let mut splices = batch(page, rng, count, REPLACEMENTS);
    if rng.below(8) == 0 && !splices.is_empty() {
        let k = rng.below(splices.len());
        splices[k].text = WITH_CR[rng.below(WITH_CR.len())];
    }
    splices
}

/// Run `check` on writer batches for every page of [`cases`] with a slot.
fn on_batches(mut check: impl FnMut(&PageText<'_>, &[Splice<'static>], &mut Rng)) {
    on_cases(|page, rng| {
        if page.slots().is_empty() {
            return;
        }
        for _ in 0..6 {
            let splices = writer_batch(page, rng);
            check(page, &splices, rng);
        }
    });
}

/// `checkedWrite_exact`: [`apply`] accepts exactly when the model's planned
/// rows of every slot read back and no replacement has a CR, and then writes
/// the model's bytes; it refuses with the reason the input gives.
#[test]
fn checked_write_exact() {
    let (mut accepted, mut cr, mut empty_end) = (0, 0, 0);
    on_batches(|page, splices, _| {
        let ours = apply(page, splices);
        let model = model::checked_write(page, splices);
        let has_cr = splices.iter().any(|s| s.text.contains('\r'));
        match (&ours, &model) {
            (Ok(out), Some(bytes)) => {
                assert_eq!(out.as_bytes(), &bytes[..], "{:?} {splices:?}", page.text());
                accepted += 1;
            }
            (Err(SpliceError::CarriageReturn(_)), None) if has_cr => cr += 1,
            (Err(SpliceError::EmptySliceEnd(_)), None) if !has_cr => empty_end += 1,
            _ => panic!("{:?} {splices:?}: {ours:?}, model {model:?}", page.text()),
        }
    });
    println!("accepted {accepted}, CR {cr}, empty slice end {empty_end}");
    assert!(
        accepted > 10_000 && cr > 1_000 && empty_end > 100,
        "{accepted} {cr} {empty_end}"
    );
}

/// `checkedWrite_commute`: every order of a separate batch gives the same
/// bytes, or a refusal of the same kind.
#[test]
fn checked_write_commute() {
    let kind =
        |r: &Result<String, SpliceError>| r.as_ref().map_err(std::mem::discriminant).cloned();
    on_batches(|page, splices, rng| {
        let first = apply(page, splices);
        let mut plan = splices.to_vec();
        for _ in 0..3 {
            for i in (1..plan.len()).rev() {
                plan.swap(i, rng.below(i + 1));
            }
            assert_eq!(
                kind(&apply(page, &plan)),
                kind(&first),
                "{:?} {plan:?}",
                page.text()
            );
        }
    });
}

/// The bytes of `out` around the patches of `plan`: each patch's file range
/// in the snapshot, in file order. `R3`'s kept runs are the snapshot bytes
/// between them; returns the bytes written in place of each, if the kept
/// runs are all in `out`, in order, and `out` is nothing else.
fn kept_runs_and_patches(
    text: &str,
    out: &str,
    patches: &[(Range<usize>, Vec<u8>)],
) -> Vec<Vec<u8>> {
    let (text, out) = (text.as_bytes(), out.as_bytes());
    let mut sorted = patches.to_vec();
    sorted.sort_by_key(|(file, _)| (file.start, file.end));
    let mut written = Vec::new();
    let (mut at, mut put) = (0, 0);
    for (file, bytes) in &sorted {
        let kept = &text[at..file.start];
        assert_eq!(&out[put..put + kept.len()], kept);
        put += kept.len();
        written.push(out[put..put + bytes.len()].to_vec());
        put += bytes.len();
        at = file.end;
    }
    assert_eq!(&out[put..], &text[at..]);
    written
}

/// The model's patches of every edited slot.
fn model_patches(page: &PageText<'_>, splices: &[Splice<'_>]) -> Vec<(Range<usize>, Vec<u8>)> {
    let plan = model::plan_page(page, splices);
    plan.slots.into_iter().flat_map(|s| s.patches).collect()
}

/// `R3_patchStart`: for an accepted edit, the new file is the old file's
/// kept runs, in order, and between them, at each splice's mapped gap, the
/// bytes the model's byte program puts for that splice (its repaired
/// replacement, a separator, a CR that repairs the next kept run). No byte
/// is written outside a splice.
#[test]
fn r3_patch_start() {
    let mut patches = 0;
    on_batches(|page, splices, _| {
        let Ok(out) = apply(page, splices) else {
            return;
        };
        let plan = model_patches(page, splices);
        let written = kept_runs_and_patches(page.text(), &out, &plan);
        for ((_, want), got) in plan.iter().zip(&written) {
            assert_eq!(got, want, "{:?} {splices:?} -> {out:?}", page.text());
        }
        patches += plan.len();
    });
    assert!(patches > 20_000, "{patches}");
}

/// `R3`: each splice's patch is its replacement, every `"\n"` written as the
/// break of the snapshot line holding the splice's start (CRLF on a CRLF
/// line, else LF) and the slot's continuation layout, with exactly three
/// additions: a CR before an LF break written right after a bare CR; a CR at
/// its end when the kept run after it starts with an LF and a bare CR comes
/// before it; and one space at the start of a bare header's first patch.
#[test]
fn r3() {
    let mut repairs = 0;
    on_batches(|page, splices, _| {
        let Ok(out) = apply(page, splices) else {
            return;
        };
        let text = page.text();
        let mut sorted = splices.to_vec();
        sorted.sort_by_key(|s| (s.slot, s.range.start, s.range.end));
        let plan: Vec<_> = model_patches(page, splices);
        let written = kept_runs_and_patches(text, &out, &plan);
        // Rebuild each patch from the rule, with the byte written before it.
        let mut before: Vec<u8> = Vec::new();
        let mut at = 0;
        for (k, (splice, got)) in sorted.iter().zip(&written).enumerate() {
            let map = page.content_map(Owner::Slot(splice.slot));
            let file = map.to_file(splice.range.start)..map.to_file(splice.range.end);
            before.extend_from_slice(&text.as_bytes()[at..file.start]);
            let start = before.len();
            let first = k == 0 || sorted[k - 1].slot != splice.slot;
            let space = first && got.first() == Some(&b' ') && !splice.text.starts_with(' ');
            if space {
                assert!(page.slots()[splice.slot].bare() && splice.range.start == 0);
                before.push(b' ');
            }
            let style: &[u8] = match map.break_at(splice.range.start) {
                Break::CrLf => b"\r\n",
                _ => b"\n",
            };
            let continuation = page.continuation(splice.slot);
            for (i, segment) in splice.text.split('\n').enumerate() {
                if i > 0 {
                    if style == b"\n" && before.last() == Some(&b'\r') {
                        before.push(b'\r');
                        repairs += 1;
                    }
                    before.extend_from_slice(style);
                    before.extend_from_slice(continuation.as_bytes());
                }
                before.extend_from_slice(segment.as_bytes());
            }
            at = file.end;
            let next = sorted.get(k + 1).map_or(text.len(), |n| {
                let m = page.content_map(Owner::Slot(n.slot));
                m.to_file(n.range.start)
            });
            if text.as_bytes()[at..next].first() == Some(&b'\n') && before.last() == Some(&b'\r') {
                before.push(b'\r');
                repairs += 1;
            }
            assert_eq!(&before[start..], &got[..], "{text:?} {splice:?} -> {out:?}");
        }
    });
    assert!(repairs > 100, "{repairs}");
}

/// `R3_separatorPatch`: when a bare header earns its space, the first sorted
/// splice of the slot starts at the first content gap, its patch is the
/// space and then what the patch would be without it, and the space is
/// earned exactly when the first text byte changed to one that is not a
/// separator.
#[test]
fn r3_separator_patch() {
    let mut spaces = 0;
    on_batches(|page, splices, _| {
        if apply(page, splices).is_err() {
            return;
        }
        let plan = model::plan_page(page, splices);
        for (n, slot) in plan.slots.iter().enumerate() {
            let mut mine: Vec<_> = splices.iter().filter(|s| s.slot == n).collect();
            mine.sort_by_key(|s| (s.range.start, s.range.end));
            let old = page.content(n);
            let new = apply_content(&old, &mine);
            let text_byte = |s: &str| s.bytes().next().filter(|b| *b != b'\n');
            let earned = page.slots()[n].bare()
                && text_byte(&new)
                    .is_some_and(|b| Some(b) != text_byte(&old) && !tine_text::takes_separator(b));
            assert_eq!(slot.separator, earned, "{:?} {mine:?}", page.text());
            if earned {
                spaces += 1;
                assert_eq!(mine[0].range.start, 0);
                let gap = page.content_map(Owner::Slot(n)).to_file(0);
                assert_eq!(slot.patches[0].0.start, gap);
                assert_eq!(slot.patches[0].1[0], b' ');
            }
        }
    });
    assert!(spaces > 300, "{spaces}");
}

/// Rows of consecutive slices joined across a cut inside a physical line.
fn joined(owners: &[Vec<model::Row>]) -> Vec<(Vec<u8>, Break)> {
    let mut rows = Vec::new();
    let mut carry: Vec<u8> = Vec::new();
    for row in owners.iter().flatten() {
        let (body, end) = row.line();
        let body = [std::mem::take(&mut carry), body].concat();
        match end {
            Break::Continues => carry = body,
            _ => rows.push((body, end)),
        }
    }
    assert!(carry.is_empty());
    rows
}

/// `repairedPage_byteJoins`: the lexer reads the whole new file of an
/// accepted edit as every slice's planned rows in turn: no break of one
/// slice joins the next slice's first bytes into a CRLF.
#[test]
fn repaired_page_byte_joins() {
    let mut pages = 0;
    on_batches(|page, splices, _| {
        let Ok(out) = apply(page, splices) else {
            return;
        };
        let plan = model::plan_page(page, splices);
        let lexed: Vec<(Vec<u8>, Break)> = tine_text::lines(&out)
            .map(|(body, e)| (out.as_bytes()[body].to_vec(), e))
            .collect();
        assert_eq!(lexed, joined(&plan.owners), "{:?} {splices:?}", page.text());
        pages += 1;
    });
    assert!(pages > 10_000, "{pages}");
}

/// Batches outside the write's contract are rejected before the rule runs:
/// overlapping ranges, an insertion strictly inside a replaced range, two
/// insertions at one gap; touching ranges and an insertion at a range's ends
/// are separate.
#[test]
fn batches_that_are_not_separate_are_rejected() {
    let page = PageText::read(b"- abcd\n", Format::Markdown).unwrap();
    let s = |range: Range<usize>, text| Splice {
        slot: 0,
        range,
        text,
    };
    for bad in [
        [s(0..2, "x"), s(1..3, "y")],
        [s(0..3, "x"), s(1..1, "y")],
        [s(2..2, "x"), s(2..2, "y")],
        [s(1..3, "x"), s(1..3, "y")],
    ] {
        assert_eq!(apply(&page, &bad), Err(SpliceError::NotSeparate(1)));
        let reversed = [bad[1].clone(), bad[0].clone()];
        assert!(matches!(
            apply(&page, &reversed),
            Err(SpliceError::NotSeparate(_))
        ));
    }
    let good = [s(0..2, "x"), s(2..2, "y"), s(2..4, "z"), s(0..0, "w")];
    assert_eq!(apply(&page, &good).unwrap(), "- wxyz\n");
    assert_eq!(
        apply(&page, &[s(1..5, "")]),
        Err(SpliceError::OutOfRange(0))
    );
}

/// The lexer's reading of `text` as rows.
fn lexed(text: &str) -> Vec<(&[u8], Break)> {
    tine_text::lines(text)
        .map(|(body, brk)| (&text.as_bytes()[body], brk))
        .collect()
}

/// `lexicalCheck_iff`: [`check_planned_rows`] holds exactly when lexing the
/// rows' bytes gives the rows back.
#[test]
fn lexical_check_iff() {
    let mut rng = Rng(0x6c65_7869_6361_6c);
    const BODIES: &[&str] = &["", "", "a", "bc", " ", "a\rb", "\n", "x\r"];
    const BREAKS: &[Break] = &[
        Break::Lf,
        Break::CrLf,
        Break::Cr,
        Break::Eof,
        Break::Lf,
        Break::Cr,
    ];
    let (mut yes, mut no) = (0, 0);
    for _ in 0..50_000 {
        let rows: Vec<(&[u8], Break)> = (0..rng.below(5))
            .map(|_| {
                (
                    BODIES[rng.below(BODIES.len())].as_bytes(),
                    BREAKS[rng.below(BREAKS.len())],
                )
            })
            .collect();
        let bytes: String = rows
            .iter()
            .map(|(body, brk)| format!("{}{}", std::str::from_utf8(body).unwrap(), brk.as_str()))
            .collect();
        let reads_back = lexed(&bytes) == rows;
        assert_eq!(check_planned_rows(&rows), reads_back, "{rows:?}");
        if reads_back {
            yes += 1
        } else {
            no += 1
        }
    }
    assert!(yes > 5_000 && no > 5_000, "{yes} {no}");
}

/// The file-layer model's kernel-checked witnesses that this crate's reader
/// and the writer's rule-independent parts reproduce, as fixtures.
#[test]
fn model_witnesses() {
    // `* T\n\n`: one content row and one trailer row; a splice keeps it.
    let page = PageText::read(b"* T\n\n", Format::Org).unwrap();
    assert_eq!(page.content(0), "T");
    let trailer: Vec<_> = page
        .trailer(Owner::Slot(0))
        .map(|p| (p.range(), p.kind))
        .collect();
    assert_eq!(trailer, [(4..5, Break::Lf)]);
    let splice = Splice {
        slot: 0,
        range: 0..1,
        text: "U",
    };
    assert_eq!(apply(&page, &[splice]).unwrap(), "* U\n\n");
    // Three real break kinds and a nonempty last row in one slot, with its gaps.
    let mixed = "- A\n B\r C\r\n D";
    assert_eq!(
        lexed(mixed).iter().map(|r| r.1).collect::<Vec<_>>(),
        [Break::Lf, Break::Cr, Break::CrLf, Break::Eof]
    );
    let page = PageText::read(mixed.as_bytes(), Format::Markdown).unwrap();
    assert_eq!(page.content(0), "A\nB\nC\nD");
    let map = page.content_map(Owner::Slot(0));
    assert_eq!([3, 4, 5, 6].map(|i| map.to_file(i)), [6, 8, 9, 12]);
    // Adjacent ranges of one snapshot, an insertion at a replacement's start.
    let page = PageText::read(b"- ABC", Format::Markdown).unwrap();
    let plan = [
        Splice {
            slot: 0,
            range: 1..2,
            text: "Y",
        },
        Splice {
            slot: 0,
            range: 0..1,
            text: "X",
        },
        Splice {
            slot: 0,
            range: 1..1,
            text: "!",
        },
    ];
    assert_eq!(apply(&page, &plan).unwrap(), "- X!YC");
    // An empty file has no lines, and a no-op write keeps it.
    let page = PageText::read(b"", Format::Org).unwrap();
    assert_eq!(page.line_count(), 0);
    assert_eq!(apply(&page, &[]).unwrap(), "");
    // A header cut into a line body keeps every byte; its first row has no
    // removed prefix.
    let text = ":PROPERTIES:\n:key: value\n:END: # Kept\n";
    let page = PageText::read(text.as_bytes(), Format::Markdown).unwrap();
    assert!(!page.opens_line(0));
    let first = page.pieces(Owner::Slot(0)).next().unwrap();
    assert_eq!(first.layout.start, page.slots()[0].start());
    r0_holds(&page);
    // Planned rows the lexer cannot read back: a retained bare CR before an
    // emitted empty LF row (`-\r` + `\n` + `X`), a deletion that leaves a bare
    // CR before a retained LF (`-\r\n`), and an empty last row (`-\n`).
    let rows = |rows: &[(&'static str, Break)]| {
        rows.iter()
            .map(|(b, k)| (b.as_bytes(), *k))
            .collect::<Vec<_>>()
    };
    assert!(!check_planned_rows(&rows(&[
        ("-", Break::Cr),
        ("", Break::Lf),
        ("X", Break::Eof)
    ])));
    assert!(!check_planned_rows(&rows(&[
        ("-", Break::Cr),
        ("", Break::Lf)
    ])));
    assert!(!check_planned_rows(&rows(&[
        ("-", Break::Lf),
        ("", Break::Eof)
    ])));
    // The repair the model names reads back: `-\r\r\nX`.
    assert!(check_planned_rows(&rows(&[
        ("-", Break::Cr),
        ("", Break::CrLf),
        ("X", Break::Eof)
    ])));
    assert_eq!(
        lexed("-\r\r\nX"),
        rows(&[("-", Break::Cr), ("", Break::CrLf), ("X", Break::Eof)])
    );
}
/// One write fixture: a page, splices `(slot, range, replacement)`, and the
/// exact bytes [`apply`] writes or the refusal; for an accepted edit, the
/// contents lsdoc reads back (`None` where the parser places the slots
/// differently from the plan, named at the fixture).
struct Fixture {
    format: Format,
    input: &'static str,
    splices: &'static [(usize, Range<usize>, &'static str)],
    output: Result<&'static str, SpliceError>,
    read_back: Option<&'static [&'static str]>,
}

impl Fixture {
    fn check(&self) {
        let page = PageText::read(self.input.as_bytes(), self.format).unwrap();
        let mut splices: Vec<Splice<'_>> = self
            .splices
            .iter()
            .map(|(slot, range, text)| Splice {
                slot: *slot,
                range: range.clone(),
                text,
            })
            .collect();
        let want = self.output.map(str::to_string);
        assert_eq!(apply(&page, &splices), want, "{:?}", self.input);
        splices.reverse();
        assert_eq!(apply(&page, &splices), want, "{:?} reversed", self.input);
        // The model's transcription agrees.
        let model = model::checked_write(&page, &splices).map(|b| String::from_utf8(b).unwrap());
        assert_eq!(model, want.clone().ok(), "{:?} model", self.input);
        if let (Ok(out), Some(contents)) = (&want, self.read_back) {
            let again = PageText::read(out.as_bytes(), self.format).unwrap();
            let got: Vec<(u32, String)> = (0..again.slots().len())
                .map(|n| (again.slots()[n].level(), again.content(n)))
                .collect();
            let levels = page.slots().iter().map(|s| s.level());
            let want: Vec<(u32, String)> =
                levels.zip(contents.iter().map(|c| c.to_string())).collect();
            assert_eq!(got, want, "{:?} -> {out:?}", self.input);
        }
    }
}

const MD: Format = Format::Markdown;
const ORG: Format = Format::Org;

/// The model's kernel-checked witnesses of the write rule, each on a page the
/// reader reads (the model's own pages are given as byte lists with an
/// outline beside them; where the reader cannot produce that outline, the
/// fixture is the nearest page that has it, said at the fixture).
#[test]
fn model_write_rule_witnesses() {
    let fixtures = [
        // A bare header earns a space: Markdown `-<tab>Title`, Org
        // `*<form feed>Title`, Markdown `-# Heading`, insert `x` at 0.
        Fixture {
            format: MD,
            input: "-\tTitle",
            splices: &[(0, 0..0, "x")],
            output: Ok("- x\tTitle"),
            read_back: Some(&["x\tTitle"]),
        },
        Fixture {
            format: ORG,
            input: "*\x0cTitle",
            splices: &[(0, 0..0, "x")],
            output: Ok("* x\x0cTitle"),
            read_back: Some(&["x\x0cTitle"]),
        },
        Fixture {
            format: MD,
            input: "-# Heading",
            splices: &[(0, 0..0, "x")],
            output: Ok("- x# Heading"),
            read_back: Some(&["x# Heading"]),
        },
        // A new first byte the header rule takes as a separator earns no
        // space; an equal first byte keeps the header bare. The model's Org
        // `*#` is not an Org headline, so the form feed case is `-#` too.
        // The space case reads back without its space: lsdoc takes the one
        // space after the marker as prefix (a placement question).
        Fixture {
            format: MD,
            input: "-#\n",
            splices: &[(0, 0..1, " ")],
            output: Ok("- \n"),
            read_back: None,
        },
        Fixture {
            format: MD,
            input: "-#\n",
            splices: &[(0, 0..1, "\t")],
            output: Ok("-\t\n"),
            read_back: Some(&["\t"]),
        },
        Fixture {
            format: MD,
            input: "-#\n",
            splices: &[(0, 0..1, "\x0c")],
            output: Ok("-\x0c\n"),
            read_back: Some(&["\x0c"]),
        },
        Fixture {
            format: MD,
            input: "-#\n",
            splices: &[(0, 0..0, "#")],
            output: Ok("-##\n"),
            read_back: Some(&["##"]),
        },
        // Deleting the first byte exposes a different kept byte. The model's
        // `-#X` is not a header here, so the kept byte follows a tab.
        Fixture {
            format: MD,
            input: "-\tX\n",
            splices: &[(0, 0..1, "")],
            output: Ok("- X\n"),
            read_back: Some(&["X"]),
        },
        // `* T\n\n`: one content row and one trailer row, kept.
        Fixture {
            format: ORG,
            input: "* T\n\n",
            splices: &[(0, 0..1, "U")],
            output: Ok("* U\n\n"),
            read_back: Some(&["U"]),
        },
        // A bare-CR edit, with an independent next slot.
        Fixture {
            format: ORG,
            input: "* A\rline\n* Next\n",
            splices: &[(0, 1..6, "\n")],
            output: Ok("* A\n\n* Next\n"),
            read_back: Some(&["A\n", "Next"]),
        },
        // `-\n- N\n` with `x` inserted into its bare first header.
        Fixture {
            format: MD,
            input: "-\n- N\n",
            splices: &[(0, 0..0, "x")],
            output: Ok("- x\n- N\n"),
            read_back: Some(&["x", "N"]),
        },
        // A written LF after a kept bare CR gets a CR of its own (the model's
        // empty-first-body variant `\rX` has the same bytes after a header).
        Fixture {
            format: ORG,
            input: "*\rX",
            splices: &[(0, 1..1, "\n")],
            output: Ok("*\r\r\nX"),
            read_back: Some(&["\n\nX"]),
        },
        // A deletion exposes a kept LF to a kept bare CR; the CR goes at the
        // deletion's end. Org reads the file's final empty line as trailer,
        // so the Org page's read-back is the Markdown one's.
        Fixture {
            format: ORG,
            input: "*\rX\n",
            splices: &[(0, 1..2, "")],
            output: Ok("*\r\r\n"),
            read_back: None,
        },
        Fixture {
            format: MD,
            input: "-\rX\n",
            splices: &[(0, 1..2, "")],
            output: Ok("-\r\r\n"),
            read_back: Some(&["\n"]),
        },
        // With no continuation layout a final LF cannot open a line at the
        // end of the file; Markdown's continuation layout carries it.
        Fixture {
            format: ORG,
            input: "*",
            splices: &[(0, 0..0, "\n")],
            output: Err(SpliceError::EmptySliceEnd(0)),
            read_back: None,
        },
        Fixture {
            format: ORG,
            input: "*",
            splices: &[(0, 0..0, "\r")],
            output: Err(SpliceError::CarriageReturn(0)),
            read_back: None,
        },
        Fixture {
            format: MD,
            input: "-",
            splices: &[(0, 0..0, "\n")],
            output: Ok("-\n  "),
            read_back: Some(&["\n"]),
        },
        // Two touching deletions expose a kept LF; the second owns the CR.
        Fixture {
            format: ORG,
            input: "*\rXY\n",
            splices: &[(0, 1..2, ""), (0, 2..3, "")],
            output: Ok("*\r\r\n"),
            read_back: None,
        },
        Fixture {
            format: MD,
            input: "-\rXY\n",
            splices: &[(0, 1..2, ""), (0, 2..3, "")],
            output: Ok("-\r\r\n"),
            read_back: Some(&["\n"]),
        },
        // Insertions on both sides of one kept bare CR.
        Fixture {
            format: ORG,
            input: "* A\rX",
            splices: &[(0, 1..1, "\n"), (0, 2..2, "\n")],
            output: Ok("* A\n\r\r\nX"),
            read_back: Some(&["A\n\n\nX"]),
        },
        // A first and only row with no layout cannot be emptied at the end
        // of the file (an unbulleted heading has no prefix).
        Fixture {
            format: MD,
            input: "#",
            splices: &[(0, 0..1, "")],
            output: Err(SpliceError::EmptySliceEnd(0)),
            read_back: None,
        },
        // A slice cut inside a line: the first slot of `# $$x$$ # ...` is
        // the unbulleted heading `# $$x$$` (no layout), its line goes on
        // into the second header lsdoc finds inside it. Erasing all of it
        // leaves an empty row that continues, which the write refuses. Under
        // a bullet the row keeps the `- ` layout, so the same erase of
        // `- $$x$$ # ...` is accepted.
        Fixture {
            format: MD,
            input: "# $$x$$ # #+BEGIN_NOTE",
            splices: &[(0, 0..7, "")],
            output: Err(SpliceError::EmptySliceEnd(0)),
            read_back: None,
        },
        Fixture {
            format: MD,
            input: "- $$x$$ # #+BEGIN_NOTE",
            splices: &[(0, 0..5, "")],
            output: Ok("-  # #+BEGIN_NOTE"),
            read_back: None,
        },
        // A slot with no layout loses its only row at the end of its slice:
        // the second header of `- $$x$$ # #+BEGIN_NOTE` starts inside the
        // line, so it has no layout.
        Fixture {
            format: MD,
            input: "- $$x$$ # #+BEGIN_NOTE",
            splices: &[(1, 0..15, "")],
            output: Err(SpliceError::EmptySliceEnd(0)),
            read_back: None,
        },
        // The collision across a cut: the preamble ends in a bare CR, the
        // slot (an unbulleted heading, no layout) loses its bytes; only the
        // slot's write gains a CR. The heading is then gone, so no read-back.
        Fixture {
            format: MD,
            input: "a\r# X\n",
            splices: &[(0, 0..3, "")],
            output: Ok("a\r\r\n"),
            read_back: None,
        },
        // Adjacent ranges of one snapshot, an insertion at a range's start.
        Fixture {
            format: MD,
            input: "- ABC",
            splices: &[(0, 1..2, "Y"), (0, 0..1, "X"), (0, 1..1, "!")],
            output: Ok("- X!YC"),
            read_back: Some(&["X!YC"]),
        },
        // No splices: the bytes back, an empty file included.
        Fixture {
            format: ORG,
            input: "",
            splices: &[],
            output: Ok(""),
            read_back: Some(&[]),
        },
        Fixture {
            format: MD,
            input: "-",
            splices: &[],
            output: Ok("-"),
            read_back: Some(&[""]),
        },
    ];
    for fixture in &fixtures {
        fixture.check();
    }
    // The CR a deletion's repair writes belongs to the splice before the
    // exposed LF.
    let page = PageText::read(b"*\rXY\n", ORG).unwrap();
    let splices = [
        Splice {
            slot: 0,
            range: 1..2,
            text: "",
        },
        Splice {
            slot: 0,
            range: 2..3,
            text: "",
        },
    ];
    let plan = model::plan_page(&page, &splices);
    let puts: Vec<&[u8]> = plan.slots[0].patches.iter().map(|p| &p.1[..]).collect();
    assert_eq!(puts, [&b""[..], b"\r"]);
}

/// The breaks the header pages below end in: `""` for the end of the file.
const ENDINGS: [&str; 4] = ["\n", "\r\n", "\r", ""];

/// A page whose slot `slot` is `header` ended by `ending`, after `before`
/// and followed by a `next` slot when the ending is a real break.
fn header_page(before: &str, header: &str, ending: &str, next: &str) -> String {
    let mut page = format!("{before}{header}{ending}");
    if !ending.is_empty() {
        page += &format!("{next}{ending}");
    }
    page
}

/// A bare header given text gains the separator (without the rule the write is `-x`,
/// which is no header): bare dashes, indented by space, tab or form feed,
/// followed by tab or form feed, bare Org stars and a nested `**`, and the
/// headers whose content starts right after the marker, with LF, CRLF, bare
/// CR and end-of-file endings.
#[test]
fn a_bare_header_given_text_gains_a_space() {
    let cases: &[(Format, &str, &str, usize, &str)] = &[
        // (format, slots before, header, slot index, next slot)
        (MD, "", "-", 0, "- Next"),
        (MD, "", " -", 0, "- Next"),
        (MD, "", "\t-", 0, "- Next"),
        (MD, "", "\x0c-", 0, "- Next"),
        (MD, "", "-\t", 0, "- Next"),
        (MD, "", "-\x0c", 0, "- Next"),
        (MD, "", "-\tTitle", 0, "- Next"),
        (MD, "", "-# Heading", 0, "- Next"),
        (MD, "", "-## Heading", 0, "- Next"),
        (ORG, "", "*", 0, "* Next"),
        (ORG, "", "*\x0cTitle", 0, "* Next"),
        (ORG, "* a\n", "**", 1, "* Next"),
    ];
    for (format, before, header, slot, next) in cases {
        for ending in ENDINGS {
            let input = header_page(before, header, ending, next);
            let page = PageText::read(input.as_bytes(), *format).unwrap();
            assert!(page.slots()[*slot].bare(), "{input:?}");
            let marker = header.rfind(['-', '*']).unwrap() + 1;
            let (head, rest) = header.split_at(marker);
            let splice = Splice {
                slot: *slot,
                range: 0..0,
                text: "x",
            };
            let out = apply(&page, &[splice]).unwrap();
            let edited = format!("{head} x{rest}");
            assert_eq!(out, header_page(before, &edited, ending, next));
            let again = PageText::read(out.as_bytes(), *format).unwrap();
            assert_eq!(again.slots().len(), page.slots().len(), "{out:?}");
            let level = |p: &PageText<'_>| p.slots()[*slot].level();
            assert_eq!(level(&again), level(&page));
            assert_eq!(again.content(*slot), format!("x{rest}"), "{out:?}");
        }
    }
}

/// A break written or exposed next to a kept bare CR stays its own break
/// (without the repair it fuses with the CR into one CRLF): insert `"\nx"` after
/// the header text, insert `"\n"` before a bare-CR line's successor, delete
/// a line between a bare CR and an LF; under an Org headline, a Markdown
/// bullet and a Markdown heading, with an unindented continuation line.
#[test]
fn a_break_next_to_a_bare_cr_stays_a_break() {
    // (format, header, its first content line)
    for (format, header, first) in [(ORG, "* a", "a"), (MD, "- a", "a"), (MD, "# a", "# a")] {
        fn page_of(text: &str, format: Format) -> PageText<'_> {
            PageText::read(text.as_bytes(), format).unwrap()
        }
        let continuation = page_of(&format!("{header}\n"), format).continuation(0);
        let c = continuation.as_str();
        let k = first.len();
        let cases = [
            (
                format!("{header}\rb\n"),
                k..k,
                "\nx",
                format!("{header}\n{c}x\rb\n"),
                format!("{first}\nx\nb"),
            ),
            (
                format!("{header}\rb\n"),
                k + 1..k + 1,
                "\n",
                format!("{header}\r\r\n{c}b\n"),
                format!("{first}\n\nb"),
            ),
            (
                format!("{header}\rb\nc\n"),
                k + 1..k + 2,
                "",
                format!("{header}\r\r\nc\n"),
                format!("{first}\n\nc"),
            ),
        ];
        for (input, range, text, output, content) in cases {
            let page = page_of(&input, format);
            assert!(
                page.content(0).starts_with(&format!("{first}\nb")),
                "{input:?}"
            );
            let splice = Splice {
                slot: 0,
                range,
                text,
            };
            let out = apply(&page, &[splice]).unwrap();
            assert_eq!(out, output, "{input:?}");
            let again = page_of(&out, format);
            assert_eq!(again.slots().len(), 1, "{out:?}");
            assert_eq!(again.content(0), content, "{out:?}");
        }
    }
}
