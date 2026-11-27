use super::*;
use crate::block_regions::parse;
use crate::doc::DocBlock;

fn seed(org: bool, keys: &[&str]) -> String {
    let keys: Vec<String> = keys.iter().map(|key| key.to_string()).collect();
    new_checkbox_row_raw(org, &keys)
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
        "end",
        "EnD",
        "END",
        "properties",
        "Properties",
        "PROPERTIES",
    ] {
        assert_only_ready_beside(key);
    }
}

#[test]
fn end_in_any_case_never_closes_or_opens_a_drawer() {
    for org in [false, true] {
        for keys in [
            ["EnD", "ready"],
            ["ready", "EnD"],
            ["end", "ready"],
            ["ready", "END"],
        ] {
            let raw = seed(org, &keys);
            assert_eq!(raw, ready_only(org), "{keys:?}");
            assert_eq!(raw.matches(":END:").count(), usize::from(org));
            assert_eq!(raw.matches(":PROPERTIES:").count(), usize::from(org));
        }
    }
}

#[test]
fn one_line_per_engine_key_under_the_first_spelling() {
    for org in [false, true] {
        for (keys, first) in [
            (["Shipped", "shipped"], "Shipped"),
            (["shipped", "Shipped"], "shipped"),
            (["a_b", "a-b"], "a_b"),
            (["a-b", "a_b"], "a-b"),
        ] {
            let raw = seed(org, &keys);
            let expected = if org {
                format!("\n:PROPERTIES:\n:{first}: false\n:END:")
            } else {
                format!("\n{first}:: false")
            };
            assert_eq!(raw, expected);
            let child = page_child(&raw, org, "\n");
            for spelling in keys {
                assert_eq!(
                    child.property(spelling).as_deref(),
                    Some("false"),
                    "{spelling}"
                );
            }
        }
        // A rejected spelling does not hold a later valid one back.
        assert_eq!(seed(org, &["id", "ready", "Ready"]), ready_only(org));
    }
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
                        format!("id{}", ":".repeat(i % 3 + 1))
                    }
                })
                .chain(["id".to_string(), "end".to_string()])
                .collect();
            let rejected: Vec<String> = (0..k).map(|i| format!("x {i}")).collect();
            for (name, keys, writes) in [
                ("valid", valid, 1),
                ("mixed", mixed, 1),
                ("all-rejected", rejected, 0),
            ] {
                WRITES.with(|n| n.set(0));
                let start = std::time::Instant::now();
                let raw = new_checkbox_row_raw(org, &keys);
                let elapsed = start.elapsed();
                assert_eq!(WRITES.with(|n| n.get()), writes, "{name} k={k} org={org}");
                assert_eq!(raw.is_empty(), writes == 0);
                eprintln!("seed org={org} k={k} {name}: {writes} writer call(s), {elapsed:?}");
            }
        }
    }
}
