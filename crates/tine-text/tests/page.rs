mod develop;

use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};
use tine_text::{
    apply, BreakKind, Format, Line, OutlineRefusal, PageText, ReadError, Splice, SpliceError,
};

const MIXED: [&str; 2] = [
    "- T\r\n  old\n  tail\r\n- Next\n",
    "- T\n  old\r\n  tail\r\n- Next\n",
];
const MIDLINE: &str = ":PROPERTIES:\n:key: value\n:END: # Kept\nnext:: property\n";
const OVERLAP: &str = "- $$x$$ # #+BEGIN_NOTE\r\nx\r\n#+END_NOTE";

fn pieces<'p>(page: &'p PageText<'_>) -> impl Iterator<Item = Line> + 'p {
    page.preamble()
        .iter()
        .chain(
            page.slots()
                .flat_map(|slot| slot.lines().iter().chain(slot.trailer())),
        )
        .copied()
}

fn r0_r1(page: &PageText<'_>) {
    let input = page.source().as_bytes();
    let mut prev = 0;
    for line in pieces(page) {
        for range in [line.layout(), line.text(), line.line_break()] {
            assert_eq!(prev, range.start, "{input:?}");
            assert!(input.get(range.clone()).is_some(), "{input:?}");
            prev = range.end;
        }
        let kind = match &input[line.line_break()] {
            b"\n" => BreakKind::Lf,
            b"\r\n" => BreakKind::CrLf,
            b"\r" => BreakKind::Cr,
            b"" if prev == input.len() => BreakKind::Eof,
            b"" => BreakKind::Boundary,
            _ => panic!("unexpected line break: {input:?}"),
        };
        assert_eq!(line.break_kind(page), kind);
    }
    assert_eq!(prev, input.len());
    assert_eq!(apply(page, &[]).unwrap(), input);
}

fn develop_content(page: &PageText<'_>, format: Format) {
    let expected = develop::read(page.source(), format);
    let got: Vec<_> = page
        .headers()
        .zip(page.slots())
        .map(|(header, slot)| {
            assert_eq!(Some(header.level), slot.level());
            (header.level, slot.content())
        })
        .collect();
    assert_eq!(got, expected, "{:?}", page.source());

    let lf = tine_text::normalize_line_endings(page.source());
    let body = match format {
        Format::Org => lf.trim_end_matches('\n'),
        Format::Markdown => lf.strip_suffix('\n').unwrap_or(&lf),
    };
    let lines: Vec<_> = body.split('\n').collect();
    let headers: Vec<_> = page.headers().collect();
    let got: Vec<_> = tine_text::block_contents(&lines, &headers, None).collect();
    assert_eq!(got, expected, "{:?}", page.source());
}

fn map_and_r3(page: &PageText<'_>, salt: usize) {
    let source = page.source();
    for (id, slot) in page.slots().enumerate() {
        if slot.is_opaque() {
            let start = slot.trailer()[0].layout().start;
            let end = slot.trailer().last().unwrap().line_break().end;
            assert_eq!(slot.file_offset(0), Some(start));
            assert!((start..=end).all(|native| slot.content_offset(native) == Some(0)));
            continue;
        }
        let content = slot.content();
        let mut prev = None;
        for at in 0..=content.len() {
            let native = slot.file_offset(at).unwrap();
            assert!(prev.is_none_or(|prev| prev < native), "{source:?}");
            assert_eq!(
                slot.content_offset(native),
                Some(at),
                "{source:?}, {content:?}"
            );
            if at < content.len() {
                let byte = source.as_bytes()[native];
                let expected = if byte == b'\r' { b'\n' } else { byte };
                assert_eq!(content.as_bytes()[at], expected, "{source:?}");
            }
            prev = Some(native);
        }
        let first = slot.lines()[0].layout().start;
        let last = slot
            .trailer()
            .last()
            .or_else(|| slot.lines().last())
            .unwrap()
            .line_break()
            .end;
        let mut prev = 0;
        for native in first..=last {
            let mapped = slot.content_offset(native).unwrap();
            assert!((prev..=content.len()).contains(&mapped), "{source:?}");
            prev = mapped;
        }
        for line in slot.lines() {
            let at = line.content_offset();
            assert!(line
                .layout()
                .all(|native| slot.content_offset(native) == Some(at)));
            let at_break = at + line.text().len();
            assert!(line
                .line_break()
                .all(|native| slot.content_offset(native) == Some(at_break)));
        }
        for line in slot.trailer() {
            assert!((line.layout().start..line.line_break().end)
                .all(|native| slot.content_offset(native) == Some(content.len())));
        }
        let boundaries: Vec<_> = content
            .char_indices()
            .map(|(at, _)| at)
            .chain([content.len()])
            .collect();
        let a = boundaries[salt % boundaries.len()];
        let b = boundaries[(salt + id + content.len() / 3) % boundaries.len()];
        let (start, end) = (a.min(b), a.max(b));
        let native_start = slot.file_offset(start).unwrap();
        let native_end = slot.file_offset(end).unwrap();
        let result = apply(
            page,
            &[Splice {
                slot: id,
                range: start..end,
                replacement: ["", "Z", "U\nV", "\n"][salt % 4],
            }],
        )
        .unwrap();
        assert_eq!(
            &result[..native_start],
            &source.as_bytes()[..native_start],
            "{source:?}"
        );
        assert!(
            result.ends_with(&source.as_bytes()[native_end..]),
            "{source:?}"
        );
        assert!(
            result.len() >= native_start + source.len() - native_end,
            "{source:?}"
        );
    }
}

fn properties(source: &str, format: Format, salt: usize) {
    let page = PageText::read(source.as_bytes(), format).unwrap();
    r0_r1(&page);
    develop_content(&page, format);
    map_and_r3(&page, salt);
}

#[test]
fn r0_r1_r2_r3_model_specimens() {
    for (n, source) in MIXED
        .into_iter()
        .chain([
            MIDLINE,
            OVERLAP,
            "Intro\n-",
            "-\n- Next\n",
            "# lf\n- crlf\r\n  - cr\r# eof",
            "intro\n \t\n- T\n",
            "-\n\t x",
            "\r\r\n",
        ])
        .enumerate()
    {
        properties(source, Format::Markdown, n);
    }
    for (n, source) in [
        "* T\n\n",
        "* A\rline\n* Next\n",
        "\n\r\n",
        "*",
        "* T\n \t\n\r\n",
    ]
    .into_iter()
    .enumerate()
    {
        properties(source, Format::Org, n);
    }
    let page = PageText::read(MIDLINE.as_bytes(), Format::Markdown).unwrap();
    let slot = page.slots().next().unwrap();
    assert_eq!(slot.lines()[0].layout(), 30..30);
    assert_eq!(slot.content(), " # Kept\nnext:: property");
    assert_eq!(
        page.preamble().last().unwrap().break_kind(&page),
        BreakKind::Boundary
    );
    assert!(matches!(
        page.outline_refusal(),
        Some(OutlineRefusal::NotOnePerLine { .. })
    ));
    let page = PageText::read(OVERLAP.as_bytes(), Format::Markdown).unwrap();
    let mut slots = page.slots();
    assert!(slots.next().unwrap().is_opaque());
    assert!(slots.next().is_none());
    assert_eq!(
        apply(
            &page,
            &[Splice {
                slot: 0,
                range: 0..0,
                replacement: "x"
            }]
        ),
        Err(SpliceError::Opaque { edit: 0 })
    );
    let page = PageText::read(b"* T\n\n", Format::Org).unwrap();
    let slot = page.slots().next().unwrap();
    assert_eq!(slot.content(), "T");
    assert_eq!(slot.trailer()[0].line_break(), 4..5);
}

#[test]
fn r0_breaks_match_the_pinned_lexer() {
    let source = "# lf\n- crlf\r\n  - cr\r# eof";
    let page = PageText::read(source.as_bytes(), Format::Markdown).unwrap();
    let expected = lsdoc::parse_outline(source, "md").unwrap().headers;
    assert_eq!(expected.len(), 4);
    for (slot, header) in page.slots().zip(expected) {
        assert_eq!(
            slot.lines()[0].line_break(),
            header.line_content.end..header.line.end
        );
    }
    assert_eq!(
        pieces(&page)
            .map(|line| line.break_kind(&page))
            .collect::<Vec<_>>(),
        [
            BreakKind::Lf,
            BreakKind::CrLf,
            BreakKind::Cr,
            BreakKind::Eof
        ]
    );
}

#[test]
fn r3_named_ranges_and_readback() {
    for (source, format, range, replacement, expected, expected_blocks) in [
        (
            MIXED[0],
            Format::Markdown,
            2..5,
            "new",
            "- T\r\n  new\n  tail\r\n- Next\n",
            &["T\nnew\ntail", "Next"][..],
        ),
        (
            MIXED[1],
            Format::Markdown,
            2..5,
            "new",
            "- T\n  new\r\n  tail\r\n- Next\n",
            &["T\nnew\ntail", "Next"],
        ),
        (
            MIXED[0],
            Format::Markdown,
            5..5,
            "\nnew",
            "- T\r\n  old\n  new\n  tail\r\n- Next\n",
            &["T\nold\nnew\ntail", "Next"],
        ),
        (
            "- T\r\n- Next\n",
            Format::Markdown,
            1..1,
            "\nnew\nmore",
            "- T\r\n  new\r\n  more\r\n- Next\n",
            &["T\nnew\nmore", "Next"],
        ),
        (
            "* A\rline\n* Next\n",
            Format::Org,
            1..6,
            "\n",
            "* A\n\n* Next\n",
            &["A\n", "Next"],
        ),
        (
            "* A\rline\n\n",
            Format::Org,
            1..6,
            "\n",
            "* A\n\n\n",
            &["A"],
        ),
        (
            "-\n- Next\n",
            Format::Markdown,
            0..0,
            "x",
            "- x\n- Next\n",
            &["x", "Next"],
        ),
        ("-\n  x", Format::Markdown, 0..1, "", "- x", &["x"]),
        (
            "*\n* Next\n",
            Format::Org,
            0..0,
            "x",
            "* x\n* Next\n",
            &["x", "Next"],
        ),
        (
            "\t- T\r\n- Next\n",
            Format::Markdown,
            1..1,
            "\rnew",
            "\t- T\r\n\t  new\r\n- Next\n",
            &["T\nnew", "Next"],
        ),
    ] {
        let page = PageText::read(source.as_bytes(), format).unwrap();
        let written = apply(
            &page,
            &[Splice {
                slot: 0,
                range,
                replacement,
            }],
        )
        .unwrap();
        assert_eq!(written, expected.as_bytes(), "{source:?}");
        let reread = PageText::read(&written, format).unwrap();
        assert_eq!(
            reread.slots().map(|s| s.content()).collect::<Vec<_>>(),
            expected_blocks,
            "{source:?}"
        );
        for (before, after) in page.slots().skip(1).zip(reread.slots().skip(1)) {
            assert_eq!(before.level(), after.level());
            let raw = |s: tine_text::SlotText<'_>| {
                s.lines()[0].layout().start..s.lines().last().unwrap().line_break().end
            };
            assert_eq!(&page.source()[raw(before)], &reread.source()[raw(after)]);
        }
    }
}

#[test]
fn r3_multiple_splices_and_invalid_coordinates() {
    let page = PageText::read("-\n- α old and kept\r\n".as_bytes(), Format::Markdown).unwrap();
    let edits = [
        Splice {
            slot: 1,
            range: 7..7,
            replacement: "only ",
        },
        Splice {
            slot: 0,
            range: 0..0,
            replacement: "a",
        },
        Splice {
            slot: 1,
            range: 3..6,
            replacement: "new",
        },
        Splice {
            slot: 0,
            range: 0..0,
            replacement: "b",
        },
    ];
    assert_eq!(
        apply(&page, &edits).unwrap(),
        "- ab\n- α new only and kept\r\n".as_bytes()
    );
    assert_eq!(
        apply(
            &page,
            &[Splice {
                slot: 1,
                range: 1..2,
                replacement: "x"
            }]
        ),
        Err(SpliceError::Range { edit: 0 })
    );
    assert_eq!(
        apply(
            &page,
            &[Splice {
                slot: 2,
                range: 0..0,
                replacement: "x"
            }]
        ),
        Err(SpliceError::Slot { edit: 0 })
    );
    assert_eq!(
        apply(
            &page,
            &[
                Splice {
                    slot: 1,
                    range: 2..5,
                    replacement: ""
                },
                Splice {
                    slot: 1,
                    range: 3..6,
                    replacement: ""
                }
            ]
        ),
        Err(SpliceError::Overlap { edit: 1 })
    );
}

#[test]
fn develop_refusals() {
    assert!(matches!(
        PageText::read(b"- \xff", Format::Markdown),
        Err(ReadError::Encoding(_))
    ));
    let text = format!("- {}x", "> ".repeat(129));
    assert!(matches!(
        PageText::read(text.as_bytes(), Format::Markdown),
        Err(ReadError::SourceDepth)
    ));
    let fenced = format!("```\n{}x\n```\n- T\n", "> ".repeat(129));
    let page = PageText::read(fenced.as_bytes(), Format::Markdown).unwrap();
    assert_eq!(page.outline_refusal(), Some(OutlineRefusal::Unowned));
    assert_eq!(page.slots().len(), 0);
    r0_r1(&page);
    develop_content(&page, Format::Markdown);
    let text = format!("{} x", "*".repeat(129));
    assert!(matches!(
        PageText::read(text.as_bytes(), Format::Org),
        Err(ReadError::OrgHeadlineDepth)
    ));
    let large = vec![0; tine_text::PAGE_MAX_BYTES as usize + 1];
    assert!(matches!(
        PageText::read(&large, Format::Markdown),
        Err(ReadError::TooLarge { .. })
    ));
}

fn walk(dir: &Path, file: &mut impl FnMut(&Path, Format)) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            walk(&path, file);
        } else if entry.file_type().unwrap().is_file() {
            match path.extension().and_then(|ext| ext.to_str()) {
                Some("md") => file(&path, Format::Markdown),
                Some("org") => file(&path, Format::Org),
                _ => {}
            }
        }
    }
}

fn corpus_file(path: &Path, format: Format, salt: usize, splice: bool) {
    let bytes = fs::read(path).unwrap();
    match PageText::read(&bytes, format) {
        Ok(page) => {
            r0_r1(&page);
            develop_content(&page, format);
            if splice {
                map_and_r3(&page, salt);
            }
        }
        Err(ReadError::Encoding(_)) => {
            assert!(std::str::from_utf8(&bytes).is_err(), "{}", path.display())
        }
        Err(ReadError::TooLarge { len }) => {
            assert!(len > tine_text::PAGE_MAX_BYTES, "{}", path.display())
        }
        Err(ReadError::SourceDepth) => {
            let text = std::str::from_utf8(&bytes).unwrap();
            assert!(
                !tine_text::parse_input_depth_within_limit(text),
                "{}",
                path.display()
            );
        }
        Err(ReadError::OrgHeadlineDepth) => {
            let text = std::str::from_utf8(&bytes).unwrap();
            assert_eq!(format, Format::Org, "{}", path.display());
            assert!(
                develop::read(text, format)
                    .iter()
                    .any(|(level, _)| *level as usize > tine_text::SOURCE_DEPTH_MAX),
                "{}",
                path.display()
            );
        }
    }
}

#[test]
fn r0_r1_r2_r3_public_corpora() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut count = 0;
    for dir in [
        "src/mock",
        "src/fixtures",
        "scripts/fixtures",
        "tests",
        "fixtures",
        "samples/demo-graph",
        "crates/tine-core/src/query/fixtures",
        "crates/tine-core/tests",
        "crates/tine-store/tests",
        "crates/tine-graph-features/tests",
    ] {
        walk(&root.join(dir), &mut |path, format| {
            corpus_file(path, format, count, true);
            count += 1;
        });
    }
    assert!(count >= 150, "{count} corpus files");
    println!("Checked {count} public corpus files");
}

#[test]
#[ignore = "requires a local TINE_TEXT_CORPUS directory"]
fn r0_r1_and_develop_content_private_corpus() {
    let dir =
        std::env::var_os("TINE_TEXT_CORPUS").expect("set TINE_TEXT_CORPUS to the corpus directory");
    let mut count = 0;
    walk(Path::new(&dir), &mut |path, format| {
        corpus_file(path, format, count, false);
        count += 1;
    });
    assert!(count > 0, "corpus contains no Markdown or Org files");
    println!("Checked {count} corpus files");
}

struct Rng(u64);

impl Rng {
    fn next(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound as u64) as usize
    }
}

#[test]
fn r0_r1_r2_r3_generated_pages() {
    let start = Instant::now();
    let mut rng = Rng(0x5eed_7e57);
    let eol = ["\n", "\r\n", "\r"];
    for n in 0..20_000 {
        let format = if rng.next(2) == 0 {
            Format::Markdown
        } else {
            Format::Org
        };
        let mut text = String::new();
        if n % 73 == 0 && format == Format::Markdown {
            text.push_str(if n % 2 == 0 { MIDLINE } else { OVERLAP });
            text.push_str(eol[rng.next(3)]);
        }
        let count = rng.next(12);
        for i in 0..count {
            let indent = ["", "\t", " ", "  ", "\t ", " \t ", "\x0c "][rng.next(7)];
            let level = 1 + rng.next(5);
            match rng.next(8) {
                0 | 1 | 2 if format == Format::Org => {
                    text.push_str(&"*".repeat(level));
                    text.push_str(["", " Tiếng", " Ωβ", " item"][rng.next(4)]);
                }
                0 | 1 | 2 => {
                    text.push_str(&indent.repeat(level - 1));
                    text.push_str(["-", "- item", "- éα", "# Heading", "-\tTabbed"][rng.next(5)]);
                }
                3 => text.push_str("phrase - mid-line # heading * headline"),
                4 => text.push_str(indent),
                5 => {}
                6 => {
                    text.push_str(indent);
                    text.push_str("kept Ωβ");
                }
                _ => text.push_str("alias:: X [[α]]"),
            }
            if i + 1 < count || rng.next(2) != 0 {
                text.push_str(eol[rng.next(3)]);
            }
        }
        properties(&text, format, n);
    }
    let elapsed = start.elapsed();
    println!("20,000 generated pages in {elapsed:?}");
    assert!(elapsed < Duration::from_secs(30), "{elapsed:?}");
}
