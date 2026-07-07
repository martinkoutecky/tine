//! Round-trip fidelity tests: `serialize(parse(x)) == x` for well-formed
//! Logseq markdown, plus structural and canonicalization checks.

use pretty_assertions::assert_eq;
use tine_core::{
    doc::{self, DocBlock},
    org,
};

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
    assert_eq!(
        doc.roots[0].children.len(),
        0,
        "code fence must not become children"
    );
    assert_eq!(
        doc.roots[0].raw,
        "```clojure\n(defn f [x] x)\n- not a child\n```"
    );
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
    assert_eq!(
        doc.roots[0].children.len(),
        0,
        "inner ``` must not split the block"
    );
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
    assert_eq!(
        doc.pre_block.as_deref(),
        Some("title:: My Page\ntags:: a, b\nalias:: Other")
    );
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
fn sheets_positional_grid_encoding_round_trips() {
    let input = concat!(
        "- Sheet fixture\n",
        "  tine.view:: grid\n",
        "  tine.header:: true\n",
        "  tine.col-widths:: 0=120;2=88\n",
        "  tine.col-aggregates:: 0=count;2=unique\n",
        "\t-\n",
        "\t\t- Column A\n",
        "\t\t- Column B\n",
        "\t\t- Column C\n",
        "\t-\n",
        "\t\t- TODO Cell with [[Page Ref]]\n",
        "\t\t- Nested grid\n",
        "\t\t  tine.view:: grid\n",
        "\t\t\t-\n",
        "\t\t\t\t- Inner A1\n",
        "\t\t\t\t- Inner A2\n",
        "\t\t\t-\n",
        "\t\t\t\t- Inner B1\n",
        "\t\t- Plain tail\n",
        "\t-\n",
    );

    assert_roundtrip(input);
}

#[test]
fn sheets_org_positional_grid_encoding_round_trips() {
    let input = concat!(
        "* Sheet fixture\n",
        ":PROPERTIES:\n",
        ":tine.view: grid\n",
        ":tine.header: true\n",
        ":tine.col-widths: 0=120;2=88\n",
        ":tine.col-aggregates: 0=count;2=unique\n",
        ":END:\n",
        "**\n",
        "*** Column A\n",
        "*** Column B\n",
        "*** Column C\n",
        "**\n",
        "*** TODO Cell with [[Page Ref]]\n",
        "*** Nested grid\n",
        ":PROPERTIES:\n",
        ":tine.view: grid\n",
        ":END:\n",
        "****\n",
        "***** Inner A1\n",
        "***** Inner A2\n",
        "****\n",
        "***** Inner B1\n",
        "*** Plain tail\n",
        "**\n",
    );

    assert!(org::org_round_trips(input));
    let out = org::serialize_org(&org::parse_org(input));
    assert_eq!(out, input, "org sheet fixture round-trip mismatch");
}

#[test]
fn sheets_field_table_and_board_query_round_trip() {
    let input = concat!(
        "- Sheet phase 3 fixture\n",
        "\t- Children field table\n",
        "\t  tine.view:: table\n",
        "\t  tine.fields:: state=state;owner=text;status=enum:todo,doing,done;page=page\n",
        "\t  tine.formula.total:: price * qty\n",
        "\t  tine.col-aggregates:: prop:owner=unique;state=checked\n",
        "\t\t- TODO [#A] Draft field layer #sheets\n",
        "\t\t  owner:: Martin\n",
        "\t\t- DOING Implement renderer\n",
        "\t\t  owner:: Codex\n",
        "\t- {{query (todo TODO DOING DONE)}}\n",
        "\t  tine.view:: board\n",
        "\t  tine.group-by:: state\n",
    );

    assert_roundtrip(input);
}

#[test]
fn sheets_org_field_table_and_board_query_round_trip() {
    let input = concat!(
        "* Sheet phase 3 fixture\n",
        "** Children field table\n",
        ":PROPERTIES:\n",
        ":tine.view: table\n",
        ":tine.fields: state=state;owner=text;status=enum:todo,doing,done;page=page\n",
        ":tine.formula.total: price * qty\n",
        ":tine.col-aggregates: prop:owner=unique;state=checked\n",
        ":END:\n",
        "*** TODO [#A] Draft field layer :sheets:\n",
        ":PROPERTIES:\n",
        ":owner: Martin\n",
        ":END:\n",
        "*** DOING Implement renderer\n",
        ":PROPERTIES:\n",
        ":owner: Codex\n",
        ":END:\n",
        "** {{query (todo TODO DOING DONE)}}\n",
        ":PROPERTIES:\n",
        ":tine.view: board\n",
        ":tine.group-by: state\n",
        ":END:\n",
    );

    assert!(org::org_round_trips(input));
    let out = org::serialize_org(&org::parse_org(input));
    assert_eq!(out, input, "org sheet phase 3 fixture round-trip mismatch");
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
    let doc = doc::Document {
        pre_block: None,
        roots: vec![parent],
    };
    let out = doc::serialize(&doc);
    assert_eq!(out, "- parent\n\t- child one\n\t- child two\n\t\t- deep\n");
}
