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
