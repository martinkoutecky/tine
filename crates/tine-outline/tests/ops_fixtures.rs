//! The outline kernel's verbs against the shared fixtures (outline-kernel
//! DESIGN §9). The same file drives the window's TS edits
//! (`src/document/edits/kernelFixtures.test.ts`), which recorded `expect`; a
//! difference between the two is investigated, never resolved by editing
//! `expect` alone.

use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use tine_outline::block_regions;
use tine_outline::editor_text::{self, HidePolicy};
use tine_outline::outline::{self, Block, Format, KeySource, OutlinePage, Verb};

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    page: Value,
    verb: Value,
    expect: Value,
}

fn page_of(v: &Value) -> OutlinePage {
    fn add(node: &Value, blocks: &mut HashMap<String, Block>) -> String {
        let key = node["key"].as_str().unwrap().to_string();
        let children = node["children"]
            .as_array()
            .map_or(vec![], |c| c.iter().map(|c| add(c, blocks)).collect());
        blocks.insert(
            key.clone(),
            Block {
                raw: node["raw"].as_str().unwrap().to_string(),
                children,
            },
        );
        key
    }
    let mut blocks = HashMap::new();
    let roots = v["roots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| add(r, &mut blocks))
        .collect();
    OutlinePage {
        name: v["name"].as_str().unwrap().to_string(),
        format: serde_json::from_value(v["format"].clone()).unwrap(),
        pre_block: None,
        roots,
        blocks,
        header_root: v["header_root"].as_str().map(str::to_string),
        read_only: v["read_only"].as_bool().unwrap_or(false),
        frozen: v["frozen"].as_bool().unwrap_or(false),
    }
}

fn tree(page: &OutlinePage, collapsed: &HashMap<String, bool>) -> Value {
    fn node(page: &OutlinePage, collapsed: &HashMap<String, bool>, key: &str) -> Value {
        let b = &page.blocks[key];
        let mut out = json!({ "key": key, "raw": b.raw });
        if !b.children.is_empty() {
            out["children"] = b
                .children
                .iter()
                .map(|c| node(page, collapsed, c))
                .collect();
        }
        if collapsed.get(key).copied().unwrap_or(false) {
            out["collapsed"] = json!(true);
        }
        out
    }
    let format = serde_json::to_value(page.format).unwrap();
    let roots: Vec<Value> = page
        .roots
        .iter()
        .map(|r| node(page, collapsed, r))
        .collect();
    json!({ "name": page.name, "format": format, "roots": roots })
}

/// The editor offset (UTF-16, built-in properties hidden) of a raw offset.
fn editor_offset(raw: &str, format: Format, raw_offset: usize) -> usize {
    let regions = block_regions::parse(raw, format.is_org());
    let p = HidePolicy::Builtin;
    let visible = editor_text::split_with(raw, format.is_org(), &regions, &p).visible;
    let at = editor_text::raw_to_visible_with(raw, format.is_org(), &regions, &p, raw_offset);
    editor_text::byte_to_utf16(&visible, at)
}

#[test]
fn m1_verbs_match_the_fixtures() {
    let fixture: Fixture = serde_json::from_str(include_str!("fixtures/ops/m1.json")).unwrap();
    assert!(fixture.cases.len() >= 40, "the M1 fixture set shrank");
    let mut failures = Vec::new();
    for case in &fixture.cases {
        let mut page = page_of(&case.page);
        let verb: Verb = serde_json::from_value(case.verb.clone()).unwrap();
        // The view collapse the fixtures assume (kernelFixtures.test.ts `toDto`).
        let mut collapsed = HashMap::new();
        if let Verb::Split {
            key,
            children_visible: false,
            ..
        } = &verb
        {
            if !page.blocks[key].children.is_empty() {
                collapsed.insert(key.clone(), true);
            }
        }
        let result = outline::apply(
            std::slice::from_ref(&page),
            &verb,
            &KeySource {
                prefix: "n".into(),
                next: 0,
            },
        );
        let got = match result {
            Err(reason) => {
                let mut v = json!({ "refused": true });
                if case.expect.get("reason").is_some() {
                    v["reason"] = serde_json::to_value(reason).unwrap();
                }
                v
            }
            Ok(change) => {
                let mut kinds = Value::Null;
                for patch in &change.pages {
                    patch.apply_to(&mut page);
                    for key in &patch.deleted {
                        collapsed.remove(key);
                    }
                    for u in &patch.upserts {
                        if let Some(c) = u.collapsed {
                            collapsed.insert(u.key.clone(), c);
                        }
                    }
                    kinds = serde_json::to_value(&patch.kinds).unwrap();
                }
                let focus = change.focus.map(|f| {
                    let offset = f
                        .raw_offset
                        .map(|o| editor_offset(&page.blocks[&f.key].raw, page.format, o));
                    json!({ "key": f.key, "editor_offset": offset })
                });
                json!({ "page": tree(&page, &collapsed), "kinds": kinds, "focus": focus })
            }
        };
        if got != case.expect {
            failures.push(format!(
                "{}\n  want {}\n  got  {}",
                case.name,
                serde_json::to_string(&case.expect).unwrap(),
                serde_json::to_string(&got).unwrap()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases differ:\n{}",
        failures.len(),
        fixture.cases.len(),
        failures.join("\n")
    );
}
