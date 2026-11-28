//! A property write, an editor save or a merge never deletes a drawer's
//! `:PROPERTIES:` or `:END:` line, the text folded beside it, or the block's id,
//! and never panics, whatever shares the delimiter's line.
use tine_outline::{
    block_regions::{self, Edit},
    editor_text::{self, HidePolicy},
    outline::{self, KeySource, OutlinePage, Reason, Verb},
    page_property_edit,
};

fn write(org: bool, raw: &str, key: &str, value: Option<&str>) -> String {
    let edit = Edit::Property {
        key: key.into(),
        value: value.map(Into::into),
        exact: false,
    };
    let run = std::panic::catch_unwind(|| block_regions::edit(raw, org, edit));
    match run {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => format!("ERR {e}"),
        Err(_) => "PANIC".into(),
    }
}

fn crlf(s: &str) -> String {
    s.replace('\n', "\r\n")
}

/// `(org, raw, key, value, expected)`; each row also runs with CRLF line ends.
fn check_writes(rows: &[(bool, &str, &str, Option<&str>, &str)], with_crlf: bool) {
    let mut failures = Vec::new();
    for &(org, raw, key, value, expected) in rows {
        let mut cases = vec![(raw.to_string(), expected.to_string())];
        if with_crlf {
            cases.push((crlf(raw), crlf(expected)));
        }
        for (raw, expected) in cases {
            let got = write(org, &raw, key, value);
            if got != expected {
                failures.push(format!(
                    "org={org} {raw:?} {key} {value:?}\n  want {expected:?}\n  got  {got:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_markdown_row_after_a_suffixed_closer_is_set_in_place() {
    let n = Some("N");
    check_writes(
        &[
            (
                false,
                "Title\n:PROPERTIES:\n:a: A\n:END: rest\nb:: B\nafter",
                "b",
                n,
                "Title\n:PROPERTIES:\n:a: A\n:END: rest\nb:: N\nafter",
            ),
            (
                false,
                "Title\n:PROPERTIES:\n:a: A\n:END: rest\nb:: B",
                "b",
                n,
                "Title\n:PROPERTIES:\n:a: A\n:END: rest\nb:: N",
            ),
            (
                false,
                ":PROPERTIES:\n:a: A\n:END: rest\nb:: B\nafter",
                "b",
                n,
                ":PROPERTIES:\n:a: A\n:END: rest\nb:: N\nafter",
            ),
        ],
        true,
    );
}

#[test]
fn a_markdown_drawer_after_a_bare_cr_closer_is_set_in_place() {
    let n = Some("N");
    let two = "T\n:PROPERTIES:\n:a: A\n:END:\r:PROPERTIES:\n:b: B\n:END:\nafter";
    let two_set = "T\n:PROPERTIES:\n:a: A\n:END:\r:PROPERTIES:\n:b:  N\n:END:\nafter";
    let double = "T\n:PROPERTIES:\n:a: A\n:END:\r\r:PROPERTIES:\n:b: B\n:END:\nafter";
    let double_set = "T\n:PROPERTIES:\n:a: A\n:END:\r\r:PROPERTIES:\n:b:  N\n:END:\nafter";
    let three =
        "T\n:PROPERTIES:\n:a: A\n:END:\r:PROPERTIES:\n:b: B\n:END:\r:PROPERTIES:\n:b: C\n:END:\nafter";
    // The duplicate `b` row goes, as every Set removes duplicates.
    let three_set =
        "T\n:PROPERTIES:\n:a: A\n:END:\r:PROPERTIES:\n:b:  N\n:END:\r:PROPERTIES:\n:END:\nafter";
    let titleless = &two[2..];
    let titleless_set = &two_set[2..];
    let id_row = two.replace(":a: A", ":id: A");
    let id_row_set = two_set.replace(":a: A", ":id: A");
    check_writes(
        &[
            (false, two, "b", n, two_set),
            (false, titleless, "b", n, titleless_set),
            (false, &id_row, "b", n, &id_row_set),
            (false, double, "b", n, double_set),
            (false, three, "b", n, three_set),
        ],
        false,
    );
    check_writes(&[(false, three, "b", n, three_set)], true);
}

#[test]
fn a_delimiter_line_is_never_a_property_row() {
    let n = Some("N");
    let x1 = "Title\n:PROPERTIES:\n:ready: false\n:properties: true\n:END:\nafter";
    let end_directive = "Title\n:PROPERTIES:\n:a: A\n:END:\n#+END: own\nafter";
    check_writes(
        &[
            // Markdown: a folded `:END: tail` is the closer, not an `end` row.
            (
                false,
                "Title\nend:: 1\n:PROPERTIES:\n:a: A\n:END: tail\nafter",
                "end",
                None,
                "Title\n:PROPERTIES:\n:a: A\n:END: tail\nafter",
            ),
            (
                false,
                "Title\nend:: 1\n:PROPERTIES:\n:a: A\n:END: tail\nafter",
                "end",
                n,
                "Title\nend:: N\n:PROPERTIES:\n:a: A\n:END: tail\nafter",
            ),
            (
                false,
                "Title\nend:: 1\n:PROPERTIES:\n:a: A\n:END:\nafter",
                "end",
                None,
                "Title\n:PROPERTIES:\n:a: A\n:END:\nafter",
            ),
            // Org: a `properties` or `END` key leaves the drawer's frame alone.
            (
                true,
                x1,
                "properties",
                None,
                "Title\n:PROPERTIES:\n:ready: false\n:END:\nafter",
            ),
            (
                true,
                x1,
                "properties",
                n,
                "Title\n:PROPERTIES:\n:ready: false\n:properties: N\n:END:\nafter",
            ),
            (
                true,
                end_directive,
                "END",
                None,
                "Title\n:PROPERTIES:\n:a: A\n:END:\nafter",
            ),
            (
                true,
                end_directive,
                "END",
                n,
                "Title\n:PROPERTIES:\n:a: A\n:END:\n#+END: N\nafter",
            ),
        ],
        true,
    );
}

fn page_write(org: bool, part: &str, key: &str, value: Option<&str>) -> String {
    let parts = vec![part.to_string()];
    let run = std::panic::catch_unwind(|| page_property_edit::set(&parts, org, key, value));
    match run {
        Ok(Ok(out)) => out.concat(),
        Ok(Err(e)) => format!("ERR {e}"),
        Err(_) => "PANIC".into(),
    }
}

#[test]
fn the_page_writer_keeps_the_delimiters_and_a_directive_beside_a_closer() {
    let shared = ":PROPERTIES:\n:a: A\n:END:#+a b: c\nafter";
    let prose = "prose\r#+x: B\nbody";
    let mut rows = vec![
        (shared.to_string(), "a b", None, shared.to_string()),
        (
            shared.to_string(),
            "a b",
            Some("N"),
            format!("#+a b: N\n{shared}"),
        ),
        (
            crlf(shared),
            "a b",
            Some("N"),
            format!("#+a b: N\n{}", crlf(shared)),
        ),
        (crlf(shared), "a b", None, crlf(shared)),
        (
            ":PROPERTIES:\n:ready: false\n:properties: true\n:END:\nafter".into(),
            "properties",
            Some("N"),
            ":PROPERTIES:\n:ready: false\n:properties: N\n:END:\nafter".into(),
        ),
        (
            ":PROPERTIES:\n:a: A\n:END:\n#+END: own\nafter".into(),
            "END",
            Some("N"),
            ":PROPERTIES:\n:a: A\n:END:\n#+end: N\nafter".into(),
        ),
        // A directive after a bare CR does not own the prose before it.
        (prose.into(), "x", None, prose.into()),
        (prose.into(), "x", Some("N"), format!("#+x: N\n{prose}")),
    ];
    let mut failures = Vec::new();
    for (part, key, value, expected) in rows.drain(..) {
        let got = page_write(true, &part, key, value);
        if got != expected {
            failures.push(format!(
                "{part:?} {key} {value:?}\n  want {expected:?}\n  got  {got:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn one_char_edit(v: &str) -> String {
    match v.find('\n') {
        Some(i) => format!("{}x{}", &v[..i], &v[i..]),
        None => format!("{v}x"),
    }
}

/// An Org drawer whose delimiters are not its region's own two lines shows as
/// text, and every save writes back exactly what the editor shows.
#[test]
fn the_editor_shows_and_saves_a_folded_org_drawer_as_text() {
    let inputs = [
        "Title\n:PROPERTIES:\n:id: A\n:END:\n:PROPERTIES:\n:collapsed: true\n:END: keep\nafter",
        "Title\n:PROPERTIES:\n:id: A\n:END: keep\nafter",
        ":PROPERTIES:\n:ID: U\n:END:\n#+title: Foo\n",
        ":PROPERTIES:\n:ID: U\n:END:\n#+title: Foo\nbody",
        "Title\n:PROPERTIES:\n:ID: U\n:END:\n#+title: Foo\nbody",
        "Title\n:PROPERTIES:\n:a: A\n:END:\n#+END: own\nafter",
        "Title\n:PROPERTIES:\n:a: A\n:END: \nafter",
    ];
    let mut failures = Vec::new();
    for input in inputs {
        for raw in [input.to_string(), crlf(input)] {
            for policy in [HidePolicy::Builtin, HidePolicy::SheetCell] {
                let run = std::panic::catch_unwind(|| {
                    let split = editor_text::split(&raw, true, &policy);
                    let edited = one_char_edit(&split.visible);
                    (
                        split.clone(),
                        editor_text::set_text_with_policy(&raw, true, &split.visible, &policy),
                        editor_text::join(&split.visible, &split.hidden, true),
                        editor_text::set_text_with_policy(&raw, true, &edited, &policy),
                    )
                });
                let Ok((split, save, joined, save1)) = run else {
                    failures.push(format!("{raw:?} {policy:?}: PANIC"));
                    continue;
                };
                let want = (raw.as_str(), "");
                let got = (split.visible.as_str(), split.hidden.as_str());
                if got != want {
                    failures.push(format!("{raw:?} {policy:?}: split {got:?}"));
                }
                if save.as_deref() != Ok(raw.as_str()) {
                    failures.push(format!("{raw:?} {policy:?}: save {save:?}"));
                }
                if joined.as_deref() != Ok(raw.as_str()) {
                    failures.push(format!("{raw:?} {policy:?}: join {joined:?}"));
                }
                if save1.as_deref() != Ok(one_char_edit(&raw).as_str()) {
                    failures.push(format!("{raw:?} {policy:?}: edit {save1:?}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn org_page(blocks: &[(&str, &str)]) -> OutlinePage {
    let map: serde_json::Map<_, _> = blocks
        .iter()
        .map(|(k, raw)| (k.to_string(), serde_json::json!({ "raw": raw })))
        .collect();
    let roots: Vec<_> = blocks.iter().map(|(k, _)| *k).collect();
    serde_json::from_value(serde_json::json!({
        "name": "P", "format": "org", "roots": roots, "blocks": map
    }))
    .unwrap()
}

/// A merge would show a folded drawer as the survivor's own text and drop the
/// survivor's hidden id, so it refuses and both blocks stay as they are.
#[test]
fn a_merge_refuses_a_block_with_a_folded_org_drawer() {
    let plain = "S\n:PROPERTIES:\n:id: A\n:END:";
    let folded = [
        "L\n:PROPERTIES:\n:id: B\n:END: keep",
        "L\n:PROPERTIES:\n:id: B\n:END: keep\nafter",
        "L\n:PROPERTIES:\n:id: B\n:END:\n#+title: T",
        ":PROPERTIES:\n:id: B\n:END: keep",
    ];
    let mut failures = Vec::new();
    for gone in folded {
        for (keep, absorbed) in [(plain, gone), (gone, plain)] {
            for raws in [
                (keep.to_string(), absorbed.to_string()),
                (crlf(keep), crlf(absorbed)),
            ] {
                let page = org_page(&[("a", &raws.0), ("b", &raws.1)]);
                for referenced in [false, true] {
                    let verbs = [
                        Verb::Merge {
                            survivor: "a".into(),
                            absorbed: "b".into(),
                            absorbed_referenced: referenced,
                        },
                        Verb::MergeMany {
                            survivor: "a".into(),
                            absorbed: vec!["b".into()],
                            referenced: if referenced { vec!["b".into()] } else { vec![] },
                        },
                    ];
                    for verb in verbs {
                        let got = outline::apply_with_policy(
                            std::slice::from_ref(&page),
                            &verb,
                            &KeySource::default(),
                            &HidePolicy::Builtin,
                        );
                        if !matches!(got, Err(Reason::Unparseable)) {
                            failures.push(format!("{raws:?} {verb:?}: {got:?}"));
                        }
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
