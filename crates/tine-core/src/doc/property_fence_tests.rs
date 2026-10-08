use super::*;
#[test]
fn property_lines_skip_fenced_key_colons() {
    // A `key:: value` line inside a code fence is literal content, not a block
    // property — it must not become a chip / match a (property …) query, and it
    // must stay in the visible (searchable) text.
    let b = DocBlock::new("title:: Real\n```\nlang:: rust\nlet x = 1;\n```\nfoo:: bar");
    let props = b.properties();
    assert!(props.iter().any(|(k, _)| k == "title"));
    assert!(props.iter().any(|(k, _)| k == "foo"));
    assert!(
        !props.iter().any(|(k, _)| k == "lang"),
        "fenced lang:: is not a property: {props:?}"
    );
    assert_eq!(b.property("lang"), None);
    // The fenced property line stays visible (it's code); real props are dropped.
    let vis = b.visible_folded(true).to_owned();
    assert!(
        vis.contains("lang:: rust"),
        "fenced line searchable: {vis:?}"
    );
    assert!(
        !vis.contains("title:: real"),
        "real property dropped from visible text"
    );
}

#[test]
fn parse_normalizes_crlf_to_lf() {
    let crlf = parse("title:: x\r\n\r\n- a\r\n- b\r\n");
    // No stray CR leaks into the model (would otherwise pollute property/id values).
    assert_eq!(crlf.pre_block.as_deref(), Some("title:: x"));
    for b in &crlf.roots {
        assert!(!b.raw.contains('\r'), "stray CR in block raw: {:?}", b.raw);
    }
    // CRLF parses to the same model as LF; serialize is LF-canonical.
    let lf = parse("title:: x\n\n- a\n- b\n");
    assert_eq!(serialize(&crlf), serialize(&lf));
    assert!(!serialize(&crlf).contains('\r'));
}

#[test]
fn detect_trailing_newlines_is_crlf_robust() {
    assert_eq!(
        SerializeOpts::detect(Some("- a\r\n\r\n")).trailing_newlines,
        2
    );
    assert_eq!(SerializeOpts::detect(Some("- a\r\n")).trailing_newlines, 1);
    assert_eq!(SerializeOpts::detect(Some("- a\n\n")).trailing_newlines, 2);
}

#[test]
fn lone_cr_and_mixed_endings_parse_like_lf() {
    // K01a (og 15a): mldoc and lsdoc end a line at a lone `\r` too.
    let lf = "title:: x\n\n- a\n\t- b\n  cont\n- c\n\n";
    for ending in ["\r", "\r\n"] {
        let other = lf.replace('\n', ending);
        assert_eq!(parse(&other), parse(lf), "{ending:?}");
        let opts = SerializeOpts::detect(Some(&other));
        let lf_opts = SerializeOpts::detect(Some(lf));
        assert_eq!(opts.trailing_newlines, lf_opts.trailing_newlines);
        assert_eq!(opts.blank_after_props, lf_opts.blank_after_props);
        assert_eq!(opts.indent, lf_opts.indent);
    }
    let mixed = "title:: x\r\n\r- a\r\n\t- b\r  cont\n- c\r\n\r";
    assert_eq!(parse(mixed), parse(lf));
}
