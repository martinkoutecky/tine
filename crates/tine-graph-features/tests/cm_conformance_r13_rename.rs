//! Candidate-oriented r13 adapter. The current product API is deliberately
//! exercised as-is; missing candidates become attributed diagnostic rows.
use super::*;
use tine_core::model::PageId;
use tine_graph_features::pages::merge_payload::{self, MergeProposal, Origin};
#[path = "cm_conformance_native_rename.rs"]
mod native;

fn merge_proof(
    store: &Store,
    spec: &GraphSpec,
    old: &str,
    _requested: &str,
    choice: &Choice,
) -> Option<(
    String,
    String,
    Result<MergeProposal, merge_payload::RealizationError>,
)> {
    let Choice::Merge(path) = choice else {
        return None;
    };
    let view = store.whole_graph().unwrap();
    let Resolved::Existing { id: source, .. } = view.resolve(old, false) else {
        return None;
    };
    let target = PageId::from(path.as_str());
    let src = &spec.files[source.as_str()];
    let dst = &spec.files[path];
    let a = store.page_of(&source, src.as_bytes()).unwrap().doc;
    let b = store.page_of(&target, dst.as_bytes()).unwrap().doc;
    // Independent reader spelling, rather than the planner's map or the
    // typed alias. Ambiguous display labels cannot identify a journal day.
    let mut selected = b.name.clone();
    if path.starts_with("journals/") {
        if let Some(day) = spec
            .journal_format()
            .parse(Path::new(path).file_stem()?.to_str()?)
        {
            if spec.journal_format().parse(&selected) != Some(day) {
                selected = day.file_stem();
            }
        }
    }
    let links = std::collections::HashMap::from([(tine_core::refs::page_key(old), selected)]);
    Some((
        path.clone(),
        source.as_str().to_owned(),
        merge_payload::propose(
            &b,
            dst,
            &a,
            src,
            &links,
            &links,
            path.ends_with(".org"),
            spec.name_format(),
        ),
    ))
}

#[derive(Clone)]
enum Choice {
    Rename,
    Own,
    Merge(String),
}

fn options(store: &Store, old: &str, new: &str) -> Vec<(Value, Choice)> {
    let view = store.whole_graph().unwrap();
    let file_source = matches!(view.resolve(old, false), Resolved::Existing { .. });
    let identity = |path: &str| {
        let entries = view.inventory(tine_store::InventoryScope::File(&PageId::from(path)));
        let name = &entries.0.first().unwrap().name;
        json!({"destination": name, "survivor": if file_source { json!(path) } else { Value::Null }})
    };
    let Ok(offer) = rename_offer(store, old, new, None) else {
        return Vec::new();
    };
    match offer {
        RenameOffer::Rename => vec![(
            json!({"destination": new, "survivor": null}),
            Choice::Rename,
        )],
        RenameOffer::Merge { into, .. } => vec![(identity(&into), Choice::Merge(into))],
        RenameOffer::AliasOf {
            owner,
            merge,
            own_page,
            ..
        } => {
            let mut out = Vec::new();
            if merge {
                out.push((identity(&owner), Choice::Merge(owner)));
            }
            if own_page {
                out.push((json!({"destination": new, "survivor": null}), Choice::Own));
            }
            out
        }
        RenameOffer::Refused { .. } => Vec::new(),
    }
}

fn execute(
    store: &Arc<Store>,
    old: &str,
    new: &str,
    choice: Option<&Choice>,
) -> Result<RenameOutcome, String> {
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
    let c = match choice {
        Some(Choice::Rename) | None => RenameChoice::Rename,
        Some(Choice::Own) => RenameChoice::OwnPage,
        Some(Choice::Merge(path)) => RenameChoice::MergeInto(path),
    };
    let result = rename_page_as(store, &host, old, new, None, c, &[])
        .map(|r| r.outcome)
        .map_err(|e| e.to_string());
    drop(host);
    result
}

fn projection(file: &Value) -> Value {
    json!({"name": file["title"].as_str().filter(|_| file["dir"] == "pages")
        .unwrap_or_else(|| file["stem"].as_str().unwrap()), "aliases": file["aliases"],
        "blocks": file["blocks"].as_array().unwrap().iter().map(|b| json!({
            "level": b["level"], "occ": b["occ"], "stored": b["stored"]
        })).collect::<Vec<_>>()})
}

fn normalized(mut p: Value) -> Value {
    // Property-key extraction returns canonical keys, even when the writer
    // was given a mixed-case spelling. Compare that parser boundary by key.
    for b in p["blocks"].as_array_mut().unwrap() {
        for o in b["occ"].as_array_mut().unwrap() {
            if o[0] == "propertyKey" {
                o[1] = json!(tine_core::refs::page_key(o[1].as_str().unwrap()));
            }
        }
    }
    p
}

fn legacy_projection(
    p: &Value,
    path: &str,
    candidate: &Value,
    before: &Value,
    old: &str,
    requested: &str,
) -> Option<Value> {
    let destination = candidate["candidate"]["destination"].as_str().unwrap();
    let map =
        std::collections::HashMap::from([(tine_core::refs::page_key(old), requested.to_owned())]);
    fn rewrite(
        v: &mut Value,
        map: &std::collections::HashMap<String, String>,
        root: Option<(&str, &str)>,
    ) {
        match v {
            Value::String(s) => {
                if let Some((old, destination)) = root.filter(|(old, _)| {
                    tine_core::refs::page_key(old) == tine_core::refs::page_key(s)
                }) {
                    let _ = old;
                    *s = destination.to_owned();
                } else if let Some(to) = tine_core::refs::renamed_page_name(s, map) {
                    *s = to.into_owned();
                }
            }
            Value::Array(vs) => vs.iter_mut().for_each(|v| rewrite(v, map, root)),
            Value::Object(vs) => vs.values_mut().for_each(|v| rewrite(v, map, root)),
            _ => {}
        }
    }
    let mut p = p.clone();
    let mut origins = BTreeSet::new();
    for (i, b) in p["blocks"].as_array_mut().unwrap().iter_mut().enumerate() {
        let image = candidate["images"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["after"].as_array().unwrap().contains(&json!([path, i])))?;
        let source = image["before"][0].as_str().unwrap();
        origins.insert(source);
        let f = before["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"] == source)?;
        let original = &f["blocks"][image["before"][1].as_u64().unwrap() as usize];
        b["occ"] = original["occ"].clone();
        b["stored"] = original["stored"].clone();
        for o in b["occ"].as_array_mut().unwrap() {
            if o[0] != "taskMarker" && o[0] != "priority" {
                rewrite(&mut o[1], &map, None);
            }
        }
        rewrite(&mut b["stored"], &map, Some((old, destination)));
    }
    p["aliases"] = json!(before["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| origins.contains(f["path"].as_str().unwrap()))
        .flat_map(|f| f["aliases"].as_array().unwrap().clone())
        .collect::<Vec<_>>());
    rewrite(&mut p["aliases"], &map, None);
    let aliases = p["aliases"].as_array_mut().unwrap();
    aliases.sort_by_key(Value::to_string);
    aliases.dedup_by(|a, b| {
        tine_core::refs::page_key(a.as_str().unwrap())
            == tine_core::refs::page_key(b.as_str().unwrap())
    });
    Some(normalized(p))
}

fn occurrence_bag(p: &Value) -> BTreeMap<String, usize> {
    let mut bag = BTreeMap::new();
    for b in p["blocks"].as_array().unwrap() {
        for o in b["occ"]
            .as_array()
            .unwrap()
            .iter()
            .chain(b["stored"].as_array().unwrap())
        {
            *bag.entry(o.to_string()).or_default() += 1;
        }
    }
    bag
}

fn equal_header_loss(
    predicted: &Value,
    observed: &Value,
    spec: &GraphSpec,
    files: &BTreeMap<String, String>,
    contract: &Value,
) -> bool {
    let mut bag = occurrence_bag(predicted);
    for fragment in contract["protectedText"].as_array().into_iter().flatten() {
        let text = fragment.as_str().unwrap();
        let Some((key, _)) = text.split_once("::") else {
            continue;
        };
        let before: usize = spec.files.values().map(|s| s.matches(text).count()).sum();
        let after: usize = files.values().map(|s| s.matches(text).count()).sum();
        if before > after {
            let occurrence = json!(["propertyKey", tine_core::refs::page_key(key)]).to_string();
            let Some(count) = bag.get_mut(&occurrence) else {
                return false;
            };
            let Some(remaining) = count.checked_sub(before - after) else {
                return false;
            };
            *count = remaining;
        }
    }
    bag.retain(|_, count| *count > 0);
    bag == occurrence_bag(observed)
}

fn repair_projection(input: &mut Value, path: &str, p: &Value) {
    let f = input["files"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|f| f["path"] == path)
        .unwrap();
    f["aliases"] = p["aliases"].clone();
    f["title"] = p["name"].clone();
    f["blocks"] = p["blocks"].clone();
    for (i, b) in f["blocks"].as_array_mut().unwrap().iter_mut().enumerate() {
        b["text"] = json!(format!("cause-repair:{path}:{i}"));
    }
}

fn compare_outputs(
    bridge: &Path,
    spec: &GraphSpec,
    old: &str,
    requested: &str,
    contract: &Value,
    root: &Path,
    candidate: &Value,
    original_input: &Value,
    writer: Option<&(
        String,
        String,
        Result<MergeProposal, merge_payload::RealizationError>,
    )>,
    diffs: &mut Vec<String>,
) -> Value {
    if let (Ok(native_bridge), Some((target, source, Ok(proof)))) =
        (std::env::var("CM_NATIVE_BRIDGE"), writer)
    {
        let mut historical = Vec::new();
        let mut evidence = compare_historical_outputs(
            bridge,
            spec,
            old,
            requested,
            contract,
            root,
            candidate,
            original_input,
            writer,
            &mut historical,
        );
        let files = read_graph(root);
        let store = Store::open(root, Default::default()).unwrap().0;
        let chosen = candidate["candidate"]["destination"].as_str().unwrap();
        let packet = native::packet(
            &store,
            spec,
            old,
            chosen,
            source,
            target,
            proof,
            &files,
            original_input["queries"].clone(),
        );
        let result = shared::run_bridge(Path::new(&native_bridge), &packet).unwrap();
        if result["checks"]["selection"] == false && spec.journal_format().parse(old).is_some() {
            assert_eq!(
                result["accepted"], false,
                "fixed-day sources have no ordinary native merge certificate"
            );
            diffs.push(format!("G4/B3 fixed-day source: {old:?} was executed as an ordinary merge; Native15c rejects source selection (day-content moves are separate)"));
            evidence["historicalR13Diagnostics"] = json!(historical);
            evidence["nativePacket"] = packet;
            evidence["nativeAdmission"] = result;
            return evidence;
        }
        native::admit(Path::new(&native_bridge), &packet);
        if chosen != requested {
            let corrected_dir = tempfile::tempdir().unwrap();
            spec.write(corrected_dir.path());
            let corrected_store = Arc::new(
                Store::open(corrected_dir.path(), Default::default())
                    .unwrap()
                    .0,
            );
            let corrected_proof = merge_proof(
                &corrected_store,
                spec,
                old,
                chosen,
                &Choice::Merge(target.clone()),
            )
            .unwrap()
            .2
            .unwrap();
            execute(
                &corrected_store,
                old,
                chosen,
                Some(&Choice::Merge(target.clone())),
            )
            .unwrap();
            let corrected_files = read_graph(corrected_dir.path());
            assert_eq!(
                files, corrected_files,
                "typed and selected requests must produce the same graph"
            );
            let corrected = native::packet(
                &corrected_store,
                spec,
                old,
                chosen,
                source,
                target,
                &corrected_proof,
                &corrected_files,
                original_input["queries"].clone(),
            );
            evidence["chosenControl"] = native::admit(Path::new(&native_bridge), &corrected);
        }
        // Favorites are outside native admission; keep their existing checks.
        diffs.extend(
            historical
                .iter()
                .filter(|row| row.starts_with("G10/B4a"))
                .cloned(),
        );
        evidence["historicalR13Diagnostics"] = json!(historical);
        evidence["nativePacket"] = packet;
        evidence["nativeAdmission"] = result;
        return evidence;
    }
    compare_historical_outputs(
        bridge,
        spec,
        old,
        requested,
        contract,
        root,
        candidate,
        original_input,
        writer,
        diffs,
    )
}

fn compare_historical_outputs(
    bridge: &Path,
    spec: &GraphSpec,
    old: &str,
    requested: &str,
    contract: &Value,
    root: &Path,
    candidate: &Value,
    original_input: &Value,
    writer: Option<&(
        String,
        String,
        Result<MergeProposal, merge_payload::RealizationError>,
    )>,
    diffs: &mut Vec<String>,
) -> Value {
    let files = read_graph(root);
    if let Some((path, _, proof)) = writer {
        let proof = proof
            .as_ref()
            .expect("an applied merge has a realizable writer proposal");
        proof
            .trace
            .check(&files[path], &proof.trace.substitution)
            .expect("production TraceValid, against disk bytes");
    }
    let links = std::collections::HashMap::from([(
        tine_core::refs::page_key(old),
        candidate["candidate"]["destination"]
            .as_str()
            .unwrap()
            .to_owned(),
    )]);
    let mut referrer_traces = 0;
    for (path, after) in &files {
        if writer.is_some_and(|(survivor, _, _)| survivor == path) {
            continue;
        }
        let Some(before) = spec.files.get(path).filter(|before| *before != after) else {
            continue;
        };
        let org = path.ends_with(".org");
        let mut proposal =
            tine_core::refs::rename_proposal(before, org, &links, &links, spec.name_format())
                .unwrap();
        if proposal.text != *after {
            let format = if org {
                tine_core::model::Format::Org
            } else {
                tine_core::model::Format::Md
            };
            let name = tine_core::model::decode_page_name(
                Path::new(path).file_stem().unwrap().to_str().unwrap(),
                spec.name_format(),
            );
            if let Some(title) = tine_core::model::page_title_from_preamble(before, format) {
                if let Some(new) = tine_core::refs::renamed_page_name(&title, &links)
                    .filter(|new| tine_core::refs::same_page(new, &name))
                {
                    proposal.rebind_title(org, &new).unwrap();
                }
            }
        }
        proposal
            .trace
            .check(after, &proposal.trace.substitution)
            .expect("production referrer certificate against disk bytes");
        referrer_traces += 1;
    }
    for fragment in contract["protectedText"].as_array().into_iter().flatten() {
        let text = fragment.as_str().unwrap();
        let before: usize = spec.files.values().map(|s| s.matches(text).count()).sum();
        let after: usize = files.values().map(|s| s.matches(text).count()).sum();
        if before != after {
            diffs.push(format!(
                "G7/B3a: protected fragment {text:?} multiplicity: before {before}, after {after}"
            ));
        }
    }
    let actual_config = tine_core::config::Config::parse(
        &fs::read_to_string(root.join("logseq/config.edn")).unwrap_or_default(),
    );
    let mut after_spec = spec.clone();
    after_spec.files = files.clone();
    after_spec.favorites_page = actual_config.favorites_page.clone();
    after_spec.renames.clear();
    after_spec.navigate = candidate["meaning"]["navigate"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    let store = Store::open(root, Default::default()).unwrap().0;
    let view = store.whole_graph().unwrap();
    let mut input = shared::model_input_r13(&after_spec);
    if let Some((path, source_path, Ok(proof))) = writer {
        let file = input["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"] == *path)
            .unwrap();
        assert_eq!(
            proof.origins.len(),
            file["blocks"].as_array().unwrap().len(),
            "production origins cover every parsed output block"
        );
        assert!(proof.origins.iter().all(|origins| !origins.is_empty()));
        let before = original_input["files"].as_array().unwrap();
        let target = before.iter().find(|f| f["path"] == *path).unwrap();
        // Byte-identical source files of the same format have the same block count. The
        // trace binds the actual merge input; no model-side image is used.
        let source = before.iter().find(|f| f["path"] == *source_path).unwrap();
        let org = path.ends_with(".org");
        let parsed = if org {
            tine_core::org::parse_org(&proof.trace.inputs[0])
        } else {
            tine_core::doc::parse(&proof.trace.inputs[0])
        };
        let header = parsed.pre_block.as_deref().map_or_else(
            || {
                usize::from(
                    tine_core::page_header::page_header_text(
                        None,
                        parsed.roots.first().map(|b| b.raw()),
                    )
                    .is_some(),
                )
            },
            |pre| tine_core::page_properties::pre_block_projections(pre, org).len(),
        );
        let mut target = vec![false; target["blocks"].as_array().unwrap().len()];
        let mut source = vec![false; source["blocks"].as_array().unwrap().len()];
        for origins in &proof.origins {
            for origin in origins {
                match origin {
                    Origin::TargetHeader => {
                        assert!(header > 0);
                        for slot in &mut target[..header] {
                            *slot = true;
                        }
                    }
                    Origin::Target(j) => {
                        target[*j] = true;
                    }
                    Origin::Source(j) => {
                        source[*j] = true;
                    }
                    Origin::Leftover => {}
                }
            }
        }
        assert!(
            target.into_iter().chain(source).all(|image| image),
            "every input block has a production origin image"
        );
    }
    input["favorites"] = json!(actual_config.favorites);
    let actual: BTreeMap<_, _> = input["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["path"].as_str().unwrap(), f))
        .collect();
    let expected: BTreeMap<_, _> = candidate["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["path"].as_str().unwrap(), f))
        .collect();
    // Each counterfactual changes only an observed, independently diagnosed
    // product difference. A semantic row inherits a gap only if that repair
    // makes the entire compared result agree. Otherwise it remains FINDING.
    let mut repairs = BTreeMap::<&str, Value>::new();
    if actual.keys().collect::<Vec<_>>() != expected.keys().collect::<Vec<_>>() {
        let destination = candidate["candidate"]["destination"].as_str().unwrap();
        let gap = if tine_core::refs::page_key(old) == tine_core::refs::page_key(destination) {
            "G6/B3"
        } else if destination.starts_with(&format!("{old}/")) {
            "G5/B3b"
        } else if candidate["candidate"]["destination"] != requested {
            "G3/B4"
        } else {
            "FINDING"
        };
        diffs.push(format!(
            "{gap}: physical output paths: model {:?}, Tine {:?}",
            expected.keys(),
            actual.keys()
        ));
        if gap == "G6/B3" {
            let mut fixed = input.clone();
            for f in fixed["files"].as_array_mut().unwrap() {
                if !expected.contains_key(f["path"].as_str().unwrap()) {
                    if let Some(path) = expected.keys().find(|path| {
                        !actual.contains_key(**path)
                            && tine_core::refs::page_key(
                                expected[**path]["projection"]["name"].as_str().unwrap(),
                            ) == tine_core::refs::page_key(
                                projection(f)["name"].as_str().unwrap(),
                            )
                    }) {
                        f["path"] = json!(path);
                    }
                }
            }
            let repaired_paths: BTreeSet<_> = fixed["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f["path"].as_str().unwrap())
                .collect();
            if repaired_paths == expected.keys().copied().collect() {
                repairs.insert(gap, fixed);
            } else {
                *diffs.last_mut().unwrap() = diffs.last().unwrap().replacen(gap, "FINDING", 1);
            }
        }
        if gap == "G3/B4" {
            let mut fixed = input.clone();
            for movement in candidate["moves"].as_array().unwrap() {
                let source = original_input["files"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|f| f["path"] == movement["source"])
                    .unwrap();
                let name = source["title"]
                    .as_str()
                    .unwrap_or_else(|| source["stem"].as_str().unwrap());
                let map = std::collections::HashMap::from([(
                    tine_core::refs::page_key(old),
                    requested.to_owned(),
                )]);
                if let Some(legacy) = tine_core::refs::renamed_page_name(name, &map) {
                    let path = movement["path"].as_str().unwrap();
                    let ext = Path::new(path).extension().unwrap().to_str().unwrap();
                    let legacy_path = format!(
                        "pages/{}.{ext}",
                        tine_core::model::encode_page_name(&legacy, spec.name_format())
                    );
                    if legacy_path != path {
                        if let Some(f) = fixed["files"]
                            .as_array_mut()
                            .unwrap()
                            .iter_mut()
                            .find(|f| f["path"] == legacy_path)
                        {
                            f["path"] = json!(path);
                            f["title"] = expected[path]["projection"]["name"].clone();
                        }
                    }
                }
            }
            let repaired_paths: BTreeSet<_> = fixed["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f["path"].as_str().unwrap())
                .collect();
            if repaired_paths == expected.keys().copied().collect() {
                repairs.insert(gap, fixed);
            } else {
                *diffs.last_mut().unwrap() = diffs.last().unwrap().replacen(gap, "FINDING", 1);
            }
        }
    }
    for (path, file) in &expected {
        match actual.get(path) {
            None => {}
            Some(actual) => {
                if let Some(bytes) = file["text"].as_str() {
                    if files[*path] != bytes {
                        diffs.push(format!("G7/B3a: unchanged physical bytes differ at {path}"));
                    }
                }
                let predicted = normalized(file["projection"].clone());
                let observed = normalized(projection(actual));
                if predicted != observed {
                    let destination = candidate["candidate"]["destination"].as_str().unwrap();
                    let legacy = legacy_projection(
                        &predicted,
                        path,
                        candidate,
                        original_input,
                        old,
                        requested,
                    );
                    let split = destination != requested
                        && legacy.as_ref().is_some_and(|legacy| *legacy == observed);
                    let split_layout = destination != requested
                        && legacy.as_ref().is_some_and(|legacy| {
                            legacy["name"] == observed["name"]
                                && legacy["aliases"] == observed["aliases"]
                                && occurrence_bag(legacy) == occurrence_bag(&observed)
                        });
                    let layout = candidate["candidate"]["survivor"] == *path
                        && predicted["name"] == observed["name"]
                        && predicted["aliases"] == observed["aliases"]
                        && (occurrence_bag(&predicted) == occurrence_bag(&observed)
                            || equal_header_loss(&predicted, &observed, spec, &files, contract));
                    let gap = if split {
                        "G3/B4"
                    } else if split_layout {
                        "G3/B4+G7/B3a"
                    } else if layout {
                        "G7/B3a"
                    } else {
                        "FINDING"
                    };
                    diffs.push(format!(
                        "{gap}: ordered file projection {path}: model {}, Tine {}",
                        file["projection"],
                        projection(actual)
                    ));
                    if gap != "FINDING" {
                        repair_projection(
                            repairs.entry(gap).or_insert_with(|| input.clone()),
                            path,
                            &predicted,
                        );
                    }
                }
                for key in ["dir", "format"] {
                    assert_eq!(file[key], actual[key], "physical format/kind preserved");
                }
            }
        }
    }
    if let Some(survivor) = candidate["candidate"]["survivor"].as_str() {
        if !files.contains_key(survivor) {
            diffs.push(format!(
                "FINDING: survivor identity {survivor} was not retained"
            ));
        }
    }
    if json!(actual_config.favorites) != candidate["favorites"] {
        diffs.push(format!(
            "G10/B4a: favorites: model {}, Tine {:?}",
            candidate["favorites"], actual_config.favorites
        ));
    }
    if actual_config.favorites_page != spec.config().favorites_page {
        diffs.push(format!(
            "G10/B4a: Favorites exclusion setting changed to {:?}",
            actual_config.favorites_page
        ));
        let mut fixed = input.clone();
        fixed["favoritesPage"] = json!(spec
            .favorites_page
            .clone()
            .unwrap_or_else(|| shared::NO_FAVORITES.to_owned()));
        repairs.insert("G10/B4a", fixed);
    }
    let model_after =
        shared::run_bridge(bridge, &input).expect("actual writer output is parsed by the bridge");
    assert_eq!(model_after["round"], "r13");
    assert!(shared::harness_reports(&model_after).is_empty());
    let as_set = |v: &Value| -> BTreeSet<String> {
        v.as_array().unwrap().iter().map(Value::to_string).collect()
    };
    let expected_meaning = &candidate["meaning"];
    let repaired: Vec<_> = repairs
        .into_iter()
        .map(|(gap, input)| {
            (
                gap,
                shared::run_bridge(bridge, &input).expect("causal repair bridge executes"),
            )
        })
        .collect();
    for field in ["pages", "links", "typedAliasEdges"] {
        if as_set(&model_after[field]) != as_set(&expected_meaning[field]) {
            let gap = repaired
                .iter()
                .find(|(_, meaning)| as_set(&meaning[field]) == as_set(&expected_meaning[field]))
                .map(|(gap, _)| *gap)
                .unwrap_or("FINDING");
            diffs.push(format!(
                "{gap}: computed after-meaning {field}: model {}, actual writer {}",
                expected_meaning[field], model_after[field]
            ));
        }
    }
    for (i, rows) in expected_meaning["queries"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        if as_set(rows) != as_set(&model_after["queries"][i]) {
            let gap = repaired
                .iter()
                .find(|(_, meaning)| as_set(&meaning["queries"][i]) == as_set(rows))
                .map(|(gap, _)| *gap)
                .unwrap_or("FINDING");
            diffs.push(format!(
                "{gap}: computed after-query {i}: model {rows}, actual writer {}",
                model_after["queries"][i]
            ));
        }
    }
    // Whole bytes are retained as evidence. Symbolic predictions cannot certify
    // protected text/layout or true writer origins (G7), even when projections agree.
    let writer=writer.map(|(path,source,proof)| match proof {
        Ok(p)=>json!({"path":path,"source":source,"pieces":p.trace.pieces.len(),"consumed":p.trace.consumed,"origins":format!("{:?}",p.origins),"unrealized":format!("{:?}",p.unrealized),"traceValid":true}),
        Err(e)=>json!({"path":path,"unrealizable":format!("{e:?}")}),
    });
    let evidence = json!({"files": files, "parsed": input["files"], "meaning": model_after, "writer":writer,"referrerTraces":referrer_traces,
        "causalRepairs": repaired});
    drop(view);
    store.close();
    evidence
}

pub fn run(bridge: &Path) {
    assert!(
        std::env::var_os("CM_NATIVE_BRIDGE").is_some(),
        "r13 describes offers; native merge conformance requires CM_NATIVE_BRIDGE (accepted 15c)"
    );
    let manifest = env!("CARGO_MANIFEST_DIR");
    let mut section = Section::default();
    let mut report = BTreeMap::new();
    let mut candidates_run = 0;
    let n = std::env::var("CM_RANDOM_RENAMES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(RANDOM_DEFAULT);
    let random = (0..n as u64).map(|seed| {
        let mut spec = shared::random_graph(seed);
        spec.renames = vec![shared::random_rename(seed, &spec)];
        (spec, Value::Null)
    });
    let specs: Vec<_> = rename_fixtures()
        .into_iter()
        .map(|s| (s, Value::Null))
        .chain(shared::r13::fixtures(manifest))
        .chain(random)
        .collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..WORKERS {
            scope.spawn(|| loop {
        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let Some((spec, expected)) = specs.get(i) else { break };
        let mut section = Section::default();
        let mut report = BTreeMap::new();
        let mut candidates_run = 0;
        let seed = tempfile::tempdir().unwrap();
        spec.write(seed.path());
        let initial_config = fs::read(seed.path().join("logseq/config.edn")).ok();
        let initial = Arc::new(Store::open(seed.path(), Default::default()).unwrap().0);
        let view = initial.whole_graph().unwrap();
        let input = shared::model_input_r13(&spec);
        let model = shared::run_bridge(bridge, &input).unwrap();
        shared::r13::assert_contract(spec, &model, expected);
        drop(view);
        for operation in model["renames"].as_array().unwrap() {
            let old = operation["old"].as_str().unwrap();
            let new = operation["new"].as_str().unwrap();
            let offered = options(&initial, old, new);
            let actual_ids: Vec<_> = offered.iter().map(|(id, _)| id.clone()).collect();
            let mut diffs = Vec::new();
            if json!(actual_ids) != operation["offers"] {
                diffs.push(format!("G3/B4: offered identities: model {}, Tine {}", operation["offers"], json!(actual_ids)));
            }
            for candidate in operation["candidates"].as_array().unwrap() {
                candidates_run += 1;
                let id = &candidate["candidate"];
                let label = format!("{}: {old} -> {new}: {id}", spec.name);
                let mut rows = Vec::new();
                let evidence = if let Some((_, choice)) = offered.iter().find(|(actual, _)| actual == id) {
                    let dir = tempfile::tempdir().unwrap();
                    spec.write(dir.path());
                    let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
                    let before = read_graph(dir.path());
                    let writer=merge_proof(&store,spec,old,new,choice);
                    let result = execute(&store, old, new, Some(choice));
                    let allowed = result.is_ok();
                    if allowed != (candidate["safe"] == true) {
                        rows.push(format!("G4/B3: admission: model safe={}, Tine result={result:?}; checks {}", candidate["safe"], candidate["checks"]));
                    }
                    store.close();
                    if allowed {
                        compare_outputs(bridge, &spec, old, new, expected, dir.path(), candidate, &input, writer.as_ref(), &mut rows)
                    } else {
                        assert_eq!(before, read_graph(dir.path()), "refusal must not write page bytes");
                        json!({"refused": result.err()})
                    }
                } else {
                    if candidate["safe"] == true {
                        let gap = if new.starts_with(&format!("{old}/")) { "G5/B3b" } else { "G3/B4" };
                        rows.push(format!("{gap}: safe candidate cannot be selected through the original request"));
                    }
                    Value::Null
                };
                report.insert(label.clone(), json!({"differences": rows, "before": input, "candidate": candidate, "actual": evidence}));
                section.fixtures.insert(label, rows);
            }
            // Cancel is a UI decision: the backend has no cancel command.
            // Reading the offer must itself leave all bytes intact.
            assert_eq!(read_graph(seed.path()), spec.files, "cancel/offer page bytes");
            assert_eq!(fs::read(seed.path().join("logseq/config.edn")).ok(),
                initial_config, "cancel/offer config bytes");
            let dir = tempfile::tempdir().unwrap();
            spec.write(dir.path());
            let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
            let before = read_graph(dir.path());
            let config_before = fs::read(dir.path().join("logseq/config.edn")).ok();
            let result = execute(&store, old, new, None);
            let automatic = !operation["automatic"].is_null();
            if result.is_ok() != automatic {
                diffs.push(format!("G9/B6: unattended cardinality {}: model allowed={automatic}, Tine {result:?}", operation["offers"].as_array().unwrap().len()));
            }
            if result.is_err() {
                assert_eq!(before, read_graph(dir.path()), "unattended refusal is byte-exact");
                assert_eq!(config_before, fs::read(dir.path().join("logseq/config.edn")).ok());
            }
            store.close();
            let label = format!("{}: {old} -> {new}: offers/automatic", spec.name);
            report.insert(label.clone(), json!({"differences": diffs, "automatic": {
                "result": format!("{result:?}"), "before": before, "after": read_graph(dir.path()),
                "configBefore": config_before, "configAfter": fs::read(dir.path().join("logseq/config.edn")).ok()
            }}));
            section.fixtures.insert(label, diffs);
        }
        initial.close();
        results.lock().unwrap().push((section, report, candidates_run));
            });
        }
    });
    for (rows, evidence, count) in results.into_inner().unwrap() {
        section.fixtures.extend(rows.fixtures);
        report.extend(evidence);
        candidates_run += count;
    }
    assert!(candidates_run > 0);
    assert!(section.harness_skipped.is_empty());
    if let Ok(dest) = std::env::var("CM_REPORT_RENAME") {
        fs::write(dest, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    }
    let findings: Vec<_> = section
        .fixtures
        .iter()
        .flat_map(|(name, rows)| {
            rows.iter()
                .filter(|row| row.starts_with("FINDING"))
                .map(move |row| format!("{name}: {row}"))
        })
        .collect();
    assert!(
        findings.is_empty(),
        "unexplained r13 mutation discrepancies: {findings:#?}"
    );
    shared::check_expected(manifest, "renameNative15c", &section, "CM_REFRESH=1 CM_BRIDGE=<r13-offers> CM_NATIVE_BRIDGE=<15c> cargo test -p tine-graph-features --test integration conceptual_model_rename_conformance");
    eprintln!("r13 evaluated {candidates_run} candidates; zero skipped graphs");
}
