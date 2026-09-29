//! lsdoc is the outline authority (master cc9ab56ee, final behaviour at master
//! HEAD `promoted_heading_tests`): which lines open a block, and how blocks
//! nest, is lsdoc's (= mldoc's) decision, never a Tine line scanner's.
use super::*;

fn raws(blocks: &[DocBlock]) -> Vec<String> {
    blocks.iter().map(|block| block.raw.clone()).collect()
}

/// Every fixture must keep its meaning through the canonical serializer.
fn assert_structural_round_trip(source: &str) {
    let doc = parse(source);
    let canonical = serialize_with(&doc, &SerializeOpts::detect(Some(source)));
    assert_eq!(parse(&canonical), doc, "{source:?} -> {canonical:?}");
}

#[test]
fn a_leading_heading_owns_its_indented_children() {
    // OG writes a leading Markdown heading unbulleted with its children one
    // level in (og@6e7afa8 `file/core.cljs` `transform-content`,
    // `markdown-top-heading?`); lsdoc nests them under it.
    let source = "# Project\n\t- child one\n\t- child two\n- sibling\n\t- nested sibling child";
    let doc = parse(source);
    assert_eq!(doc.pre_block, None);
    assert_eq!(raws(&doc.roots), ["# Project", "sibling"]);
    assert_eq!(raws(&doc.roots[0].children), ["child one", "child two"]);
    assert_eq!(raws(&doc.roots[1].children), ["nested sibling child"]);
    assert_structural_round_trip(source);
}

#[test]
fn unbulleted_headings_between_bullets_are_their_own_blocks() {
    let source = "- editable root\n## first section\n### second section\n- trailing root";
    let doc = parse(source);
    assert_eq!(
        raws(&doc.roots),
        [
            "editable root",
            "## first section",
            "### second section",
            "trailing root"
        ]
    );
    assert_structural_round_trip(source);
}

#[test]
fn legacy_collapsed_heading_uses_parser_owned_same_level_topology() {
    // Superseded og `promote_preamble_collapsed_heading`: the bullets after an
    // unbulleted heading at the same column are its siblings in lsdoc.
    let source = "# Parent\ncollapsed:: true\n- child\n- sibling";
    let doc = parse(source);
    assert_eq!(doc.pre_block, None);
    assert_eq!(
        raws(&doc.roots),
        ["# Parent\ncollapsed:: true", "child", "sibling"]
    );
    assert!(doc.roots.iter().all(|root| root.children.is_empty()));
    assert_structural_round_trip(source);
}

#[test]
fn page_properties_stay_the_preamble_before_a_heading() {
    let source = "title:: Page\n\n# Project\ntext\n\t- child\n- sibling";
    let doc = parse(source);
    assert_eq!(doc.pre_block.as_deref(), Some("title:: Page"));
    assert_eq!(raws(&doc.roots), ["# Project\ntext", "sibling"]);
    assert_eq!(raws(&doc.roots[0].children), ["child"]);
    assert_eq!(
        serialize_with(&doc, &SerializeOpts::detect(Some(source))),
        "title:: Page\n\n- # Project\n  text\n\t- child\n- sibling",
        "the whole-page serializer keeps the preamble separator"
    );
    assert_structural_round_trip(source);
}

#[test]
fn a_heading_inside_a_block_body_opens_a_child_block() {
    // mldoc reads an indented `# ` line after a bullet as a heading at the
    // line's own column, so it is a child block, not continuation text.
    let source = "- x\n  # H\n- a";
    let doc = parse(source);
    assert_eq!(raws(&doc.roots), ["x", "a"]);
    assert_eq!(raws(&doc.roots[0].children), ["  # H"]);
    assert_structural_round_trip(source);
}

#[test]
fn a_hash_without_heading_shape_stays_block_text() {
    // Benign extreme of the heading fixtures: tags and `#+` directives in a
    // body are not headings.
    let source = "- x\n  #tag and more\n  #+caption: c\n- y";
    let doc = parse(source);
    assert_eq!(raws(&doc.roots), ["x\n#tag and more\n#+caption: c", "y"]);
}

#[test]
fn dash_bullet_forms_lsdoc_accepts_keep_their_text() {
    let source = "-\tx\n- \n-\n-  y";
    let doc = parse(source);
    assert_eq!(raws(&doc.roots), ["\tx", "", "", " y"]);
    assert_structural_round_trip(source);
}

#[test]
fn a_form_feed_indent_nests_as_lsdoc_measures_it() {
    // mldoc's `is_space` includes form feed, so the bullet is one level in.
    let source = "- a\n\u{c}- b";
    let doc = parse(source);
    assert_eq!(raws(&doc.roots), ["a"]);
    assert_eq!(raws(&doc.roots[0].children), ["b"]);
}

#[test]
fn an_unrepresentable_outline_is_kept_whole_as_page_text() {
    // lsdoc reports two headers on one physical line; Tine's one-block-per-
    // line model cannot hold that, so the page is shown (and saved) as text.
    let source = "- $$x$$ # #+BEGIN_NOTE\r\nx\r\n#+END_NOTE";
    let doc = parse(source);
    assert!(doc.roots.is_empty());
    assert_eq!(
        doc.pre_block.as_deref(),
        Some("- $$x$$ # #+BEGIN_NOTE\nx\n#+END_NOTE")
    );
    assert_eq!(
        serialize_with(&doc, &SerializeOpts::detect(Some(source))),
        "- $$x$$ # #+BEGIN_NOTE\nx\n#+END_NOTE"
    );
}

#[test]
fn a_deep_quote_staircase_degrades_instead_of_recursing() {
    // Hostile: lsdoc recurses per quote level while parsing (I-22).
    let hostile = format!("- {}x\n- y", "> ".repeat(20_000));
    let doc = parse(&hostile);
    assert!(doc.roots.is_empty());
    assert!(doc.pre_block.is_some_and(|pre| pre.ends_with("- y")));
    // Benign extreme: a staircase at the bound still parses as an outline.
    let benign = format!("- {}x\n- y", "> ".repeat(100));
    assert_eq!(parse(&benign).roots.len(), 2);
}

#[test]
fn a_deep_outline_parses_without_recursion() {
    let deep: String = (0..3_000).map(|i| format!("{}- x\n", "\t".repeat(i))).collect();
    let mut depth = 0;
    let mut level = &parse(&deep).roots;
    while let Some(first) = level.first() {
        depth += 1;
        level = &first.children;
    }
    assert_eq!(depth, 3_000);
    let flat: String = (0..100_000).map(|i| format!("- block {i}\n")).collect();
    let doc = parse(&flat);
    assert_eq!(doc.roots.len(), 100_000);
    assert_eq!(doc.roots[99_999].raw, "block 99999");
}
