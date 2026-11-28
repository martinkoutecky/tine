//! The physical layer's properties R0–R3 as tests, on the repository's page
//! corpora, on the model checks' specimens and on generated pages.
//!
//! - R0: the pieces concatenate to the file.
//! - R1: [`apply`] with no splices returns the file.
//! - Content equality: every slot's content is develop's `DocBlock.raw`
//!   (`develop_reader`, the frozen reader), and the content map round-trips.
//! - R3: after a splice every byte outside the mapped range is the file's, and
//!   re-reading gives the same slots, the spliced one with the new content.

mod develop_reader;

use std::path::{Path, PathBuf};
use tine_text::{apply, Break, Format, Owner, PageText, Splice};

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

/// R0, R1, content equality and the map, for one page.
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
    // R0: preamble, slots and tail concatenate to the file, piece by piece.
    let mut at = 0;
    let owners = std::iter::once(Owner::Preamble).chain((0..page.slots().len()).map(Owner::Slot));
    for owner in owners {
        assert_eq!(page.range(owner).start, at, "{owner:?} of {text:?}");
        for piece in page.pieces(owner) {
            assert_eq!(piece.layout.start, at, "{text:?}");
            assert_eq!(piece.layout.end, piece.text.start);
            assert_eq!(piece.text.end, piece.brk.start);
            assert_eq!(&text[piece.brk.clone()], piece.kind.as_str(), "{text:?}");
            assert!(!text[piece.text.clone()].contains(['\n', '\r']), "{text:?}");
            at = piece.brk.end;
        }
        assert_eq!(page.range(owner).end, at, "{owner:?} of {text:?}");
    }
    assert_eq!(page.tail().start, at);
    assert!(text[page.tail()].bytes().all(|b| b == b'\n' || b == b'\r'));
    // R1.
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
    for n in 0..page.slots().len() {
        seen.slots += 1;
        check_map(&page, n);
    }
    check_breaks_against_lsdoc(&page);
    Some(page)
}

/// The content map is total and round-trips every content offset; text bytes
/// map to themselves.
fn check_map(page: &PageText<'_>, n: usize) {
    let text = page.text();
    let content = page.content(n);
    let map = page.content_map(n);
    assert_eq!(map.len(), content.len());
    let mut last = None;
    for at in 0..=content.len() {
        let file = map.to_file(at);
        assert_eq!(map.to_content(file), at, "slot {n} offset {at} of {text:?}");
        assert!(last.is_none_or(|l| l < file), "{text:?}");
        last = Some(file);
        if at < content.len() && content.as_bytes()[at] != b'\n' {
            assert_eq!(text.as_bytes()[file], content.as_bytes()[at]);
        }
    }
    for file in page.range(Owner::Slot(n)) {
        let at = map.to_content(file);
        assert!(at <= content.len());
        let in_text = map.pieces().iter().any(|(_, p)| p.text.contains(&file));
        if in_text {
            assert_eq!(map.to_file(at), file);
        }
    }
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
        let map = page.content_map(n);
        let pieces = map.pieces();
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
/// file that is one empty line. Those bytes are the tail.
#[test]
fn the_tail_is_what_the_reader_does_not_read() {
    let page = PageText::read(b"* a\nb\n\r\n\n", Format::Org).unwrap();
    assert_eq!(page.content(0), "a\nb");
    assert_eq!(page.tail(), 6..9);
    let page = PageText::read(b"- a\nb\n\n", Format::Markdown).unwrap();
    assert_eq!(page.content(0), "a\nb\n");
    assert_eq!(page.tail(), 7..7);
    assert_eq!(
        PageText::read(b"\r\n", Format::Markdown).unwrap().tail(),
        0..2
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
