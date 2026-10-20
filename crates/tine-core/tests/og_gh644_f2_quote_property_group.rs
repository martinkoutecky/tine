//! REG-OG-GH644-F2-QUOTE-PROPERTY-GROUP: a quote whose first line is a
//! `#+NAME: VALUE` line followed by a `key::` line (`>#+BEGIN_:\no::`, found
//! by fuzz) made lsdoc reparse the quote body's own span forever. That is a
//! stack overflow, which aborts the process: Tine's per-file isolation cannot
//! catch it, so one such file killed Tine at every graph load. Fixed in lsdoc
//! v0.5.10 (mldoc has no property parser in block content; the line is text).
//! Invariant: I-2 (one bad file must not refuse the graph).

use tine_core::doc::parse;

const SHAPES: &[&str] = &[
    ">#+BEGIN_:\no::",
    "> #+BEGIN_:\n> o::",
    "> #+BEGIN_: v\n> a:: b",
    "#+BEGIN_QUOTE\n#+BEGIN_:\no::\n#+END_QUOTE",
    "- > #+BEGIN_: x\n  > k:: v",
];

#[test]
fn a_quoted_property_group_parses_as_a_page_and_as_a_block() {
    for shape in SHAPES {
        let doc = parse(shape);
        for block in &doc.roots {
            let _ = block.projection();
        }
        let bulleted = format!("- {}", shape.replace('\n', "\n  "));
        let doc = parse(&bulleted);
        assert!(!doc.roots.is_empty(), "{bulleted:?}");
        for block in &doc.roots {
            let _ = block.projection();
        }
    }
}
