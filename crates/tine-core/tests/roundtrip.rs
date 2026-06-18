//! Round-trip fidelity tests: `serialize(parse(x)) == x` for well-formed
//! Logseq markdown, plus structural and canonicalization checks.

use tine_core::doc::{self, DocBlock};
use pretty_assertions::assert_eq;

/// Assert byte-exact round-trip.
fn assert_roundtrip(input: &str) {
    let parsed = doc::parse(input);
    let out = doc::serialize(&parsed);
    assert_eq!(out, input, "round-trip mismatch");
    // Idempotence: parsing our own output yields the same string again.
    let out2 = doc::serialize(&doc::parse(&out));
    assert_eq!(out2, out, "not idempotent");
}

#[test]
fn simple_flat_blocks() {
    assert_roundtrip("- first\n- second\n- third\n");
}

#[test]
fn nested_blocks_tabs() {
    let input = "- parent\n\t- child a\n\t- child b\n\t\t- grandchild\n- sibling\n";
    assert_roundtrip(input);
    let doc = doc::parse(input);
    assert_eq!(doc.roots.len(), 2);
    assert_eq!(doc.roots[0].children.len(), 2);
    assert_eq!(doc.roots[0].children[1].children.len(), 1);
    assert_eq!(doc.roots[0].children[1].children[0].raw, "grandchild");
}

#[test]
fn multiline_block_continuation() {
    // Top-level continuation = 2 spaces; nested = 1 tab + 2 spaces.
    let input = "- first line\n  second line\n  third line\n\t- child\n\t  child cont\n";
    assert_roundtrip(input);
    let doc = doc::parse(input);
    assert_eq!(doc.roots[0].raw, "first line\nsecond line\nthird line");
    assert_eq!(doc.roots[0].children[0].raw, "child\nchild cont");
}

#[test]
fn fenced_code_with_bullet_line_stays_one_block() {
    // A `- ` line inside a ``` fence is literal code, NOT a child block; it must
    // round-trip and not split the block (the DS6 corruption case).
    let input = "- ```clojure\n  (defn f [x] x)\n  - not a child\n  ```\n- after\n";
    assert_roundtrip(input);
    let doc = doc::parse(input);
    assert_eq!(doc.roots.len(), 2);
    assert_eq!(doc.roots[0].children.len(), 0, "code fence must not become children");
    assert_eq!(doc.roots[0].raw, "```clojure\n(defn f [x] x)\n- not a child\n```");
    assert_eq!(doc.roots[1].raw, "after");
}

#[test]
fn nested_backtick_fence_not_closed_early() {
    // A ```` (4-backtick) fence whose body contains ``` must NOT close at the
    // inner ```; the `- ` line stays literal and the block survives round-trip.
    let input = "- ````\n  ```\n  - still code\n  ````\n- after\n";
    assert_roundtrip(input);
    let doc = doc::parse(input);
    assert_eq!(doc.roots.len(), 2);
    assert_eq!(doc.roots[0].children.len(), 0, "inner ``` must not split the block");
    assert_eq!(doc.roots[0].raw, "````\n```\n- still code\n````");
}

#[test]
fn tilde_fence_protects_bullet_lines() {
    let input = "- ~~~\n  - not a child\n  ~~~\n- after\n";
    assert_roundtrip(input);
    let doc = doc::parse(input);
    assert_eq!(doc.roots.len(), 2);
    assert_eq!(doc.roots[0].children.len(), 0);
    assert_eq!(doc.roots[0].raw, "~~~\n- not a child\n~~~");
}

#[test]
fn fenced_code_on_child_block() {
    let input = "- parent\n\t- ```\n\t  - inner\n\t  ```\n\t- real sibling\n";
    assert_roundtrip(input);
    let doc = doc::parse(input);
    assert_eq!(doc.roots[0].children.len(), 2);
    assert_eq!(doc.roots[0].children[0].raw, "```\n- inner\n```");
    assert_eq!(doc.roots[0].children[0].children.len(), 0);
    assert_eq!(doc.roots[0].children[1].raw, "real sibling");
}

#[test]
fn block_properties() {
    let input = "- a task\n  id:: 628953c1-8d75-49fe-a648-f4c612109098\n  collapsed:: true\n";
    assert_roundtrip(input);
    let doc = doc::parse(input);
    let b = &doc.roots[0];
    assert!(b.collapsed());
    assert_eq!(
        b.property("id").as_deref(),
        Some("628953c1-8d75-49fe-a648-f4c612109098")
    );
    let props = b.properties();
    assert_eq!(props.len(), 2);
}

#[test]
fn page_properties_pre_block() {
    let input = "title:: My Page\ntags:: a, b\nalias:: Other\n\n- first block\n- second\n";
    assert_roundtrip(input);
    let doc = doc::parse(input);
    assert_eq!(doc.pre_block.as_deref(), Some("title:: My Page\ntags:: a, b\nalias:: Other"));
    assert_eq!(doc.roots.len(), 2);
}

#[test]
fn markers_detected() {
    let doc = doc::parse("- TODO buy milk\n- DONE laundry\n- just text\n");
    assert_eq!(doc.roots[0].marker(), Some("TODO"));
    assert_eq!(doc.roots[1].marker(), Some("DONE"));
    assert_eq!(doc.roots[2].marker(), None);
}

#[test]
fn headings() {
    let doc = doc::parse("- ## A heading\n- not # a heading\n");
    assert_eq!(doc.roots[0].heading_level(), Some(2));
    assert_eq!(doc.roots[1].heading_level(), None);
}

#[test]
fn empty_block() {
    assert_roundtrip("- before\n-\n- after\n");
    let doc = doc::parse("- before\n-\n- after\n");
    assert_eq!(doc.roots[1].raw, "");
}

#[test]
fn deep_nesting() {
    let input = "- l1\n\t- l2\n\t\t- l3\n\t\t\t- l4\n\t\t\t\t- l5\n";
    assert_roundtrip(input);
}

#[test]
fn inline_syntax_preserved_in_raw() {
    // Inline markup is rendered later; here we just ensure it survives I/O.
    let input = "- **bold** and *italic* and `code`\n- a [[Page Ref]] and #tag and ((uuid-here))\n- $$E=mc^2$$ math\n";
    assert_roundtrip(input);
}

#[test]
fn canonicalize_space_indent_to_tabs() {
    // Space-indented input (e.g. the shui-graph's contents.md uses 4 spaces) is
    // accepted with nesting preserved, and normalized to TABs on output —
    // Logseq-compatible reformatting.
    let input = "- parent\n  - child\n    - grandchild\n";
    let parsed = doc::parse(input);
    assert_eq!(parsed.roots.len(), 1);
    assert_eq!(parsed.roots[0].children.len(), 1);
    assert_eq!(parsed.roots[0].children[0].children.len(), 1);
    let out = doc::serialize(&parsed);
    assert_eq!(out, "- parent\n\t- child\n\t\t- grandchild\n");
    // And the canonical form is stable.
    assert_eq!(doc::serialize(&doc::parse(&out)), out);
}

#[test]
fn properties_only_file_no_blocks() {
    // A page that is only properties (no bullets) must not gain a blank line.
    let input = "table-example:: true\n";
    assert_roundtrip(input);
    let doc = doc::parse(input);
    assert!(doc.roots.is_empty());
    assert_eq!(doc.pre_block.as_deref(), Some("table-example:: true"));
}

#[test]
fn manual_tree_serialization() {
    let mut parent = DocBlock::new("parent");
    parent.children.push(DocBlock::new("child one"));
    let mut child_two = DocBlock::new("child two");
    child_two.children.push(DocBlock::new("deep"));
    parent.children.push(child_two);
    let doc = doc::Document { pre_block: None, roots: vec![parent] };
    let out = doc::serialize(&doc);
    assert_eq!(out, "- parent\n\t- child one\n\t- child two\n\t\t- deep\n");
}
