use tine_core::model::{page_title_from_preamble, preamble_read, Format, PreambleRead};

#[test]
fn org_directive_discovery_does_not_reparse_growing_prefixes() {
    let mut prefix = String::from("#+TITLE: Named\n");
    for _ in 0..1000 {
        prefix.push_str("#+KEY: value\n");
        assert_eq!(preamble_read(&prefix, Format::Org), PreambleRead::More);
    }
    prefix.push_str("* first\n");
    assert_eq!(
        preamble_read(&prefix, Format::Org),
        PreambleRead::Settled(Some("Named".into()))
    );
    assert_eq!(
        page_title_from_preamble(&prefix, Format::Org),
        Some("Named".into())
    );
    // An existing bounded per-line filter must run before any full-prefix parse.
    let source = include_str!("../src/model.rs");
    let body = source
        .split("pub fn preamble_read(")
        .nth(1)
        .unwrap()
        .split("fn org_meta_line")
        .next()
        .unwrap();
    assert!(body.find("if inert").unwrap() < body.find("preamble_end(prefix").unwrap(),
        "I-15/I-22: inert Org directives take O(last line), never parse the growing prefix; exemplar model::preamble_read");
}
