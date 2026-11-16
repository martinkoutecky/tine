use tine_outline::{
    block_regions,
    editor_text::{self, HidePolicy, BUILTIN_HIDDEN},
};

#[test]
fn source_edits_keep_all_properties_in_the_group_og_reads() {
    let hide = HidePolicy::Configured {
        keys: vec!["secret".into()],
        sheet_cell: false,
    };
    for row in ["id:: X", "collapsed:: true", "secret:: opaque"] {
        let mut raw = format!("Title\n{row}\nbody");
        // Follow the editor's per-keystroke source updates, including the
        // transition from ordinary body text to a parsed property row.
        for ch in "\nowner:: Ada".chars() {
            let text = format!("{}{ch}", editor_text::split(&raw, false, &hide).visible);
            raw = editor_text::set_text_with_policy(&raw, false, &text, &hide).unwrap();
        }
        assert_og_group(&raw, &[row.split("::").next().unwrap(), "owner"]);
    }
}

#[test]
fn source_paste_at_property_end_keeps_the_hidden_id_in_ogs_group() {
    // A paste beginning at the end of the last visible property line used to
    // insert a paragraph between the visible and hidden property rows.
    let raw = "Title\nowner:: Ada\nid:: X\nbody";
    let joined =
        editor_text::set_text(raw, false, "Title\nowner:: Ada Lovelace\nnote\nbody").unwrap();
    assert_og_group(&joined, &["owner", "id"]);
    assert!(joined.contains("owner:: Ada Lovelace"));
    assert!(joined.contains("\nnote\nbody"));
}

#[test]
fn appending_after_an_existing_visible_group_keeps_every_property_readable() {
    let raw = "Title\nid:: X\nstatus:: active\nbody";
    let joined =
        editor_text::set_text(raw, false, "Title\nstatus:: active\nbody\nowner:: Ada").unwrap();
    assert_og_group(&joined, &["id", "status", "owner"]);
    assert!(joined.contains("\nbody"));
    let raw_submission = format!("{raw}\nowner:: Ada");
    let joined = editor_text::set_text(raw, false, &raw_submission).unwrap();
    assert_og_group(&joined, &["id", "status", "owner"]);
}

#[test]
fn source_edit_repairs_a_previously_split_property_group() {
    let raw = "Title\nowner:: Ada\nbody\nid:: X";
    let joined = editor_text::set_text(raw, false, "Title X\nowner:: Ada\nbody").unwrap();
    assert_og_group(&joined, &["owner", "id"]);
    assert_eq!(joined, "Title X\nowner:: Ada\nid:: X\nbody");
}

fn assert_og_group(raw: &str, keys: &[&str]) {
    let regions = block_regions::parse(raw, false);
    assert_eq!(regions.property_regions.len(), 1, "{raw:?}");
    for key in keys {
        assert!(
            regions
                .properties
                .iter()
                .any(|p| p.region == 0 && p.key == *key),
            "missing {key} in OG's first group: {raw:?}"
        );
    }
}

#[test]
fn org_normalized_edits_keep_cr_without_a_phantom_final_line() {
    let mut failures = Vec::new();
    for (raw, text, from, to) in [
        (
            "Title\r\n:PROPERTIES:\r\n:id: X\r\n:END:\r",
            "Title X",
            "Title",
            "Title X",
        ),
        (
            "Title\n:PROPERTIES:\n:id: X\n:END:\nbody\r",
            "Title\nbody2\n",
            "body",
            "body2",
        ),
    ] {
        let joined = editor_text::set_text(raw, true, text).unwrap();
        let expected = raw.replacen(from, to, 1);
        if joined != expected {
            failures.push(format!("expected {expected:?}, got {joined:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn text_only_edits_splice_hidden_rows_without_reordering() {
    let hide = HidePolicy::Configured {
        keys: vec!["secret".into()],
        sheet_cell: false,
    };
    for raw in [
        "Title\nheading:: true\nowner:: Ada\nbody",
        "Title\nsecret:: opaque  \ncard-repeats:: 2\nowner:: Ada\nbody",
        "Title\r\nheading:: true\r\nowner:: Ada\r\nbody",
        "Title\n:PROPERTIES:\n:heading: true\n:owner: Ada\n:END:\nbody",
    ] {
        let org = raw.contains(":PROPERTIES:");
        let parts = editor_text::split(raw, org, &hide);
        for (from, to) in [("Title", "Title X"), ("body", "body 2")] {
            let text = parts.visible.replacen(from, to, 1);
            let joined =
                editor_text::join_preserving(&text, &parts.hidden, org, raw, &hide).unwrap();
            assert_eq!(joined, raw.replacen(from, to, 1));
        }
        let text = parts
            .visible
            .replace("Title", "Title X")
            .replace("body", "body 2");
        let joined = editor_text::join_preserving(&text, &parts.hidden, org, raw, &hide).unwrap();
        assert_eq!(
            joined,
            raw.replace("Title", "Title X").replace("body", "body 2")
        );
    }
}

#[test]
fn raw_read_then_edit_does_not_duplicate_retained_properties() {
    let hide = HidePolicy::Configured {
        keys: vec!["secret".into()],
        sheet_cell: false,
    };
    for org in [false, true] {
        let raw = if org {
            "Title\n:PROPERTIES:\n:heading: true\n:secret: old\n:END:\nbody"
        } else {
            "Title\nheading:: true\nsecret:: old\nbody"
        };
        for raw in [
            raw.to_owned(),
            raw.replacen(
                "\nbody",
                "\n:LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:00:00]\n:END:\nbody",
                1,
            ),
        ] {
            for submitted in [
                raw.to_owned(),
                raw.replace("Title", "Edited"),
                raw.replace("old", "new"),
            ] {
                assert_eq!(
                    editor_text::set_text_with_policy(&raw, org, &submitted, &hide).unwrap(),
                    submitted
                );
            }
        }
        // A fenced example does not replace the block's own property.
        let text = if org {
            "Title\n#+BEGIN_SRC\n:secret: literal\n#+END_SRC"
        } else {
            "Title\n```\nsecret:: literal\n```"
        };
        let joined = editor_text::set_text_with_policy(raw, org, text, &hide).unwrap();
        assert!(joined.contains(if org { ":secret: old" } else { "secret:: old" }));
    }
}

#[test]
fn crlf_hidden_final_row_never_leaves_a_bare_cr() {
    let raw = "Title\r\nowner:: Ada\r\nbody\r\nheading:: true";
    let parts = editor_text::split(raw, false, &HidePolicy::Builtin);
    let visible = parts.visible.replace("body", "body 2");
    for joined in [
        editor_text::join_preserving(&visible, &parts.hidden, false, raw, &HidePolicy::Builtin)
            .unwrap(),
        editor_text::join(&visible, &parts.hidden, false).unwrap(),
    ] {
        assert!(!joined.ends_with('\r'));
        assert!(!joined.replace("\r\n", "").contains('\r'));
        assert_eq!(
            editor_text::split(&joined, false, &HidePolicy::Builtin)
                .hidden
                .trim_end_matches('\r'),
            parts.hidden
        );
    }
}

#[test]
fn crlf_property_only_tail_keeps_the_surviving_terminator() {
    let raw = "Title\r\nowner:: Ada\r\nheading:: true";
    let parts = editor_text::split(raw, false, &HidePolicy::Builtin);
    let joined = editor_text::join(
        &parts.visible.replace("Title", "Edited"),
        &parts.hidden,
        false,
    )
    .unwrap();
    assert_eq!(joined, raw.replace("Title", "Edited"));
}

#[test]
fn textarea_normalization_of_an_org_terminal_cr_keeps_the_source_layout() {
    for raw in [
        "Title\r\n:PROPERTIES:\r\n:heading: true\r\n:owner: Ada\r\n:END:\r",
        "Title\r\n:PROPERTIES:\r\n:heading: true\r\n:END:\r",
    ] {
        let parts = editor_text::split(raw, true, &HidePolicy::Builtin);
        let text = parts
            .visible
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace("Title", "Edited");
        let joined =
            editor_text::join_preserving(&text, &parts.hidden, true, raw, &HidePolicy::Builtin)
                .unwrap();
        assert_eq!(joined, raw.replace("Title", "Edited"));
    }
}

#[test]
fn merge_keeps_survivor_heading_and_drops_absorbed_heading_per_f2() {
    use tine_outline::outline::{self, Format, KeySource, OutlinePage, Verb};
    let page: OutlinePage = serde_json::from_value(serde_json::json!({
        "name": "P", "format": Format::Md, "roots": ["a", "b"],
        "blocks": {
            "a": { "raw": "Prev\nheading:: 2" },
            "b": { "raw": "Next\nheading:: true" }
        }
    }))
    .unwrap();
    let merged = outline::apply_with_policy(
        &[page],
        &Verb::Merge {
            survivor: "a".into(),
            absorbed: "b".into(),
            absorbed_referenced: false,
        },
        &KeySource::default(),
        &HidePolicy::Builtin,
    )
    .unwrap();
    let raw = &merged.pages[0]
        .upserts
        .iter()
        .find(|u| u.key == "a")
        .unwrap()
        .raw;
    assert!(raw.contains("heading:: 2"));
    assert!(!raw.contains("heading:: true"));
}

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
fn configured_projection_keeps_metadata_on_enter_and_drops_it_on_merge_per_f2() {
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
        assert_eq!(editor_text::split(&survivor.raw, org, &hide).hidden, "");
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
            "Title X\nbackground-color:: red\nowner:: Ada\nbody line",
        ),
        (
            "Title #card\ncard-repeats:: 2\nowner:: me\nAnswer",
            "Title X #card\ncard-repeats:: 2\nowner:: me\nAnswer",
        ),
        (
            "Title\nsecret:: s\nowner:: Ada\nbody",
            "Title X\nsecret:: s\nowner:: Ada\nbody",
        ),
        // No visible group: the hidden rows remain the block's only group.
        (
            "Title\nheading:: true\nbody line",
            "Title X\nheading:: true\nbody line",
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
