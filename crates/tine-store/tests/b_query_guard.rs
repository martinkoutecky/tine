#[test]
fn query_policies_have_one_answerer() {
    let atom = include_str!("../../tine-core/src/query/atom.rs");
    let eval = include_str!("../src/query/eval.rs");
    let print = include_str!("../../tine-core/src/query/print.rs");
    let og = include_str!("../../tine-core/src/query/og.rs");
    let tql = include_str!("../../tine-core/src/query/tql.rs");
    assert!(atom.contains("AtomDeduper") && eval.contains("AtomDeduper"), "I-12/I-22: both atomization and cross-row flattening must use atom::AtomDeduper (exemplar query/atom.rs), never scan previous atoms");
    assert!(
        !eval.contains("existing.key == atom.key") && !atom.contains("seen: &mut Vec<String>"),
        "I-22: atom uniqueness must cost O(A log A) or better; imitate query/atom.rs"
    );
    assert!(
        !print.contains("fn escape_like_literal") && !og.contains("fn escape_like("),
        "I-12: LIKE literal encoding belongs to query/text.rs::escape_like_literal"
    );
    assert!(
        !print.contains("number.fract()") && print.contains("format_number"),
        "I-12: query/atom.rs::format_number owns numeric spelling"
    );
    assert!(
        !tql.contains("fn starts_with_prefix") && tql.contains("LikePattern"),
        "I-12: LIKE prefix recognition must reuse query/text.rs::LikePattern"
    );
}
