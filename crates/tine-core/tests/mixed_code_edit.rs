//! GH #510 / I-2: the actual editor outputs in the shared fixture must save
//! without changing surrounding bytes and reparse as the same outline.
use tine_core::{doc, org, DocBlock, Document};

#[test]
fn mixed_code_editor_outputs_preserve_bytes_and_reparse() {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/mixed-code-edit.json")).unwrap();
    for c in cases.as_array().unwrap() {
        let field = |key: &str| c[key].as_str().unwrap();
        for crlf in [false, true] {
            let raw = format!("{}{}{}", field("prefix"), field("body"), field("suffix"));
            let next = format!(
                "{}{}{}",
                field("prefix"),
                field("replacement"),
                field("suffix")
            );
            let document = |body: &str| Document {
                pre_block: None,
                roots: vec![
                    DocBlock::new("before"),
                    DocBlock::new(body),
                    DocBlock::new("after"),
                ],
            };
            let serialize = |d: &Document| {
                let s = if field("format") == "org" {
                    org::serialize_org(d)
                } else {
                    doc::serialize(d)
                };
                if crlf {
                    s.replace('\n', "\r\n")
                } else {
                    s
                }
            };
            let before = serialize(&document(&raw));
            let saved = serialize(&document(&next));
            let disk = |s: &str| {
                if crlf {
                    s.replace('\n', "\r\n")
                } else {
                    s.to_owned()
                }
            };
            // Compare exactly the edited serialized body range, including
            // format-specific indentation, rather than only testing substrings.
            let expected = if field("format") == "org" {
                before.replace(&disk(field("body")), &disk(field("replacement")))
            } else {
                before.replace(
                    &disk(&field("body").replace('\n', "\n  ")),
                    &disk(&field("replacement").replace('\n', "\n  ")),
                )
            };
            assert_eq!(saved, expected, "I-2: outside code-body bytes changed");
            let parse = |s: &str| {
                if field("format") == "org" {
                    org::parse_org(s)
                } else {
                    doc::parse(s)
                }
            };
            let parsed = parse(&saved);
            let mut expected_tree = parse(&before);
            let old_body = if field("format") == "org" {
                disk(field("body"))
            } else {
                field("body").to_owned()
            };
            let new_body = if field("format") == "org" {
                disk(field("replacement"))
            } else {
                field("replacement").to_owned()
            };
            let edited = expected_tree.roots[1].raw().replace(&old_body, &new_body);
            expected_tree.roots[1].set_raw(edited);
            assert_eq!(
                parsed, expected_tree,
                "I-2: saved outline changed on reparse"
            );
            assert!(
                parsed.roots[1].children.is_empty(),
                "literal bullets must stay code"
            );
        }
    }
}
