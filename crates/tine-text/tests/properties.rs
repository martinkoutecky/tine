//! The physical layer's properties as tests, on the repository's page
//! corpora, on the model checks' specimens and on generated pages. Each test
//! named after a theorem of the file-layer model checks that theorem's
//! statement on this crate's reader and writer:
//!
//! - `r0`: the pieces, trailers included, concatenate to the file.
//! - `r1`: [`apply`] with no splices returns the file.
//! - `gap_total`, `gap_monotone`, `gap_strict`: a content offset maps to a file
//!   byte inside its slot; the map is monotone, strictly so on the content.
//! - `text_address_inverse`, `content_newline`: a text byte maps to itself at
//!   one file byte; a `"\n"` maps to its line's break and the next line's
//!   removed indent.
//! - `r3`, `commute`: splices keep every other byte, in order; disjoint
//!   splices give the same bytes in any order.
//! - `lexical_check_iff`: [`check_planned_rows`] holds exactly when the lexer
//!   reads the rows back.
//!
//! Content equality: every slot's content is develop's `DocBlock.raw`
//! (`develop_reader`, the frozen reader). Read-back: a splice inside plain
//! lines re-reads as the same slots, the spliced one with the new content.

mod develop_reader;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tine_text::{apply, check_planned_rows, Break, Format, Owner, PageText, Splice};

/// What one page check covered.
#[derive(Default, Debug)]
struct Seen {
    pages: usize,
    slots: usize,
    splices: usize,
    refused: usize,
    mid_line: usize,
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

/// R3 for splices inside plain lines (lowercase letters and spaces), whose
/// replacement cannot open or close a structure: a letter run is replaced by
/// letters, possibly with one `"\n"` between letters, or the separator of two
/// plain lines is removed.
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
        seen.splices += 1;
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
        let line = match rng.below(16) {
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
    let mut seen = Seen::default();
    for _ in 0..20_000 {
        let (text, format) = generate(&mut rng);
        if let Some(page) = check_read(&text, format, &mut seen) {
            check_splices(&page, &mut rng, &mut seen);
        }
    }
    println!("generated: {seen:?} in {:?}", started.elapsed());
    assert_eq!(seen.pages, 20_000);
    assert!(seen.splices > 10_000 && seen.mid_line > 100, "{seen:?}");
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
                check_splices(&page, &mut rng, &mut seen);
            }
        }
    }
    println!(
        "corpora: {} files, {not_utf8} not UTF-8, {seen:?}",
        files.len()
    );
    assert!(seen.slots > 2_000 && seen.splices > 1_000, "{seen:?}");
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

/// Up to `count` splices of one snapshot, pairwise separate in the model's
/// sense: distinct ranges that do not overlap, so an insertion may touch a
/// range's end but not lie inside it, and no two insertions share a gap.
fn separate_splices(
    page: &PageText<'_>,
    rng: &mut Rng,
    count: usize,
) -> Vec<(Splice<'static>, std::ops::Range<usize>)> {
    const REPLACEMENTS: &[&str] = &["", "x", "yz", "\n", "a\nb", "\n\n", " é ", "q\n  r", "\t"];
    let mut chosen: Vec<(Splice<'static>, std::ops::Range<usize>)> = Vec::new();
    for _ in 0..count {
        let n = rng.below(page.slots().len());
        let content = page.content(n);
        let (a, b) = (boundary(&content, rng), boundary(&content, rng));
        let range = a.min(b)..a.max(b);
        let map = page.content_map(Owner::Slot(n));
        let file = map.to_file(range.start)..map.to_file(range.end);
        let separate = chosen.iter().all(|(_, other)| {
            (other.start, other.end) != (file.start, file.end)
                && (other.end <= file.start || file.end <= other.start)
        });
        if separate {
            let text = REPLACEMENTS[rng.below(REPLACEMENTS.len())];
            chosen.push((
                Splice {
                    slot: n,
                    range,
                    text,
                },
                file,
            ));
        }
    }
    chosen
}

/// The bytes `apply` writes for one splice alone, and its file range.
fn written(page: &PageText<'_>, splice: &Splice<'_>, file: &std::ops::Range<usize>) -> String {
    let text = page.text();
    let out = apply(page, std::slice::from_ref(splice)).unwrap();
    assert_eq!(&out[..file.start], &text[..file.start]);
    assert!(out.ends_with(&text[file.end..]));
    out[file.start..out.len() - (text.len() - file.end)].to_string()
}

/// `R3`: the old file is kept chunks and cut chunks in turn, the new file the
/// same kept chunks and the written chunks in turn; each cut chunk is the file
/// between its range's gaps; each written chunk is the replacement with every
/// `"\n"` written as a break and one continuation indent. Its premises (the
/// slot has a first row; the plan is ordered and inside the content) are, in
/// order, true of every slot read from bytes and checked by `apply`, which
/// refuses an out-of-range or overlapping plan.
#[test]
fn r3() {
    let mut splices = 0;
    on_cases(|page, rng| {
        if page.slots().is_empty() {
            return;
        }
        let text = page.text();
        let count = 1 + rng.below(4);
        let mut plan = separate_splices(page, rng, count);
        plan.sort_by_key(|(_, file)| (file.start, file.end));
        let mut expected = String::new();
        let mut at = 0;
        for (splice, file) in &plan {
            let put = written(page, splice, file);
            let lf = tine_text::lf_form(&put);
            let (ours, theirs): (Vec<_>, Vec<_>) =
                (lf.split('\n').collect(), splice.text.split('\n').collect());
            assert_eq!(ours.len(), theirs.len(), "{text:?} {splice:?}");
            assert_eq!(ours[0], theirs[0]);
            let indent = ours.get(1).map(|o| &o[..o.len() - theirs[1].len()]);
            for (o, t) in ours.iter().zip(&theirs).skip(1) {
                assert_eq!(*o, format!("{}{t}", indent.unwrap()), "{text:?} {splice:?}");
            }
            expected.push_str(&text[at..file.start]);
            expected.push_str(&put);
            at = file.end;
        }
        expected.push_str(&text[at..]);
        let splices_only: Vec<_> = plan.iter().map(|(s, _)| s.clone()).collect();
        assert_eq!(apply(page, &splices_only).unwrap(), expected, "{text:?}");
        splices += plan.len();
    });
    assert!(splices > 5_000, "{splices}");
}

/// `commute`: a separate plan gives the same bytes in any order.
#[test]
fn commute() {
    on_cases(|page, rng| {
        if page.slots().is_empty() {
            return;
        }
        let count = 2 + rng.below(4);
        let mut plan: Vec<_> = separate_splices(page, rng, count)
            .into_iter()
            .map(|(s, _)| s)
            .collect();
        let first = apply(page, &plan).unwrap();
        for _ in 0..3 {
            for i in (1..plan.len()).rev() {
                plan.swap(i, rng.below(i + 1));
            }
            assert_eq!(
                apply(page, &plan).unwrap(),
                first,
                "{:?} {plan:?}",
                page.text()
            );
        }
    });
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

/// The model's witnesses for the write rule, which this crate's writer does
/// not follow yet: a bare-CR line's new break, and the separator after a bare
/// header. They pass once the write rule lands.
#[test]
#[ignore = "the write rule for breaks and bare headers is not implemented yet"]
fn model_write_rule_witnesses() {
    let page = PageText::read(b"* A\rline\n* Next\n", Format::Org).unwrap();
    let splice = Splice {
        slot: 0,
        range: 1..6,
        text: "\n",
    };
    assert_eq!(apply(&page, &[splice]).unwrap(), "* A\n\n* Next\n");
    let page = PageText::read(b"-\n- N\n", Format::Markdown).unwrap();
    let splice = Splice {
        slot: 0,
        range: 0..0,
        text: "x",
    };
    assert_eq!(apply(&page, &[splice]).unwrap(), "- x\n- N\n");
}
