use tine_core::doc::{self, SerializeOpts};

#[test]
fn formatting_detection_uses_only_parser_owned_headers() {
    let source = "title:: Layout\n\n- ```\n      - literal bullet\n  ```\n- after\n";
    let opts = SerializeOpts::detect(Some(source));
    assert_eq!(
        opts.indent, "\t",
        "I-12: only lsdoc headers reveal layout; exemplar doc::parse_with_opts"
    );
}

#[test]
fn parse_and_layout_share_the_outline_for_all_line_endings() {
    for eol in ["\n", "\r\n", "\r"] {
        for source in [
            "title:: Layout\n- first\n  - nested\n\n",
            "title:: Layout\n\n- first\n\t- nested",
            "- only\n",
            "",
            "preamble\n",
        ] {
            let input = source.replace('\n', eol);
            let (document, opts) = doc::parse_with_opts(&input);
            assert_eq!(document, doc::parse(&input));
            let detected = SerializeOpts::detect(Some(&input));
            assert_eq!(opts.indent, detected.indent);
            assert_eq!(opts.blank_after_props, detected.blank_after_props);
            assert_eq!(opts.trailing_newlines, detected.trailing_newlines);
            let reparsed = doc::parse(&doc::serialize_with(&document, &opts));
            assert_eq!(reparsed, document);
        }
    }
}

/// Save preparation reads the page's lines once: `parse_with_opts` asks the
/// allocator for no more than `parse` does plus the knobs, on a block of
/// 100,000 continuation lines, with LF, CRLF and bare-CR endings.
#[test]
fn layout_detection_reads_the_lines_the_reader_already_has() {
    use crate::og_p10_journal_date_cost::requested_bytes;
    let block = format!("- a\n{}", "  x\n".repeat(100_000));
    for eol in ["\n", "\r\n", "\r"] {
        let input = block.replace('\n', eol);
        let (_, parse) = requested_bytes(|| doc::parse(&input));
        let (_, with_opts) = requested_bytes(|| doc::parse_with_opts(&input));
        println!("{eol:?}: parse {parse} bytes, parse_with_opts {with_opts}");
        assert!(
            with_opts <= parse + 1024,
            "{eol:?}: parse_with_opts asked for {with_opts} bytes, parse {parse}"
        );
    }
}
