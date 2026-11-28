//! `physical_lines` and `line_terminator`: the physical writers' one answer
//! to "which bytes end a line" agrees with the reader's normalization.
use super::{line_terminator, normalize_line_endings, physical_lines};

#[test]
fn physical_lines_end_where_the_reader_ends_a_line() {
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
        let lines: Vec<_> = physical_lines(text).collect();
        assert_eq!(lines.concat(), text, "{text:?}");
        // Every line but the last is terminated, by exactly one terminator.
        for (n, line) in lines.iter().enumerate() {
            let end = line_terminator(line);
            assert_eq!(
                end.is_empty(),
                n + 1 == lines.len() && !text.ends_with(['\r', '\n'])
            );
            assert!(
                !line[..line.len() - end.len()].contains(['\r', '\n']),
                "{line:?}"
            );
        }
        // The reader's LF form has the same lines.
        let lf = normalize_line_endings(text);
        assert_eq!(lines.len(), lf.split_inclusive('\n').count(), "{text:?}");
    }
    assert_eq!(line_terminator("x\r"), "\r");
    assert_eq!(line_terminator("x\r\n"), "\r\n");
    assert_eq!(line_terminator("x\n"), "\n");
    assert_eq!(line_terminator("x"), "");
}

#[test]
fn mapped_markdown_keeps_trimmed_preamble_layout_and_native_offsets() {
    let input = "pre\r\n \t\r- α\r\n \ttrailing\r  \r\n- Next";
    let read = super::parse_outlined(input, false, true);
    let map = read.map.unwrap();
    assert_eq!(read.document.pre_block.as_deref(), Some("pre"));
    assert_eq!(read.document.roots[0].raw(), "α\ntrailing\n");
    assert_eq!(read.document.roots[1].raw(), "Next");
    assert_eq!(map.pre_block(), 0..1);
    assert_eq!(map.block(0), 2..5);
    assert_eq!(map.block(1), 5..6);
    assert_eq!(map.removed(2), 2);
    assert_eq!(map.removed(3), 2);
    assert_eq!(map.offset(3), 3);
    assert_eq!(map.native(Some(0), 3), input.find("trailing").unwrap());
    assert_eq!(&input[map.line(4)], "  \r\n");
    assert_eq!(map.read_end(), input.len());
}

#[test]
fn mapped_org_keeps_native_trailer_beyond_the_reader_end() {
    let input = "\r\n* T\r\n body\r\n \t\r\n\r\n\r";
    let read = super::parse_outlined(input, true, true);
    let map = read.map.unwrap();
    assert_eq!(read.document.pre_block.as_deref(), Some(""));
    assert_eq!(read.document.roots[0].raw(), "T\n body\n \t");
    assert_eq!(map.pre_block(), 0..1);
    assert_eq!(map.block(0), 1..4);
    assert_eq!(map.native(Some(0), 2), input.find(" body").unwrap());
    assert_eq!(&input[map.read_end()..], "\r\n\r");
    assert_eq!(map.lines(), 0..6);
    assert_eq!(map.removed(5), 0);
}

#[test]
fn mapped_midline_header_keeps_the_full_line_as_preamble() {
    let input = ":PROPERTIES:\r\n:key: value\r:END: # Kept\nnext:: property\r\n";
    let read = super::parse_outlined(input, false, true);
    let map = read.map.unwrap();
    assert!(read.document.roots.is_empty());
    assert_eq!(
        read.document.pre_block.as_deref(),
        Some(":PROPERTIES:\n:key: value\n:END: # Kept\nnext:: property")
    );
    assert_eq!(map.pre_block(), 0..4);
    assert_eq!(
        map.line(2),
        input.find(":END:").unwrap()..input.find("next::").unwrap()
    );
    assert_eq!(&input[map.payload(2)], ":END: # Kept");
    assert_eq!(
        map.native(None, map.offset(2) + 5),
        input.find(":END:").unwrap() + 5
    );
    assert_eq!(map.read_end(), input.len());
}
