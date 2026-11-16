use tine_core::{block_regions, doc::DocBlock};
use tine_outline::editor_text::{self, HidePolicy};

#[test]
fn ordinary_author_rows_are_hidden_and_retained_byte_exact() {
    for nl in ["\n", "\r\n"] {
        for org in [false, true] {
            let meta = if org {
                [":PROPERTIES:", ":tine.author: Ada Test  \t", ":END:"].join(nl)
            } else {
                "tine.author:: Ada Test  \t".into()
            };
            let raw = format!("Words{nl}{meta}");
            let parts = editor_text::split(&raw, org, &HidePolicy::Builtin);
            assert_eq!(
                parts.visible,
                if nl == "\r\n" { "Words\r" } else { "Words" }
            );
            let saved =
                editor_text::set_text_with_policy(&raw, org, "Revised words", &HidePolicy::Builtin)
                    .unwrap();
            assert!(
                saved.contains(if org {
                    ":tine.author: Ada Test  \t"
                } else {
                    "tine.author:: Ada Test  \t"
                }),
                "{saved:?}"
            );
            assert_eq!(
                saved.split('\n').find(|line| line.contains("tine.author")),
                raw.split('\n').find(|line| line.contains("tine.author"))
            );
            assert_eq!(
                editor_text::split(&saved, org, &HidePolicy::Builtin)
                    .visible
                    .trim_end_matches('\r'),
                "Revised words"
            );
        }
    }
}

#[test]
fn quote_authors_share_region_editor_and_native_facet_ownership() {
    for body in [
        "> some quoted text",
        "Title\n> first\n> second",
        "> [!TIP]\n> helpful",
    ] {
        for author in ["Ada", "Ada Test  \t", "**Ada**", "[[Ada]]", "艾达"] {
            let row = format!("tine.author:: {author}");
            let raw = format!("{body}\n{row}");
            let regions = block_regions::parse(&raw, false);
            assert_eq!(
                regions.property("tine.author").unwrap().value,
                author.trim()
            );
            assert_eq!(
                editor_text::split(&raw, false, &HidePolicy::Builtin).visible,
                body
            );
            let block = DocBlock::new(raw.clone());
            assert!(block
                .properties()
                .contains(&("tine.author".into(), author.trim().into())));
            let saved = editor_text::set_text_with_policy(
                &raw,
                false,
                &format!("{body} revised"),
                &HidePolicy::Builtin,
            )
            .unwrap();
            assert!(saved.contains(&row), "{saved:?}");
            assert!(block_regions::parse(&saved, false)
                .property("tine.author")
                .is_some());
            let removed = regions
                .apply(
                    &raw,
                    false,
                    block_regions::Edit::Property {
                        key: "tine.author".into(),
                        value: None,
                    },
                )
                .unwrap();
            assert_eq!(removed, body);
        }
    }
}

#[test]
fn quoted_fenced_and_user_authors_are_ordinary_content() {
    for raw in [
        "> tine.author:: literal",
        "> text\n> tine.author:: literal",
        "```\ntine.author:: literal\n```",
        "Words\nauthor:: book author",
    ] {
        assert_eq!(
            editor_text::split(raw, false, &HidePolicy::Builtin).visible,
            raw
        );
    }
}
