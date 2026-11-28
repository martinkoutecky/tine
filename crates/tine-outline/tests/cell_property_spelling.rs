//! A table or board cell's property write (`Edit::Property { exact: true }`)
//! never changes a property line spelled differently from its column, such as
//! `ready::` under a `Ready` column: Logseq reads every spelling that agrees
//! under `property_key_norm` as one property. Rows without another spelling
//! write exactly as the non-cell (`exact: false`) form does.

use tine_outline::block_regions::{edit, Edit, OTHER_SPELLING_ERROR};

fn cell(key: &str, value: Option<&str>) -> Edit {
    Edit::Property {
        key: key.into(),
        value: value.map(str::to_string),
        exact: true,
    }
}

fn plain(key: &str, value: Option<&str>) -> Edit {
    Edit::Property {
        key: key.into(),
        value: value.map(str::to_string),
        exact: false,
    }
}

fn refused(other: &str) -> Result<String, String> {
    Err(format!("{OTHER_SPELLING_ERROR}{other}"))
}

const NEWLINES: [&str; 2] = ["\n", "\r\n"];

#[test]
fn a_cell_refuses_when_the_row_holds_another_case_of_its_key() {
    // One Markdown node and one Org drawer holding both spellings, in both
    // source orders; both columns, Set and Clear.
    for nl in NEWLINES {
        for org in [false, true] {
            for (first, second) in [("ready", "Ready"), ("Ready", "ready")] {
                let raw = if org {
                    format!("Row{nl}:PROPERTIES:{nl}:{first}: true{nl}:{second}: false{nl}:END:")
                } else {
                    format!("Row{nl}{first}:: true{nl}{second}:: false")
                };
                for (key, other) in [(first, second), (second, first)] {
                    for value in [Some("yes"), None] {
                        assert_eq!(
                            edit(&raw, org, cell(key, value)),
                            refused(other),
                            "{raw:?} org={org} key={key} value={value:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn a_cell_refuses_beside_a_single_other_spelling() {
    // `ready:: false` under a `Ready` text column (and the Org drawer form):
    // develop rewrote the checkbox line; a clear deleted it.
    for nl in NEWLINES {
        for (org, raw) in [
            (false, "Row\nready:: false"),
            (true, "Row\n:PROPERTIES:\n:ready: false\n:END:"),
        ] {
            let raw = raw.replace('\n', nl);
            for value in [Some("yes"), None] {
                assert_eq!(edit(&raw, org, cell("Ready", value)), refused("ready"));
            }
        }
    }
}

#[test]
fn a_cell_refuses_beside_a_separator_spelling_of_its_key() {
    // Develop added a second line Logseq merges with the first.
    for nl in NEWLINES {
        for (key, line, other) in [
            ("a_b", "a-b:: false", "a-b"),
            ("a-b", "a_b:: false", "a_b"),
            ("Ship_Ped", "ship-ped:: true", "ship-ped"),
            ("ship-ped", "Ship_Ped:: true", "Ship_Ped"),
            ("Shipped", "shipped:: true", "shipped"),
        ] {
            let raw = format!("Row{nl}{line}");
            for value in [Some("true"), None] {
                let got = edit(&raw, false, cell(key, value));
                assert_eq!(got, refused(other), "{raw:?} {key}");
            }
        }
    }
}

#[test]
fn a_cell_without_another_spelling_writes_what_the_plain_edit_writes() {
    for nl in NEWLINES {
        let drawer = "Row\n:PROPERTIES:\n:ready: false\n:END:";
        let cases = [
            (false, "Row\nready:: false", "ready", Some("true")),
            (false, "Row", "Ready", Some("yes")),
            (false, "Row\nready:: false", "ready", None),
            (false, "Row\nready:: a\nready:: b", "ready", Some("c")),
            // A lookalike in a code block is not a property.
            (false, "Row\n```\nReady:: x\n```", "ready", Some("true")),
            (true, drawer, "ready", Some("true")),
            (true, drawer, "ready", None),
        ];
        for (org, raw, key, value) in cases {
            let raw = raw.replace('\n', nl);
            let expected = edit(&raw, org, plain(key, value));
            assert!(expected.is_ok(), "{raw:?}");
            assert_eq!(edit(&raw, org, cell(key, value)), expected, "{raw:?} {key}");
        }
    }
}

#[test]
fn a_non_cell_edit_still_folds_case_and_an_invalid_key_keeps_its_error() {
    assert_eq!(
        edit("Row\ncollapsed:: true", false, plain("Collapsed", None)),
        Ok("Row".into())
    );
    assert_eq!(
        edit("Row\nbad:: x", false, cell("bad:key", Some("y"))),
        Err("Invalid property edit".into())
    );
}

#[test]
fn the_gate_reads_only_the_region_the_write_touches() {
    // An Org block's other drawers are not where its properties are written.
    let raw = "Row\n:PROPERTIES:\n:ready: false\n:END:\nBody\n:PROPERTIES:\n:Ready: x\n:END:";
    let spellings: Vec<_> = tine_outline::block_regions::parse(raw, true)
        .properties
        .iter()
        .map(|p| p.key_range.slice(raw).to_string())
        .collect();
    // Both drawers are parsed properties.
    assert_eq!(spellings, ["ready", "Ready"]);
    assert_eq!(
        edit(raw, true, cell("ready", Some("true"))),
        edit(raw, true, plain("ready", Some("true")))
    );
}
