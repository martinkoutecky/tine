use tine_outline::{
    block_regions,
    editor_text::{self, HidePolicy, BUILTIN_HIDDEN},
};

#[test]
fn every_hidden_builtin_and_configured_key_keeps_bytes_after_edit_and_clear() {
    let hide = HidePolicy::Configured {
        keys: vec!["secret".into(), "tine.custom".into()],
        sheet_cell: false,
    };
    for key in BUILTIN_HIDDEN
        .iter()
        .copied()
        .chain(["secret", "tine.custom"])
    {
        for org in [false, true] {
            let value = if key == "id" {
                "6512bd43-d9ca-4c95-b0b4-1c2d3e4f5a6b"
            } else {
                "true"
            };
            let line = if org {
                format!(":{key}: {value}  \t")
            } else {
                format!("{key}:: {value}  \t")
            };
            let raw = if org {
                format!("Body\n:PROPERTIES:\n{line}\n:END:")
            } else {
                format!("Body\n{line}")
            };
            let parts = editor_text::split(&raw, org, &hide);
            assert_eq!(parts.visible, "Body", "{raw:?}");
            assert_eq!(parts.hidden, line);
            for text in ["Revised", ""] {
                let joined =
                    editor_text::join_preserving(text, &parts.hidden, org, &raw, &hide).unwrap();
                assert_eq!(editor_text::split(&joined, org, &hide).hidden, line);
                assert_eq!(editor_text::split(&joined, org, &hide).visible, text);
                assert!(block_regions::parse(&joined, org)
                    .properties
                    .iter()
                    .any(|p| p.primary && p.key == key));
            }
        }
    }
}

#[test]
fn parser_ownership_not_ogs_first_match_strip_controls_hiding() {
    // Intentional differences from OG util/property.cljs remove-built-in-properties:
    // all accepted duplicate rows hide, and fenced lookalikes remain visible.
    // Saving keeps original spelling/spacing instead of OG's reserialization.
    let raw = "Body\nheading:: true  \nheading:: false\n```\nheading:: literal\n```";
    let parts = editor_text::split(raw, false, &HidePolicy::Builtin);
    assert_eq!(parts.hidden, "heading:: true  \nheading:: false");
    assert_eq!(parts.visible, "Body\n```\nheading:: literal\n```");
}

#[test]
fn org_hidden_crlf_bytes_survive_lf_textarea_edits() {
    for key in ["heading", "card-last-score", "secret"] {
        let hide = HidePolicy::Configured {
            keys: vec!["secret".into()],
            sheet_cell: false,
        };
        let row = format!(":{key}: true  \t\r");
        let raw = format!("Body\r\n:PROPERTIES:\r\n{row}\n:owner: Ada\r\n:END:");
        let parts = editor_text::split(&raw, true, &hide);
        for visible in [
            "Revised",
            "",
            "Revised\n:PROPERTIES:\n:owner: Ada\n:END:",
            "Revised\r\n:PROPERTIES:\r\n:owner: Ada\r\n:END:",
        ] {
            let joined = editor_text::join(visible, &parts.hidden, true).unwrap();
            assert!(joined.contains(&format!("{row}\n")), "{key}/{visible:?}");
            assert_eq!(
                editor_text::split(&joined, true, &hide).hidden,
                row,
                "{key}/{visible:?}: {joined:?}"
            );
        }
    }
}

#[test]
fn configured_projection_keeps_metadata_on_enter_and_merge() {
    use tine_outline::outline::{self, Format, KeySource, OutlinePage, Verb};
    let hide = HidePolicy::Configured {
        keys: vec!["secret".into()],
        sheet_cell: false,
    };
    for format in [Format::Md, Format::Org] {
        let org = format.is_org();
        let props = if org {
            ":PROPERTIES:\n:heading: true\n:secret: opaque  \n:END:"
        } else {
            "heading:: true\nsecret:: opaque  "
        };
        let raw = format!("Body\n{props}");
        let page: OutlinePage = serde_json::from_value(serde_json::json!({
            "name": "P", "format": format, "roots": ["a", "b"],
            "blocks": { "a": { "raw": "First" }, "b": { "raw": raw } }
        }))
        .unwrap();
        let merged = outline::apply_with_policy(
            &[page.clone()],
            &Verb::Merge {
                survivor: "a".into(),
                absorbed: "b".into(),
                absorbed_referenced: false,
            },
            &KeySource::default(),
            &hide,
        )
        .unwrap();
        let survivor = merged.pages[0]
            .upserts
            .iter()
            .find(|u| u.key == "a")
            .unwrap();
        assert_eq!(
            editor_text::split(&survivor.raw, org, &hide).visible,
            "FirstBody"
        );
        assert_eq!(
            editor_text::split(&survivor.raw, org, &hide).hidden,
            editor_text::split(&raw, org, &hide).hidden
        );
        let split = outline::apply_with_policy(
            &[page],
            &Verb::Split {
                key: "b".into(),
                raw_offset: 4,
                force_child: false,
                children_visible: true,
                keep_start_in_scope: false,
            },
            &KeySource::default(),
            &hide,
        )
        .unwrap();
        let original = split.pages[0]
            .upserts
            .iter()
            .find(|u| u.key == "b")
            .unwrap();
        assert_eq!(
            editor_text::split(&original.raw, org, &hide).hidden,
            editor_text::split(&raw, org, &hide).hidden
        );
    }
}

/// Review B1 (2026-10-24): a newly hidden key in an OG-layout group (title, properties,
/// body) stays in that one group on a text edit. Below the body it started a second
/// group, which OG ignores (REG-HIDDEN-PROPS-STAY-IN-THE-GROUP).
#[test]
fn a_hidden_row_stays_in_the_block_s_own_property_group() {
    let hide = HidePolicy::Configured {
        keys: vec!["secret".into()],
        sheet_cell: false,
    };
    for (raw, expected) in [
        (
            "Title\nbackground-color:: red\nowner:: Ada\nbody line",
            "Title X\nowner:: Ada\nbackground-color:: red\nbody line",
        ),
        (
            "Title #card\ncard-repeats:: 2\nowner:: me\nAnswer",
            "Title X #card\nowner:: me\ncard-repeats:: 2\nAnswer",
        ),
        (
            "Title\nsecret:: s\nowner:: Ada\nbody",
            "Title X\nowner:: Ada\nsecret:: s\nbody",
        ),
        // No visible group: the hidden rows remain the block's only group.
        (
            "Title\nheading:: true\nbody line",
            "Title X\nbody line\nheading:: true",
        ),
    ] {
        let parts = editor_text::split(raw, false, &hide);
        let edited = parts.visible.replacen("Title", "Title X", 1);
        let out = editor_text::set_text_with_policy(raw, false, &edited, &hide).unwrap();
        assert_eq!(out, expected, "{raw:?}");
        assert_eq!(
            block_regions::parse(&out, false).property_regions.len(),
            1,
            "{out:?}"
        );
    }
}

/// Review round 2 (2026-10-24): mldoc reads `key:: value` rows directly above a Markdown
/// `:PROPERTIES:` drawer as one group with it. A hidden row stays above the drawer, never
/// below the body, where OG would drop it.
#[test]
fn a_hidden_row_above_a_markdown_properties_drawer_stays_above_it() {
    for raw in [
        "Title\nheading:: true\n:PROPERTIES:\n:owner: Ada\n:END:\nbody",
        "Title\r\nheading:: true\r\n:PROPERTIES:\r\n:owner: Ada\r\n:END:\r\nbody",
    ] {
        let parts = editor_text::split(raw, false, &HidePolicy::Builtin);
        assert!(!parts.hidden.is_empty(), "{raw:?}");
        let edited = parts.visible.replacen("Title", "Title X", 1);
        let out =
            editor_text::set_text_with_policy(raw, false, &edited, &HidePolicy::Builtin).unwrap();
        assert_eq!(out, raw.replacen("Title", "Title X", 1), "{raw:?}");
    }
}

/// Review R1 (2026-10-24): the text-only splice re-splits the source with the caller's
/// policy, so a configured key in a LOGBOOK block keeps its exact place.
#[test]
fn the_logbook_splice_uses_the_caller_s_policy() {
    let hide = HidePolicy::Configured {
        keys: vec!["secret".into()],
        sheet_cell: false,
    };
    let raw = "TODO Title\n:LOGBOOK:\nCLOCK: [2026-01-01 Thu 10:00:00]--[2026-01-01 Thu 11:00:00] =>  01:00:00\n:END:\nsecret:: s";
    let parts = editor_text::split(raw, false, &hide);
    let edited = parts.visible.replacen("Title", "Title X", 1);
    let out = editor_text::set_text_with_policy(raw, false, &edited, &hide).unwrap();
    assert_eq!(out, raw.replacen("Title", "Title X", 1));
}
