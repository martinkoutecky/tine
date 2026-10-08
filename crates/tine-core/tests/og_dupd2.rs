use tine_core::lsdoc::ast::Inline;
use tine_core::{render::parse_block, standalone_macro::sole_macro};
#[test]
fn sole_visible_macro_uses_accepted_nodes_not_lines_or_html() {
    for org in [false, true] {
        for raw in [
            "{{embed [[A]]}} {{embed [[B]]}}",
            "`{{embed [[A]]}}`",
            "TODO {{embed [[A]]}}",
            "{{query (task TODO)}}\nfoo::bar",
        ] {
            assert!(sole_macro(&parse_block(raw, org)).is_none(), "{org}: {raw}");
        }
        let raw = if org {
            "{{embed [[A]]}}\n:PROPERTIES:\n:klíč: hodnota\n:END:"
        } else {
            "{{embed [[A]]}}\nklíč:: hodnota"
        };
        assert!(
            matches!(sole_macro(&parse_block(raw, org)), Some(Inline::Macro { name, .. }) if name == "embed")
        );
    }
}
