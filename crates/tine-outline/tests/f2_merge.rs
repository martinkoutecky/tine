use tine_outline::{
    block_regions,
    editor_text::HidePolicy,
    outline::{self, Format, KeySource, Notice, OutlinePage, Verb},
};

const ID: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const OTHER: &str = "aaaaaaaa-0000-4000-8000-000000000002";

fn raw(format: Format, title: &str, rows: &[(&str, &str)]) -> String {
    if rows.is_empty() {
        return title.into();
    }
    if format.is_org() {
        format!(
            "{title}\n:PROPERTIES:\n{}\n:END:",
            rows.iter()
                .map(|(k, v)| format!(":{k}: {v}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    } else {
        format!(
            "{title}\n{}",
            rows.iter()
                .map(|(k, v)| format!("{k}:: {v}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

fn merge(
    format: Format,
    keep: &str,
    gone: &str,
    referenced: bool,
) -> Result<outline::Change, outline::Reason> {
    let page: OutlinePage = serde_json::from_value(serde_json::json!({
        "name": "P", "format": format, "roots": ["a", "b"],
        "blocks": { "a": { "raw": keep }, "b": { "raw": gone, "children": ["c"] }, "c": { "raw": "child" } }
    })).unwrap();
    outline::apply_with_policy(
        &[page],
        &Verb::Merge {
            survivor: "a".into(),
            absorbed: "b".into(),
            absorbed_referenced: referenced,
        },
        &KeySource::default(),
        &HidePolicy::Configured {
            keys: vec!["secret".into()],
            sheet_cell: false,
        },
    )
}

fn survivor(change: &outline::Change) -> &str {
    &change.pages[0]
        .upserts
        .iter()
        .find(|u| u.key == "a")
        .unwrap()
        .raw
}

#[test]
fn f2_merge_drops_absorbed_heading_card_and_configured_hidden_rows() {
    for format in [Format::Md, Format::Org] {
        for keep_rows in [vec![], vec![("heading", "2")]] {
            let keep = raw(format, "one", &keep_rows);
            let gone = raw(
                format,
                "two",
                &[
                    ("heading", "true"),
                    ("card-repeats", "5"),
                    ("card-next-schedule", "2026-10-30"),
                    ("secret", "opaque"),
                ],
            );
            let change = merge(format, &keep, &gone, false).unwrap();
            assert_eq!(survivor(&change), raw(format, "onetwo", &keep_rows));
            assert_eq!(
                change.pages[0]
                    .upserts
                    .iter()
                    .find(|u| u.key == "a")
                    .unwrap()
                    .children,
                ["c"]
            );
            assert_eq!(change.pages[0].deleted, ["b"]);
        }
    }
}

#[test]
fn f2_merge_transfers_referenced_identity_and_drops_unreferenced_identity() {
    for format in [Format::Md, Format::Org] {
        for referenced in [false, true] {
            let gone = raw(format, "two", &[("id", ID)]);
            let change = merge(format, "one", &gone, referenced).unwrap();
            let id = block_regions::parse(survivor(&change), format.is_org())
                .id
                .map(|p| p.value);
            assert_eq!(id.as_deref(), referenced.then_some(ID));
            if referenced {
                // Graph referrers need no rewrite: both ordinary refs and
                // embeds retain a target in the survivor's authored identity.
                for source in [format!("(({ID}))"), format!("{{{{embed (({ID}))}}}}")] {
                    let refs =
                        tine_outline::lsdoc_block_parse::parse_projection(&source, format.is_org())
                            .refs
                            .block;
                    assert_eq!(refs, [ID]);
                    for target in refs {
                        assert_eq!(id.as_deref(), Some(target.as_str()));
                    }
                }
            }
            assert_eq!(
                change.notices,
                if referenced {
                    vec![Notice::IdTransferred {
                        from: "b".into(),
                        to: "a".into(),
                    }]
                } else {
                    vec![]
                }
            );
        }
    }
}

#[test]
fn f2_merge_keeps_both_identities_resolvable_when_ids_collide() {
    // Intentional difference from OG's update-block-refs: a page-local edit
    // cannot rewrite graph-wide referrers, so refuse without any patch.
    for format in [Format::Md, Format::Org] {
        let keep = raw(format, "one", &[("id", OTHER)]);
        let gone = raw(format, "two", &[("id", ID)]);
        let result = merge(format, &keep, &gone, true);
        assert!(
            result.is_err(),
            "both referenced identities must remain available"
        );
        assert_eq!(
            serde_json::to_value(result.unwrap_err()).unwrap(),
            "referenced_identity"
        );
        let change = merge(format, &keep, &gone, false).unwrap();
        assert_eq!(
            block_regions::parse(survivor(&change), format.is_org())
                .id
                .unwrap()
                .value,
            OTHER
        );
    }
}

#[test]
fn f2_merge_keeps_only_survivor_clocks_like_og_backspace() {
    for format in [Format::Md, Format::Org] {
        for keep in [
            "one",
            "one\n:LOGBOOK:\nCLOCK: [2026-10-20 Tue 08:00:00]\n:END:",
        ] {
            let gone = "two\n:LOGBOOK:\nCLOCK: [2026-10-21 Wed 09:00:00]\n:END:";
            let change = merge(format, keep, gone, false).unwrap();
            assert_eq!(survivor(&change), keep.replacen("one", "onetwo", 1));
        }
    }
}
