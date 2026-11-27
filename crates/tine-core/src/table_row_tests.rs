use super::*;
use crate::block_regions::parse;
use crate::doc::DocBlock;

fn seed(org: bool, keys: &[&str]) -> String {
    let keys: Vec<String> = keys.iter().map(|key| key.to_string()).collect();
    new_checkbox_row_raw(org, &keys, &[])
}

/// The row for checkbox columns `keys` beside other property columns `others`.
fn seed_beside(org: bool, keys: &[&str], others: &[&str]) -> String {
    let own = |list: &[&str]| list.iter().map(|key| key.to_string()).collect::<Vec<_>>();
    new_checkbox_row_raw(org, &own(keys), &own(others))
}

/// The row as written with only `ready` admitted.
fn ready_only(org: bool) -> &'static str {
    if org {
        "\n:PROPERTIES:\n:ready: false\n:END:"
    } else {
        "\nready:: false"
    }
}

/// The row read back as the only child of an owner block on a saved page
/// (LF or CRLF), by the page reader the store uses.
fn page_child(raw: &str, org: bool, nl: &str) -> DocBlock {
    let (head, child) = if org {
        ("* Owner", "**")
    } else {
        ("- Owner", "\t-")
    };
    let mut lines = raw.split('\n');
    let mut page = format!("{head}\n{child}");
    if let Some(first) = lines.next().filter(|line| !line.is_empty()) {
        page += &format!(" {first}");
    }
    for line in lines {
        page += &if org {
            format!("\n{line}")
        } else {
            format!("\n\t  {line}")
        };
    }
    let page = page.replace('\n', nl);
    let doc = if org {
        crate::org::parse_org(&page)
    } else {
        crate::doc::parse(&page)
    };
    assert_eq!(doc.roots.len(), 1, "{page:?}");
    assert_eq!(doc.roots[0].children.len(), 1, "{page:?}");
    doc.roots[0].children[0].clone()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Only `ready` is written, whichever side of it `other` stands, and the
/// saved page reads exactly `ready=false` back (LF and CRLF).
fn assert_only_ready_beside(other: &str) {
    for org in [false, true] {
        for keys in [[other, "ready"], ["ready", other]] {
            let raw = seed(org, &keys);
            assert_eq!(raw, ready_only(org), "{other:?} org={org}");
            for nl in ["\n", "\r\n"] {
                let child = page_child(&raw, org, nl);
                assert_eq!(
                    child.properties(),
                    pairs(&[("ready", "false")]),
                    "{other:?} org={org}"
                );
            }
        }
        assert_eq!(seed(org, &[other]), "", "{other:?} alone, org={org}");
    }
}

#[test]
fn writes_false_for_each_plain_column_below_an_empty_title_line() {
    assert_eq!(
        seed(false, &["shipped", "ready"]),
        "\nshipped:: false\nready:: false"
    );
    assert_eq!(
        seed(true, &["shipped", "ready"]),
        "\n:PROPERTIES:\n:shipped: false\n:ready: false\n:END:"
    );
    for org in [false, true] {
        assert_eq!(seed(org, &[]), "");
        for nl in ["\n", "\r\n"] {
            let child = page_child(&seed(org, &["shipped", "ready"]), org, nl);
            assert_eq!(child.raw(), seed(org, &["shipped", "ready"]));
            assert_eq!(
                child.properties(),
                pairs(&[("shipped", "false"), ("ready", "false")])
            );
        }
    }
}

#[test]
fn never_writes_a_reserved_key() {
    for key in [
        "id",
        "Id",
        "ID",
        "template",
        "custom-id",
        "custom_id",
        "collapsed",
        "Collapsed",
        "heading",
        "Tags",
        "TAGS",
        "query-sort-by",
        "created-at",
        "card-last-score",
        "done",
        "Done",
        "tine.quote",
        "tine.author",
        "tine/exports",
        "tine/favorites",
        "tine/journal-sections",
        "\u{85}tine.quote",
    ] {
        assert_only_ready_beside(key);
    }
}

#[test]
fn never_writes_the_asset_pointer_so_an_asset_named_false_stays_unreferenced() {
    for key in ["file-path", "FILE-PATH", "File-Path"] {
        assert_only_ready_beside(key);
        for org in [false, true] {
            let mut live = std::collections::HashSet::new();
            crate::asset_refs::collect_live_asset_refs(&seed(org, &[key, "ready"]), &mut live);
            assert!(live.is_empty(), "{key} org={org}: {live:?}");
        }
    }
    // Control: the same value written by the property writer names the asset.
    let clicked = block_regions::edit(
        "\n",
        false,
        Edit::Property {
            key: "file-path".into(),
            value: Some("false".into()),
        },
    )
    .unwrap();
    let mut live = std::collections::HashSet::new();
    crate::asset_refs::collect_live_asset_refs(&clicked, &mut live);
    assert!(live.contains("false"), "{live:?}");
}

#[test]
fn never_writes_a_key_that_is_not_plain() {
    let quotes = ">".repeat(1025);
    for key in [
        "\u{85}ship",
        "ship\u{85}",
        "some\tkey",
        "some\u{c}key",
        "some\u{1a}key",
        quotes.as_str(),
        "some key",
        "État",
        "état",
        "Δ",
        "\u{212a}",
        "a:b",
        "a.b",
        "a/b",
        "logseq.x",
        "1abc",
        "_x",
        "-x",
        "",
    ] {
        assert_only_ready_beside(key);
    }
}

/// In Org, a column named `end` or `properties` leaves the whole row empty
/// (a later click on it would put that entry in the seeded drawer, where the
/// drawer's own lines are mistaken for it); in Markdown only that column stays
/// unset.
#[test]
fn a_drawer_word_column_leaves_an_org_row_empty() {
    for word in [
        "end",
        "EnD",
        "END",
        "properties",
        "Properties",
        "PROPERTIES",
    ] {
        for keys in [[word, "ready"], ["ready", word]] {
            assert_eq!(seed(true, &keys), "", "{keys:?}");
            assert_eq!(seed(false, &keys), ready_only(false), "{keys:?}");
        }
        assert_eq!(seed(true, &[word]), "");
        assert_eq!(seed(false, &[word]), "");
    }
}

/// In the Org row a seed would otherwise write, clicking
/// `properties` on and then off loses `ready`; the seed now writes no such row.
#[test]
fn clicking_properties_on_and_off_cannot_follow_a_seed() {
    assert_eq!(seed(true, &["ready", "properties"]), "");
    // The empty row is develop's: the same clicks start from a blank line.
    let blank = seed(true, &["ready", "properties"]);
    assert!(parse(&blank, true).properties.is_empty());
}

/// Two columns that are the engine's same key (any ASCII case, `_` or `-` or
/// space for each other) leave the whole row empty, in both formats: a click on
/// one would write the other's line, or leave a `false` the query still finds.
#[test]
fn columns_that_are_one_key_leave_the_row_empty() {
    for org in [false, true] {
        for keys in [
            ["Shipped", "shipped"],
            ["shipped", "Shipped"],
            ["a_b", "a-b"],
            ["a-b", "a_b"],
            ["Ship_Ped", "ship-ped"],
            ["ship-ped", "Ship_Ped"],
        ] {
            assert_eq!(seed(org, &keys), "", "{keys:?} org={org}");
            let mut with_ready = vec!["ready"];
            with_ready.extend(keys);
            assert_eq!(seed(org, &with_ready), "", "{keys:?} org={org}");
        }
        // A same-key pair counts even when neither spelling is admitted.
        assert_eq!(seed(org, &["some key", "some-key", "ready"]), "");
        assert_eq!(seed(org, &["ready", "Ready"]), "");
        // Distinct keys beside a rejected one still get their value.
        assert_eq!(seed(org, &["id", "ready"]), ready_only(org));
    }
}

/// Any property column of the schema counts, not only checkbox ones: a text
/// column `Ready` beside checkbox `ready` (typing in it would overwrite the
/// seeded line), and in Org a text column `properties` (editing it would
/// break the seeded drawer), leave the row empty. Other columns that are
/// neither do not hold the seed back.
#[test]
fn a_risky_column_of_any_type_leaves_the_row_empty() {
    for org in [false, true] {
        for other in ["Ready", "READY", "ready"] {
            assert_eq!(
                seed_beside(org, &["ready"], &[other]),
                "",
                "{other} org={org}"
            );
        }
        assert_eq!(seed_beside(org, &["ready"], &["note", "Note"]), "");
        assert_eq!(
            seed_beside(org, &["ready"], &["note", "due"]),
            ready_only(org)
        );
    }
    for word in ["properties", "Properties", "PROPERTIES", "end", "End"] {
        assert_eq!(seed_beside(true, &["ready"], &[word]), "", "{word}");
        assert_eq!(
            seed_beside(false, &["ready"], &[word]),
            ready_only(false),
            "{word}"
        );
    }
}

/// A text column beside a checkbox one, through the edits a user then makes, on the row
/// the seed now writes (develop's empty row): nothing seeded is lost.
#[test]
fn typing_in_a_risky_text_column_finds_no_seed_to_lose() {
    for org in [false, true] {
        let raw = seed_beside(org, &["ready"], &["Ready"]);
        let typed = block_regions::edit(
            &raw,
            org,
            Edit::Property {
                key: "Ready".into(),
                value: Some("x".into()),
            },
        )
        .unwrap();
        assert_eq!(parse(&typed, org).properties.len(), 1, "org={org}");
        assert_eq!(parse(&typed, org).properties[0].value, "x");
    }
    let raw = seed_beside(true, &["ready"], &["properties"]);
    assert_eq!(raw, "");
    assert!(!raw.contains(":ready:"));
}

#[test]
fn the_kelvin_sign_is_a_different_key_from_k() {
    for org in [false, true] {
        for keys in [["K", "\u{212a}"], ["\u{212a}", "K"]] {
            let child = page_child(&seed(org, &keys), org, "\n");
            assert_eq!(child.properties(), pairs(&[("K", "false")]));
            assert_eq!(child.property("\u{212a}"), None);
        }
    }
}

#[test]
fn a_linkable_key_gets_the_bytes_a_click_writes() {
    for org in [false, true] {
        let clicked = block_regions::edit(
            "\n",
            org,
            Edit::Property {
                key: "shipped".into(),
                value: Some("false".into()),
            },
        )
        .unwrap();
        assert_eq!(seed(org, &["shipped"]), clicked);
    }
}

/// A small deterministic generator of plain keys: an ASCII letter, then
/// letters, digits, `_` and `-`.
fn plain_keys(count: usize) -> Vec<String> {
    const FIRST: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    const REST: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-";
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move |n: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % n as u64) as usize
    };
    (0..count)
        .map(|_| {
            let len = 1 + next(12);
            let mut key = String::from(FIRST[next(FIRST.len())] as char);
            key.extend((1..len).map(|_| REST[next(REST.len())] as char));
            key
        })
        .collect()
}

#[test]
fn a_generated_plain_key_round_trips_through_a_click() {
    let mut seeded = 0;
    for key in plain_keys(400) {
        for org in [false, true] {
            let raw = seed(org, &[&key]);
            if raw.is_empty() {
                assert!(
                    is_og_built_in_property(&key)
                        || key.eq_ignore_ascii_case("end")
                        || key.eq_ignore_ascii_case("properties"),
                    "{key:?}"
                );
                continue;
            }
            seeded += 1;
            let read = |raw: &str| -> Vec<(String, String)> {
                parse(raw, org)
                    .properties
                    .iter()
                    .map(|p| (p.key.clone(), p.value.clone()))
                    .collect()
            };
            assert_eq!(read(&raw), pairs(&[(&key, "false")]), "{key:?} org={org}");
            assert_eq!(
                page_child(&raw, org, "\r\n").properties(),
                pairs(&[(&key, "false")])
            );
            let clicked = block_regions::edit(
                &raw,
                org,
                Edit::Property {
                    key: key.clone(),
                    value: Some("true".into()),
                },
            )
            .unwrap();
            assert_eq!(
                read(&clicked),
                pairs(&[(&key, "true")]),
                "{key:?} org={org}"
            );
        }
    }
    assert!(seeded > 700, "{seeded}");
}

/// Writer calls (each one parse of the blank title line) and the time one
/// seed takes, for k columns: one call whenever something is admitted, none
/// when nothing is, whatever k.
#[test]
fn one_writer_call_whatever_the_number_of_columns() {
    for org in [false, true] {
        for k in [1usize, 64, 512] {
            let valid: Vec<String> = (0..k).map(|i| format!("c{i}")).collect();
            let mixed: Vec<String> = (0..k)
                .map(|i| {
                    if i % 2 == 0 {
                        format!("c{i}")
                    } else {
                        format!("id{i}:")
                    }
                })
                .chain(["id".to_string()])
                .collect();
            let rejected: Vec<String> = (0..k).map(|i| format!("x {i}")).collect();
            let one_key: Vec<String> = valid.iter().cloned().chain(["C0".to_string()]).collect();
            for (name, keys, writes) in [
                ("valid", valid, 1),
                ("mixed", mixed, 1),
                ("all-rejected", rejected, 0),
                ("one-key-twice", one_key, 0),
            ] {
                WRITES.with(|n| n.set(0));
                let start = std::time::Instant::now();
                let raw = new_checkbox_row_raw(org, &keys, &[]);
                let elapsed = start.elapsed();
                assert_eq!(WRITES.with(|n| n.get()), writes, "{name} k={k} org={org}");
                assert_eq!(raw.is_empty(), writes == 0);
                eprintln!("seed org={org} k={k} {name}: {writes} writer call(s), {elapsed:?}");
            }
        }
    }
}
