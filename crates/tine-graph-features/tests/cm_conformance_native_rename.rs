//! Accepted Native15c admission on real writer bytes. The r13 descriptor
//! engine remains a historical diagnostic and cannot certify this boundary.
use super::*;
use shared::native;

fn day_value(day: tine_core::date::JournalDate) -> String {
    day.file_stem()
}

fn file(spec: &GraphSpec, path: &str, text: &str) -> Value {
    let mut file = shared::file_json(
        path,
        text,
        shared::alias_spellings,
        spec.name_format(),
        &spec.config(),
    );
    native::decorate(&mut file, &spec.config());
    file
}

fn save(store: &Store, path: &str, bytes: &str) -> Value {
    let org = path.ends_with(".org");
    let document = shared::parse_file(path, bytes);
    let id = PageId::from(path);
    let loaded = store.page(&id).unwrap();
    let mut plan = store.page_plan(Some(tine_store::EditKind::SaveBlock));
    plan.save_page(
        &[tine_store::EditKind::SaveBlock],
        &id,
        tine_store::SaveBase::Existing(loaded.rev),
        &loaded.doc,
    );
    let operation = plan
        .plan()
        .expect("the real format-specific save path accepts the output");
    let planned = operation
        .pages
        .iter()
        .find_map(|(page, effect, _)| match effect {
            tine_store::HostEffect::Write(bytes) if *page == id => {
                Some(std::str::from_utf8(bytes).unwrap())
            }
            _ => None,
        })
        .unwrap_or(bytes);
    // Observe the actual format-specific serialization boundary, including
    // native layout/line-ending options and Markdown's equality retention.
    // Org gets its own exact serializer/firewall, with no Markdown fallback.
    let serialized = planned;
    let reparsed = shared::parse_file(path, serialized);
    json!({"path": path, "format": if org { "org" } else { "md" }, "bytes": bytes,
        "document": native::document(&document),
        "firstPropertiesOnly": !org && document.roots.first().is_some_and(|b| tine_core::page_header::is_page_header_candidate(b.raw())),
        "serialized": serialized, "reparsed": native::document(&reparsed), "planned": planned})
}

fn rename_ir(q: &Value, names: &std::collections::HashMap<String, String>) -> Value {
    match q {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, value)| {
                    let mapped = if matches!(key.as_str(), "refs" | "name") {
                        json!(
                            tine_core::refs::renamed_page_name(value.as_str().unwrap(), names)
                                .map(|s| s.into_owned())
                                .unwrap_or_else(|| value.as_str().unwrap().to_owned())
                        )
                    } else {
                        rename_ir(value, names)
                    };
                    (key.clone(), mapped)
                })
                .collect(),
        ),
        Value::Array(a) => json!(a.iter().map(|q| rename_ir(q, names)).collect::<Vec<_>>()),
        _ => q.clone(),
    }
}

pub fn packet(
    store: &Store,
    spec: &GraphSpec,
    old: &str,
    chosen: &str,
    source: &str,
    target: &str,
    proof: &MergeProposal,
    after_files: &BTreeMap<String, String>,
    queries: Value,
) -> Value {
    let map =
        std::collections::HashMap::from([(tine_core::refs::page_key(old), chosen.to_owned())]);
    let mut before: Vec<_> = spec
        .files
        .iter()
        .map(|(path, text)| file(spec, path, text))
        .collect();
    let mut shadows = Vec::new();
    let mut next = 0;
    for f in &mut before {
        let path = f["path"].as_str().unwrap().to_owned();
        let text = f["text"].as_str().unwrap().to_owned();
        let org = path.ends_with(".org");
        let mut rewrite =
            tine_core::refs::rename_proposal(&text, org, &map, &map, spec.name_format()).unwrap();
        let name = f["title"]
            .as_str()
            .filter(|_| !path.starts_with("journals/"))
            .unwrap_or_else(|| f["stem"].as_str().unwrap());
        let mapped = tine_core::refs::renamed_page_name(name, &map)
            .map(|s| s.into_owned())
            .unwrap_or_else(|| name.to_owned());
        let shadow_path = if path.starts_with("pages/") && mapped != name {
            if f["title"].is_string() {
                rewrite.rebind_title(org, &mapped).unwrap();
            }
            format!(
                "pages/{}.{}",
                tine_core::model::encode_page_name(&mapped, spec.name_format()),
                if org { "org" } else { "md" }
            )
        } else {
            path.clone()
        };
        let mut shadow = file(spec, &shadow_path, &rewrite.text);
        native::row_images(f, &mut shadow, old, chosen, spec, &mut next);
        shadows.push(shadow);
    }
    let mut ids = BTreeMap::new();
    for (index, path) in [target, source].into_iter().enumerate() {
        let input = before.iter_mut().find(|f| f["path"] == path).unwrap();
        let (pieces, local) =
            native::pieces(&proof.trace, index, &native::row_cuts(input), &mut next);
        input["pieces"] = json!(pieces);
        ids.extend(local);
    }
    let mut consumed: Vec<_> = proof
        .trace
        .consumed
        .iter()
        .flat_map(|id| ids[id].iter().copied())
        .collect();
    let mut placements =
        vec![json!({"path": target, "items": native::placements(&proof.trace, &ids)})];
    for (index, f) in before.iter_mut().enumerate() {
        let path = f["path"].as_str().unwrap();
        if path == target || path == source {
            continue;
        }
        let mut proposal = tine_core::refs::rename_proposal(
            f["text"].as_str().unwrap(),
            path.ends_with(".org"),
            &map,
            &map,
            spec.name_format(),
        )
        .unwrap();
        if f["dir"] == "pages" {
            if let Some(title) = f["title"].as_str() {
                if let Some(mapped) = tine_core::refs::renamed_page_name(title, &map) {
                    proposal
                        .rebind_title(path.ends_with(".org"), &mapped)
                        .unwrap();
                }
            }
        }
        let (parts, ids) = native::pieces(&proposal.trace, 0, &native::row_cuts(f), &mut next);
        placements.push(json!({"path": shadows[index]["path"], "items": native::placements(&proposal.trace, &ids)}));
        consumed.extend(
            proposal
                .trace
                .consumed
                .iter()
                .flat_map(|id| ids[id].iter().copied()),
        );
        f["pieces"] = json!(parts);
    }
    let after: Vec<_> = after_files
        .iter()
        .map(|(path, text)| file(spec, path, text))
        .collect();
    let target_before = before.iter().find(|f| f["path"] == target).unwrap();
    let header_count = target_before["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .take_while(|b| b["kind"] != "body")
        .count();
    let origins: Vec<Vec<Value>> = proof
        .origins
        .iter()
        .map(|row| {
            row.iter()
                .flat_map(|origin| match origin {
                    Origin::TargetHeader => (0..header_count).map(|j| json!([false, j])).collect(),
                    Origin::Target(j) => vec![json!([false, j])],
                    Origin::Source(j) => vec![json!([true, j])],
                    Origin::Leftover => vec![],
                })
                .collect()
        })
        .collect();
    let source_before = before.iter().find(|f| f["path"] == source).unwrap();
    let mut inputs = Vec::new();
    for (is_source, input) in [(true, source_before), (false, target_before)] {
        for (row, b) in input["blocks"].as_array().unwrap().iter().enumerate() {
            for (qi, q) in b["stored"].as_array().unwrap().iter().enumerate() {
                inputs.push((json!([is_source, row]), qi, rename_ir(q, &map), false));
            }
        }
    }
    let mut images = Vec::new();
    let output = after.iter().find(|f| f["path"] == target).unwrap();
    for (row, b) in output["blocks"].as_array().unwrap().iter().enumerate() {
        for (qi, q) in b["stored"].as_array().unwrap().iter().enumerate() {
            if let Some((origin, old_qi, _, used)) = inputs
                .iter_mut()
                .find(|(o, _, old_q, used)| !*used && origins[row].contains(o) && old_q == q)
            {
                *used = true;
                images.push(json!([origin, old_qi, row, qi]));
            }
        }
    }
    let mut extra = vec![old.to_owned(), chosen.to_owned()];
    for f in before.iter().chain(&shadows).chain(&after) {
        for b in f["blocks"].as_array().unwrap() {
            for q in b["stored"].as_array().unwrap() {
                shared::query_args(q, &mut extra);
            }
        }
    }
    for q in queries.as_array().unwrap() {
        shared::query_args(&q["q"], &mut extra);
    }
    let keys = shared::spellings(
        &json!({"files": before.iter().chain(&shadows).chain(&after).cloned().collect::<Vec<_>>()}),
        &extra,
    );
    let journal = spec.journal_format();
    let literal: BTreeMap<_, _> = keys
        .iter()
        .map(|s| (s.clone(), tine_core::refs::page_key(s)))
        .collect();
    let dates: BTreeMap<_, _> = keys
        .iter()
        .filter_map(|s| {
            journal
                .parse(s)
                .map(|d| (tine_core::refs::page_key(s), day_value(d)))
        })
        .collect();
    let days: BTreeMap<_, _> = before
        .iter()
        .chain(&shadows)
        .chain(&after)
        .filter_map(|f| {
            let path = f["path"].as_str().unwrap();
            if !path.starts_with("journals/") {
                return None;
            }
            journal
                .parse(Path::new(path).file_stem().unwrap().to_str().unwrap())
                .map(|day| (path.to_owned(), day_value(day)))
        })
        .collect();
    let saves: Vec<_> = after_files
        .iter()
        .map(|(path, text)| save(store, path, text))
        .collect();
    let packet = json!({"old": old, "chosen": chosen, "source": source, "target": target,
        "before": before, "shadows": shadows, "after": after, "origins": origins,
        "placements": placements, "consumed": consumed, "queryImages": images,
        "literalKeys": literal, "dateKeys": dates, "journalDays": days,
        "urls": keys.iter().filter(|s| tine_core::refs::url_has_origin(s)).collect::<Vec<_>>(),
        "saves": saves, "queries": queries, "givenLeaves": [], "givenBlockHits": [], "givenPageHits": []});
    if let Ok(dest) = std::env::var("CM_NATIVE_REPORT") {
        fs::write(
            Path::new(&dest).with_file_name(format!("{}.packet.json", spec.name)),
            serde_json::to_string_pretty(&packet).unwrap(),
        )
        .unwrap();
    }
    packet
}

pub fn admit(bridge: &Path, packet: &Value) -> Value {
    let result = shared::run_bridge(bridge, packet)
        .expect("native bridge decodes complete Rust observations");
    assert_eq!(result["round"], "r15c");
    assert_eq!(
        result["accepted"], true,
        "native admission: {}",
        result["checks"]
    );
    assert!(result["checks"]
        .as_object()
        .unwrap()
        .values()
        .all(|v| v == true));
    assert!(result.get("checkedMeaning").is_some());
    result
}

pub fn assert_local_saves(packet: &Value) {
    for save in packet["saves"].as_array().unwrap() {
        assert_eq!(
            save["document"], save["reparsed"],
            "full native codec equality"
        );
        assert_eq!(
            save["bytes"], save["planned"],
            "actual unchanged PagePlan bytes"
        );
        if save["format"] == "org" {
            assert_eq!(save["serialized"], save["bytes"]);
        }
        let roots = save["document"]["roots"].as_array().unwrap();
        let promotes = save["format"] == "md"
            && save["document"]["preamble"]
                .as_str()
                .unwrap_or("")
                .is_empty()
            && roots
                .first()
                .is_some_and(|r| r["descendants"].as_array().unwrap().is_empty())
            && save["firstPropertiesOnly"] == true;
        assert!(
            !promotes,
            "an actual output must not promote on unchanged save"
        );
    }
}

#[test]
fn accepted_native_merge_rows_and_adversarial_controls() {
    let _guard = crate::integration_lock();
    let Ok(bridge) = std::env::var("CM_NATIVE_BRIDGE") else {
        assert!(
            std::env::var_os("CM_REQUIRE_BRIDGE").is_none(),
            "native conformance requires CM_NATIVE_BRIDGE"
        );
        eprintln!("CM_NATIVE_BRIDGE unset: native merge conformance skipped");
        return;
    };
    let corpus =
        std::env::var("CM_NATIVE_CORPUS").expect("the accepted 15c packet directory is required");
    let bridge = Path::new(&bridge);
    let mut report = BTreeMap::new();
    for name in [
        "r11-empty-alias-source",
        "r13-alias-edge-preview",
        "r13-protected-equal-header",
        "random-101",
        "random-149",
        "random-17",
        "random-198",
        "random-38",
        "random-4",
        "random-83",
    ] {
        let supplied: Value = serde_json::from_str(
            &fs::read_to_string(Path::new(&corpus).join(format!("fixture--{name}.observed.json")))
                .unwrap(),
        )
        .unwrap();
        let old = supplied["old"].as_str().unwrap();
        let chosen = supplied["chosen"].as_str().unwrap();
        let source = supplied["source"].as_str().unwrap();
        let target = supplied["target"].as_str().unwrap();
        let requested = match name {
            "r11-empty-alias-source" => "ML",
            "r13-alias-edge-preview" => "X",
            "random-198" => "a",
            "random-83" => "2026-10-26",
            "random-101" => "n/y",
            _ => chosen,
        };
        let mut spec = GraphSpec::default();
        spec.name = name.into();
        spec.files = supplied["before"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                (
                    f["path"].as_str().unwrap().to_owned(),
                    f["text"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        spec.renames = vec![(old.into(), requested.into())];
        let dir = tempfile::tempdir().unwrap();
        spec.write(dir.path());
        let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
        let proof = merge_proof(&store, &spec, old, requested, &Choice::Merge(target.into()))
            .unwrap()
            .2
            .unwrap();
        execute(&store, old, requested, Some(&Choice::Merge(target.into()))).unwrap();
        let files = read_graph(dir.path());
        assert_eq!(files[target], proof.text);
        let packet = packet(
            &store,
            &spec,
            old,
            chosen,
            source,
            target,
            &proof,
            &files,
            supplied["queries"].clone(),
        );
        let result = shared::run_bridge(bridge, &packet).unwrap();
        if matches!(name, "random-198" | "random-4") {
            let view = store.whole_graph().unwrap();
            let rows: Vec<_> = native::block_query_rows(&view, &supplied["queries"][0])
                .into_iter()
                .filter(|(path, _)| path == target)
                .map(|(_, i)| i)
                .collect();
            assert_eq!(
                json!(rows),
                if name == "random-198" {
                    json!([0, 1, 2, 3, 4, 5, 6, 8])
                } else {
                    json!([1, 3])
                },
                "real native query anchors"
            );
        }
        admit(bridge, &packet);
        let mut chosen_control = Value::Null;
        if matches!(name, "r11-empty-alias-source" | "random-198" | "random-83") {
            assert_eq!(
                result["checks"]["rewrite"], true,
                "independent chosen shadows: {name}: {}",
                result["checks"]
            );
            assert_eq!(
                result["checks"]["queryImages"], true,
                "B4 does not waive query identity"
            );
            assert_local_saves(&packet);
            let corrected_dir = tempfile::tempdir().unwrap();
            spec.write(corrected_dir.path());
            let corrected_store = Arc::new(
                Store::open(corrected_dir.path(), Default::default())
                    .unwrap()
                    .0,
            );
            let corrected_proof = merge_proof(
                &corrected_store,
                &spec,
                old,
                chosen,
                &Choice::Merge(target.into()),
            )
            .unwrap()
            .2
            .unwrap();
            execute(
                &corrected_store,
                old,
                chosen,
                Some(&Choice::Merge(target.into())),
            )
            .unwrap();
            let corrected_files = read_graph(corrected_dir.path());
            let mut corrected_spec = spec.clone();
            corrected_spec.name = format!("{name}.chosen-control");
            let corrected_packet = self::packet(
                &corrected_store,
                &corrected_spec,
                old,
                chosen,
                source,
                target,
                &corrected_proof,
                &corrected_files,
                supplied["queries"].clone(),
            );
            let correction = admit(bridge, &corrected_packet);
            assert_eq!(
                files, corrected_files,
                "typed and chosen requests must produce the same graph: {name}"
            );
            chosen_control = correction.clone();
            if name == "r11-empty-alias-source" {
                let edges = correction["checkedMeaning"]["typedAliasEdges"]
                    .as_array()
                    .unwrap();
                for key in ["ml", "machine learning"] {
                    assert!(edges.contains(&json!([["page", "machine learning"], ["page", key]])));
                }
            }
            if name == "random-198" {
                let actual: Vec<_> = correction["checkedMeaning"]["queries"][0]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|q| q["block"][1].clone())
                    .collect();
                assert_eq!(json!(actual), json!([0, 1, 2, 3, 4, 5, 6, 8]));
            }
        }
        let mut reversed_saves = packet.clone();
        reversed_saves["saves"].as_array_mut().unwrap().reverse();
        admit(bridge, &reversed_saves);
        // Every native output goes through the literal guarded second-save
        // path twice; the independent codec observations are also checked.
        let host = crate::test_host(&store);
        for (path, bytes) in &files {
            let id = PageId::from(path.as_str());
            for _ in 0..2 {
                let loaded = store.page(&id).unwrap();
                crate::test_host_writer::save_on(
                    &host,
                    &store,
                    tine_store::EditKind::SaveBlock,
                    &id,
                    tine_store::SaveBase::Existing(loaded.rev),
                    &loaded.doc,
                )
                .unwrap();
                assert_eq!(&fs::read_to_string(dir.path().join(path)).unwrap(), bytes);
            }
        }
        if name == "random-198" || name == "random-4" {
            let expected = if name == "random-198" {
                json!([0, 1, 2, 3, 4, 5, 6, 8])
            } else {
                json!([1, 3])
            };
            let actual: Vec<_> = result["checkedMeaning"]["queries"][0]
                .as_array()
                .unwrap()
                .iter()
                .map(|q| q["block"][1].clone())
                .collect();
            assert_eq!(json!(actual), expected);
        }
        let mut missing = packet.clone();
        missing["saves"] = json!([]);
        let rejected = shared::run_bridge(bridge, &missing).unwrap();
        assert_eq!(rejected["accepted"], false);
        assert_eq!(rejected["checks"]["unchangedSaveClosure"], false);
        let mut detached = packet.clone();
        detached["saves"][0]["bytes"] = json!("detached bytes");
        assert_eq!(
            shared::run_bridge(bridge, &detached).unwrap()["checks"]["unchangedSaveClosure"],
            false
        );
        report.insert(name, json!({"classification": "native admitted", "packet": packet, "result": result, "chosenControl": chosen_control}));
    }
    if let Ok(dest) = std::env::var("CM_NATIVE_REPORT") {
        fs::write(dest, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    }
    assert_eq!(
        report.len(),
        10,
        "all four FINDING and nine G7 rows covered"
    );
}

#[test]
fn native_valid_merges_with_duplicate_members_and_header_children() {
    let _guard = crate::integration_lock();
    let Ok(bridge) = std::env::var("CM_NATIVE_BRIDGE") else {
        assert!(std::env::var_os("CM_REQUIRE_BRIDGE").is_none());
        return;
    };
    for org in [false, true] {
        let (ext, key, bullet) = if org {
            ("org", "#+alias: ", "*")
        } else {
            ("md", "alias:: ", "-")
        };
        let source_path = format!("pages/src.{ext}");
        let target_path = format!("pages/dst.{ext}");
        let mut cases = Vec::new();
        for members in [
            "a,  a",
            "[[a]], [[a]]",
            "#a, #a",
            "#[[a]], #[[a]]",
            "[[a]], a, #a, [[b]], [[a]]",
            "[[a]], [[A]], [[a]]",
        ] {
            for retained_target in [false, true] {
                let alias = format!("{key}{members}\n");
                cases.push((
                    format!(
                        "{}{bullet} source\n",
                        if retained_target { "" } else { &alias }
                    ),
                    format!(
                        "{}{bullet} target\n",
                        if retained_target { &alias } else { "" }
                    ),
                ));
            }
        }
        cases.push(if org {
            (
                "#+alias: Former\n* child\n** deep\n* sibling\n* source\n".into(),
                "#+alias: Kept\n* target\n".into(),
            )
        } else {
            (
                "- alias:: Former\n  - child\n    - deep\n  - sibling\n- source\n".into(),
                "alias:: Kept\n- target\n".into(),
            )
        });
        for (source, target) in cases {
            let mut spec = GraphSpec::default();
            spec.name = "valid-merge".into();
            spec.files =
                BTreeMap::from([(source_path.clone(), source), (target_path.clone(), target)]);
            let dir = tempfile::tempdir().unwrap();
            spec.write(dir.path());
            let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
            let choice = Choice::Merge(target_path.clone());
            let proof = merge_proof(&store, &spec, "src", "dst", &choice)
                .unwrap()
                .2
                .unwrap();
            execute(&store, "src", "dst", Some(&choice)).unwrap();
            let files = read_graph(dir.path());
            let p = packet(
                &store,
                &spec,
                "src",
                "dst",
                &source_path,
                &target_path,
                &proof,
                &files,
                json!([]),
            );
            assert_local_saves(&p);
            admit(Path::new(&bridge), &p);
        }
    }
}

/// Native15c admits the actual outputs for rebuilt-field line endings, the
/// inserted Markdown lines of a CRLF survivor, and memberless alias values.
/// A promoted source field whose own ending differs from the model's field
/// ending is a model question (RECEIPT-cm-b4-fix2), so it is not listed here.
#[test]
fn native_valid_merges_keep_line_endings_and_memberless_aliases() {
    let _guard = crate::integration_lock();
    let Ok(bridge) = std::env::var("CM_NATIVE_BRIDGE") else {
        assert!(std::env::var_os("CM_REQUIRE_BRIDGE").is_none());
        return;
    };
    for org in [false, true] {
        let (ext, key, bullet, title) = if org {
            ("org", "#+alias: ", "*", "#+title: src")
        } else {
            ("md", "alias:: ", "-", "title:: src")
        };
        let source_path = format!("pages/src.{ext}");
        let target_path = format!("pages/dst.{ext}");
        let field = format!("{key}[[a]], [[a]]\n");
        let mut cases = Vec::new();
        for (src_end, dst_end) in [
            ("\n", "\n"),
            ("\r\n", "\r\n"),
            ("\r\n", "\n"),
            ("\n", "\r\n"),
        ] {
            for (source, target) in [
                (
                    format!("{bullet} source\n"),
                    format!("{field}{bullet} target\n"),
                ),
                (
                    format!("{field}{bullet} source\n"),
                    format!("{bullet} target\n"),
                ),
                (
                    format!("{title}\n{bullet} source\n"),
                    format!("{bullet} target\n"),
                ),
                (
                    format!("{bullet} source\n"),
                    format!("{bullet} target\n{bullet} last"),
                ),
            ] {
                cases.push((source.replace('\n', src_end), target.replace('\n', dst_end)));
            }
        }
        for none in [
            format!("{key}\"literal\"\n"),
            format!("{}\n", key.trim_end()),
        ] {
            let some = format!("{key}a\n");
            for ending in ["\n", "\r\n"] {
                for (n, (source, target)) in [
                    (
                        format!("{some}{bullet} source\n"),
                        format!("{none}{bullet} target\n"),
                    ),
                    (
                        format!("{none}{bullet} source\n"),
                        format!("{some}{bullet} target\n"),
                    ),
                    (
                        format!("{bullet} source\n"),
                        format!("{none}{some}{bullet} target\n"),
                    ),
                    (
                        format!("{none}{some}{bullet} source\n"),
                        format!("{bullet} target\n"),
                    ),
                ]
                .into_iter()
                .enumerate()
                {
                    // The last case promotes a source field into a survivor
                    // with none: the model's ending is LF there.
                    if ending == "\n" || n < 3 {
                        cases.push((source.replace('\n', ending), target.replace('\n', ending)));
                    }
                }
            }
        }
        for (source, target) in cases {
            let mut spec = GraphSpec::default();
            spec.name = "valid-merge".into();
            spec.files =
                BTreeMap::from([(source_path.clone(), source), (target_path.clone(), target)]);
            let dir = tempfile::tempdir().unwrap();
            spec.write(dir.path());
            let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
            let choice = Choice::Merge(target_path.clone());
            let proof = merge_proof(&store, &spec, "src", "dst", &choice)
                .unwrap()
                .2
                .unwrap();
            execute(&store, "src", "dst", Some(&choice)).unwrap();
            let files = read_graph(dir.path());
            let p = packet(
                &store,
                &spec,
                "src",
                "dst",
                &source_path,
                &target_path,
                &proof,
                &files,
                json!([]),
            );
            assert_local_saves(&p);
            admit(Path::new(&bridge), &p);
        }
    }
}

/// Native15c admission of lone-CR and mixed-ending merges
/// (REG-CM-MERGE-LONE-CR), the inputs of
/// `merge_text_trace::valid_merge_keeps_lone_cr_line_endings`. A source field
/// promoted into a survivor without one keeps its own CR, which round 15c's
/// field ending (LF) disagrees with: pending 15f, so it is not listed here.
/// Nor is a source opening with a blank line: Native15c rejects that in pure
/// LF too, at 04029874d (`mergeBodyOccurrences`; RECEIPT-cm-b4-fix3 F-BLANK).
#[test]
fn native_valid_merges_keep_lone_cr_line_endings() {
    let _guard = crate::integration_lock();
    let Ok(bridge) = std::env::var("CM_NATIVE_BRIDGE") else {
        assert!(std::env::var_os("CM_REQUIRE_BRIDGE").is_none());
        return;
    };
    let mut rejected = Vec::new();
    for org in [false, true] {
        let (ext, key, b, title) = if org {
            ("org", "#+alias: ", "*", "#+title: src")
        } else {
            ("md", "alias:: ", "-", "title:: src")
        };
        let source_path = format!("pages/src.{ext}");
        let target_path = format!("pages/dst.{ext}");
        let cases = [
            (format!("{b} source\r{b} second\r"), format!("{b} target\n")),
            (format!("{b} source\r"), format!("{b} target\r")),
            (format!("{b} source\r"), format!("{b} first\r{b} target")),
            (
                format!("{key}b\r{b} source\r"),
                format!("{key}[[a]], [[a]]\r{b} target\r"),
            ),
            (format!("{title}\r{b} source\r"), format!("{b} target\r")),
            (
                format!("{key}a, b\r{title}\r{b} source\r"),
                format!("{key}c\r{b} target\r"),
            ),
            (
                format!("{key}a\r\n{b} one\r{b} two\n{b} three\r\n"),
                format!("{key}b\n{b} target\r"),
            ),
            (
                format!("{title}\n{key}a\r{b} one\r\n"),
                format!("{key}[[b]], [[b]]\r\n{b} target\r{b} last"),
            ),
            (
                format!("{title}\r\n{b} source\n"),
                format!("{key}a\r\n\r{b} target\r"),
            ),
        ];
        for (source, target) in cases {
            let witness = format!("org={org} source={source:?} target={target:?}");
            let mut spec = GraphSpec::default();
            spec.name = "valid-merge".into();
            spec.files =
                BTreeMap::from([(source_path.clone(), source), (target_path.clone(), target)]);
            let dir = tempfile::tempdir().unwrap();
            spec.write(dir.path());
            let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
            let choice = Choice::Merge(target_path.clone());
            let proof = merge_proof(&store, &spec, "src", "dst", &choice)
                .unwrap()
                .2
                .unwrap();
            execute(&store, "src", "dst", Some(&choice)).unwrap();
            let files = read_graph(dir.path());
            let p = packet(
                &store,
                &spec,
                "src",
                "dst",
                &source_path,
                &target_path,
                &proof,
                &files,
                json!([]),
            );
            assert_local_saves(&p);
            let result = shared::run_bridge(Path::new(&bridge), &p).unwrap();
            let failed: Vec<_> = result["checks"]
                .as_object()
                .unwrap()
                .iter()
                .filter(|(_, v)| **v != true)
                .map(|(k, _)| k.clone())
                .collect();
            if result["accepted"] != true || !failed.is_empty() {
                rejected.push(format!("{witness}: {}", failed.join(",")));
            }
        }
    }
    assert!(
        rejected.is_empty(),
        "Native15c rejected:\n{}",
        rejected.join("\n")
    );
}

/// Native15c admission of valid Org merges whose pages carry a head drawer
/// (REG-CM-MERGE-ORG-DRAWER): in the target, the source or both, with and
/// without `:ALIAS:`, beside directives, empty, under each line ending.
#[test]
fn native_valid_merges_with_org_head_drawers_are_admitted() {
    let _guard = crate::integration_lock();
    let Ok(bridge) = std::env::var("CM_NATIVE_BRIDGE") else {
        assert!(std::env::var_os("CM_REQUIRE_BRIDGE").is_none());
        return;
    };
    let (source_path, target_path) = ("pages/src.org", "pages/dst.org");
    let mut rejected = Vec::new();
    let mut pending = Vec::new();
    for e in ["\n", "\r\n", "\r"] {
        for (source, target, ..) in crate::merge_text_trace::org_drawer_cases(e) {
            let witness = format!("source={source:?} target={target:?}");
            // Field endings rounds 15c and 15f disagree on are reported by
            // the generator, not asserted (RECEIPT-cm-b4-fix2 Q1).
            if crate::merge_generated::expected(&crate::merge_generated::org_case(&source, &target))
                .pending_15f()
            {
                pending.push(witness);
                continue;
            }
            let mut spec = GraphSpec::default();
            spec.name = "valid-merge".into();
            spec.files = BTreeMap::from([
                (source_path.to_owned(), source),
                (target_path.to_owned(), target),
            ]);
            let dir = tempfile::tempdir().unwrap();
            spec.write(dir.path());
            let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
            let choice = Choice::Merge(target_path.to_owned());
            let proof = merge_proof(&store, &spec, "src", "dst", &choice)
                .unwrap()
                .2
                .unwrap();
            execute(&store, "src", "dst", Some(&choice)).unwrap();
            let files = read_graph(dir.path());
            let p = packet(
                &store,
                &spec,
                "src",
                "dst",
                source_path,
                target_path,
                &proof,
                &files,
                json!([]),
            );
            assert_local_saves(&p);
            let result = shared::run_bridge(Path::new(&bridge), &p).unwrap();
            let failed: Vec<_> = result["checks"]
                .as_object()
                .unwrap()
                .iter()
                .filter(|(_, v)| **v != true)
                .map(|(k, _)| k.clone())
                .collect();
            if result["accepted"] != true || !failed.is_empty() {
                if let Ok(dest) = std::env::var("CM_NATIVE_REPORT") {
                    let name = format!("drawer-{}.packet.json", rejected.len());
                    fs::write(Path::new(&dest).with_file_name(name), p.to_string()).unwrap();
                }
                rejected.push(format!("{witness}: {}", failed.join(",")));
            }
        }
    }
    eprintln!("pending 15f, not asserted: {pending:#?}");
    assert!(
        rejected.is_empty(),
        "Native15c rejected:\n{}",
        rejected.join("\n")
    );
}

/// Native15c admission of the generated merges (`merge_generated`): every
/// case the model admits must be admitted on Tine's actual output. Three
/// groups run and are reported, not asserted (RECEIPT-cm-b4-gen): field
/// endings rounds 15c and 15f disagree on (pending 15f); memberless-only
/// declarations (model question Q2); and a source `title::` in a first-root
/// header, whose shadow this harness does not rebind (follow-up F1). Fused
/// endings (Q4), Org head-drawer refusals (Q5, Q6, Q7 of RECEIPT-cm-b4-fix4)
/// and lsdoc's debug span tripwire are reported the same way.
fn native_generated(seed: u64, count: usize) {
    let Ok(bridge) = std::env::var("CM_NATIVE_BRIDGE") else {
        assert!(std::env::var_os("CM_REQUIRE_BRIDGE").is_none());
        return;
    };
    let mut failures = BTreeMap::<String, (usize, String)>::new();
    let mut admitted = 0;
    let mut reported = BTreeMap::<&str, (usize, usize)>::new();
    let mut cases = crate::merge_generated::witnesses();
    cases.extend(crate::merge_generated::cases(seed, count));
    let total = cases.len();
    for case in cases {
        let ext = if case.org { "org" } else { "md" };
        let source_path = format!("pages/src.{ext}");
        let target_path = format!("pages/dst.{ext}");
        let mut spec = GraphSpec::default();
        spec.name = "generated-merge".into();
        spec.files = BTreeMap::from([
            (source_path.clone(), case.src_text()),
            (target_path.clone(), case.dst_text()),
        ]);
        let dir = tempfile::tempdir().unwrap();
        spec.write(dir.path());
        let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
        let choice = Choice::Merge(target_path.clone());
        let group = if crate::merge_generated::expected(&case).pending_15f() {
            Some("pending 15f")
        } else if crate::merge_generated::memberless_only(&case) {
            Some("Q2 memberless only")
        } else if case
            .src
            .iter()
            .any(|l| l.header && !l.prefix.is_empty() && l.content.starts_with("title::"))
        {
            Some("F1 first-root source title")
        } else {
            None
        };
        let verdict = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let proof = merge_proof(&store, &spec, "src", "dst", &choice)
                .ok_or("no merge proof")?
                .2
                .map_err(|e| format!("refused: {e}"))?;
            execute(&store, "src", "dst", Some(&choice)).map_err(|e| format!("refused: {e}"))?;
            let files = read_graph(dir.path());
            let p = packet(
                &store,
                &spec,
                "src",
                "dst",
                &source_path,
                &target_path,
                &proof,
                &files,
                json!([]),
            );
            let result = shared::run_bridge(Path::new(&bridge), &p)?;
            let failed: Vec<_> = result["checks"]
                .as_object()
                .unwrap()
                .iter()
                .filter(|(_, v)| **v != true)
                .map(|(k, _)| k.clone())
                .collect();
            if result["accepted"] == true && failed.is_empty() {
                Ok(())
            } else {
                Err(format!("rejected: {}", failed.join(",")))
            }
        }))
        .unwrap_or_else(|p| {
            let message = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            Err(format!("panic: {message}"))
        });
        let refused = |reason: &str| matches!(&verdict, Err(e) if e.ends_with(reason));
        let group = group.or_else(|| {
            if refused("FusedLineEnding") {
                Some("Q4 fused ending")
            } else if refused("HiddenOccurrence") && crate::merge_generated::drawer_leftover(&case) {
                Some("Q6 drawer leftover")
            } else if refused("AliasField") && crate::merge_generated::retained_in_drawer(&case) {
                Some("Q5 retained drawer alias")
            } else if (refused("HiddenOccurrence") || refused("AddedOccurrence"))
                && crate::merge_generated::leftover_delimiters(&case)
            {
                Some("Q7 leftover drawer delimiters")
            } else if matches!(&verdict, Err(e) if e.contains(crate::merge_generated::LSDOC_TRIPWIRE))
            {
                Some("lsdoc tripwire")
            } else {
                None
            }
        });
        if let Some(group) = group {
            let entry = reported.entry(group).or_default();
            entry.0 += 1;
            entry.1 += usize::from(verdict.is_ok());
            continue;
        }
        match verdict {
            Ok(()) => admitted += 1,
            Err(class) => {
                let witness = format!(
                    "org={} source={:?} target={:?}",
                    case.org,
                    case.src_text(),
                    case.dst_text()
                );
                let entry = failures.entry(class).or_insert((0, witness.clone()));
                entry.0 += 1;
                if witness.len() < entry.1.len() {
                    entry.1 = witness;
                }
            }
        }
    }
    eprintln!(
        "native generated merges: {total} cases, {admitted} admitted; reported only (run, admitted): {reported:?}"
    );
    let listing: String = failures
        .iter()
        .map(|(class, (n, w))| format!("  {class}: {n}\n    {w}\n"))
        .collect();
    assert!(failures.is_empty(), "native failing classes:\n{listing}");
}

#[test]
fn native_generated_merges_are_admitted() {
    let _guard = crate::integration_lock();
    native_generated(crate::merge_generated::SEED, 32);
}

#[test]
#[ignore]
fn native_generated_merges_are_admitted_large() {
    let _guard = crate::integration_lock();
    let n = std::env::var("CM_GEN_COUNT").map_or(400, |v| v.parse().unwrap());
    native_generated(crate::merge_generated::SEED ^ 0xBEEF, n);
}

#[test]
fn native_split_query_and_guarded_title_controls() {
    let _guard = crate::integration_lock();
    let Ok(bridge) = std::env::var("CM_NATIVE_BRIDGE") else {
        assert!(std::env::var_os("CM_REQUIRE_BRIDGE").is_none());
        return;
    };
    let bridge = Path::new(&bridge);
    for destination in ["dst", "two words"] {
        let target_path = format!("pages/{destination}.md");
        for (source, target) in [
            ("title:: src\n", ""),
            (
                "alias:: X\nsame:: {{query [[A]]}}\n- body {{query [[A]]}}\n",
                "same:: kept\n- target\n",
            ),
            ("- source\n", "- alias:: Kept\n"),
            ("- source\n", "- title:: other\n"),
            ("alias:: #src\n- source\n", "alias:: Kept\n- target\n"),
            ("alias:: #src, X\n- source\n", "alias:: dst\n- target\n"),
            ("alias:: #[[src]]\n- source\n", "alias:: #Kept\n- target\n"),
            (
                "alias:: #src, #[[X]]\n- source\n",
                "alias:: Kept\n- target\n",
            ),
            ("alias:: X\n{{query [[A]]}}\n- source\n", "- target\n"),
            (
                "title:: src\n{{query [[A]]}}\n- source\n",
                "alias:: Kept\ntarget prose\n- target\n",
            ),
            (
                "source prose\nsecond line\n- source\n",
                "alias:: Kept\n- target\n",
            ),
            (
                "alias:: X\n- source\n",
                "target prose\nsecond line\n- target\n",
            ),
            (
                "title:: src\r\n\r\nsource prose\r\n- source\r\n",
                "alias:: Kept\r\ntarget prose\r\n- target\r\n",
            ),
        ] {
            let mut spec = GraphSpec::default();
            spec.name = if source.contains("query") {
                "split-query"
            } else if target.contains("alias") {
                "first-root-alias"
            } else if target.contains("title") {
                "first-root-title"
            } else {
                "guarded"
            }
            .into();
            spec.files = BTreeMap::from([
                ("pages/src.md".into(), source.into()),
                (target_path.clone(), target.into()),
            ]);
            let dir = tempfile::tempdir().unwrap();
            spec.write(dir.path());
            let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
            let proof = merge_proof(
                &store,
                &spec,
                "src",
                destination,
                &Choice::Merge(target_path.clone()),
            )
            .unwrap()
            .2
            .unwrap();
            execute(
                &store,
                "src",
                destination,
                Some(&Choice::Merge(target_path.clone())),
            )
            .unwrap();
            let files = read_graph(dir.path());
            let p = packet(
                &store,
                &spec,
                "src",
                destination,
                "pages/src.md",
                &target_path,
                &proof,
                &files,
                json!([]),
            );
            let admitted = admit(bridge, &p);
            let mut lost_codec = p.clone();
            lost_codec["saves"][0]["serialized"] = json!("- lost guard\n");
            lost_codec["saves"][0]["reparsed"] =
                native::document(&shared::parse_file(&target_path, "- lost guard\n"));
            assert_eq!(
                shared::run_bridge(bridge, &lost_codec).unwrap()["checks"]["unchangedSaveClosure"],
                false
            );
            if source.contains("query") {
                if source.contains("same::") {
                    assert_eq!(
                        admitted["immediateHistoricalDiagnostic"], false,
                        "split header queries require the indexed native law"
                    );
                    assert_eq!(p["queryImages"].as_array().unwrap().len(), 2);
                } else {
                    assert_eq!(p["queryImages"].as_array().unwrap().len(), 1);
                }
                let mut missing = p.clone();
                missing["queryImages"].as_array_mut().unwrap().pop();
                assert_eq!(
                    shared::run_bridge(bridge, &missing).unwrap()["checks"]["queryImages"],
                    false
                );
                let mut duplicate = p.clone();
                let image = duplicate["queryImages"][0].clone();
                duplicate["queryImages"].as_array_mut().unwrap().push(image);
                assert_eq!(
                    shared::run_bridge(bridge, &duplicate).unwrap()["checks"]["queryImages"],
                    false
                );
            } else if source == "title:: src\n" && target.is_empty() {
                let bad_dir = tempfile::tempdir().unwrap();
                let mut bad_spec = spec.clone();
                bad_spec.files = BTreeMap::from([(target_path.clone(), "- title:: src\n".into())]);
                bad_spec.write(bad_dir.path());
                let bad_store = Store::open(bad_dir.path(), Default::default()).unwrap().0;
                let mut retitling = p.clone();
                retitling["after"] = json!([file(&spec, &target_path, "- title:: src\n")]);
                retitling["placements"][0]["items"][0]["insert"] = json!("- ");
                retitling["saves"] = json!([save(&bad_store, &target_path, "- title:: src\n")]);
                assert_eq!(
                    retitling["saves"][0]["planned"], "title:: src\n",
                    "actual save reproduces retitling"
                );
                let rejected = shared::run_bridge(bridge, &retitling).unwrap();
                assert_eq!(rejected["immediateHistoricalDiagnostic"], true);
                assert_eq!(rejected["checks"]["unchangedSaveClosure"], false);
                assert_eq!(rejected["accepted"], false);
            }
            let host = crate::test_host(&store);
            let id = PageId::from(target_path.as_str());
            for _ in 0..2 {
                let loaded = store.page(&id).unwrap();
                crate::test_host_writer::save_on(
                    &host,
                    &store,
                    tine_store::EditKind::SaveBlock,
                    &id,
                    tine_store::SaveBase::Existing(loaded.rev),
                    &loaded.doc,
                )
                .unwrap();
                assert_eq!(
                    fs::read_to_string(dir.path().join(&target_path)).unwrap(),
                    files[&target_path]
                );
            }
        }
    }
}

#[test]
fn native_org_requires_exact_serialized_bytes() {
    let _guard = crate::integration_lock();
    let Ok(bridge) = std::env::var("CM_NATIVE_BRIDGE") else {
        assert!(std::env::var_os("CM_REQUIRE_BRIDGE").is_none());
        return;
    };
    for (source, target) in [
        ("#+title: src\n", ""),
        (
            "#+ALIAS: Former, Shared\n* source\n",
            "#+ALIAS: Shared, Kept\n* target\n",
        ),
        (
            "#+TITLE: src\n#+ALIAS: Former\n#+CATEGORY: work\nfree text\n* source\n",
            "#+ALIAS: Kept\n#+CATEGORY: home\n* target\n",
        ),
        (
            "#+title: src\n{{query [[A]]}}\n* source\n",
            "#+alias: Kept\ntarget prose\n* target\n",
        ),
        (
            "#+title: src\r\n{{query [[A]]}}\r\n* source\r\n",
            "#+alias: Kept\r\ntarget prose\r\n* target\r\n",
        ),
    ] {
        let mut spec = GraphSpec::default();
        spec.name = "org-guarded".into();
        spec.files = BTreeMap::from([
            ("pages/src.org".into(), source.into()),
            ("pages/dst.org".into(), target.into()),
        ]);
        let dir = tempfile::tempdir().unwrap();
        spec.write(dir.path());
        let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
        let proof = merge_proof(
            &store,
            &spec,
            "src",
            "dst",
            &Choice::Merge("pages/dst.org".into()),
        )
        .unwrap()
        .2
        .unwrap();
        execute(
            &store,
            "src",
            "dst",
            Some(&Choice::Merge("pages/dst.org".into())),
        )
        .unwrap();
        let p = packet(
            &store,
            &spec,
            "src",
            "dst",
            "pages/src.org",
            "pages/dst.org",
            &proof,
            &read_graph(dir.path()),
            json!([]),
        );
        let bridge = Path::new(&bridge);
        admit(bridge, &p);
        assert_local_saves(&p);
        let mut changed = p.clone();
        // Equal supplied documents cannot excuse different Org serialization.
        changed["saves"][0]["serialized"] = json!("* different\n");
        let rejected = shared::run_bridge(bridge, &changed).unwrap();
        assert_eq!(rejected["checks"]["unchangedSaveClosure"], false);
        assert_eq!(rejected["accepted"], false);
    }
}

#[test]
fn native_calendar_days_stay_distinct_when_display_labels_match() {
    let _guard = crate::integration_lock();
    let Ok(bridge) = std::env::var("CM_NATIVE_BRIDGE") else {
        assert!(std::env::var_os("CM_REQUIRE_BRIDGE").is_none());
        return;
    };
    let mut spec = GraphSpec::default();
    spec.name = "ambiguous-day-labels".into();
    spec.journal_title_format = Some("MM-dd".into());
    spec.files = BTreeMap::from([
        ("pages/src.md".into(), "- source\n".into()),
        ("pages/dst.md".into(), "- target\n".into()),
        ("journals/2026_10_26.md".into(), "- day 2026\n".into()),
        ("journals/2027_10_26.md".into(), "- day 2027\n".into()),
    ]);
    let dir = tempfile::tempdir().unwrap();
    spec.write(dir.path());
    let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    let proof = merge_proof(
        &store,
        &spec,
        "src",
        "dst",
        &Choice::Merge("pages/dst.md".into()),
    )
    .unwrap()
    .2
    .unwrap();
    execute(
        &store,
        "src",
        "dst",
        Some(&Choice::Merge("pages/dst.md".into())),
    )
    .unwrap();
    let p = packet(
        &store,
        &spec,
        "src",
        "dst",
        "pages/src.md",
        "pages/dst.md",
        &proof,
        &read_graph(dir.path()),
        json!([]),
    );
    let journals: Vec<_> = p["before"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["dir"] == "journals")
        .collect();
    assert_eq!(journals[0]["stem"], journals[1]["stem"]);
    assert_ne!(
        p["journalDays"]["journals/2026_10_26.md"],
        p["journalDays"]["journals/2027_10_26.md"]
    );
    admit(Path::new(&bridge), &p);
}
