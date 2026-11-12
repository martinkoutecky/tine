//! Round-13 diagnostics consume typed date vertices and model projections.
use super::*;

#[test]
fn stored_query_inputs_keep_recursive_names_and_opaque_tokens_in_both_formats() {
    let base = r#"{{query (and [[Old]] (not (page "Branch")))}} {{tine-query @page and name = 'Other'}} {{query (task TODO)}}"#;
    for org in [false, true] {
        let literal = if org {
            "~{{query [[Hidden]]}}~"
        } else {
            "`{{query [[Hidden]]}}`"
        };
        let syntax = stored_queries(&format!("{base} {literal}"), org);
        assert_eq!(syntax.len(), 3, "literal macro is protected");
        let mut args = Vec::new();
        for q in &syntax {
            query_args(q, &mut args);
        }
        assert_eq!(args, ["Old", "Branch", "Other"]);
        assert_eq!(
            syntax[0],
            json!({"and": [{"refs": "Old"}, {"not": {"page": {"name": "Branch"}}}]})
        );
        assert!(syntax[2]["given"].as_str().unwrap().contains("task"));
    }
}

pub fn is_bridge(bridge: &Path) -> bool {
    run_bridge(bridge, &json!({})).expect("the selected bridge must run")["round"] == "r13"
}

pub fn fixtures(manifest: &str) -> Vec<(GraphSpec, Value)> {
    let path = fixture_dir(manifest).join("r13/cases.json");
    let cases: Vec<Value> = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    cases
        .into_iter()
        .map(|v| {
            (
                GraphSpec::from_fixture(v["name"].as_str().unwrap(), &v),
                v["expect"].clone(),
            )
        })
        .collect()
}

pub fn assert_contract(spec: &GraphSpec, model: &Value, expected: &Value) {
    assert_eq!(model["round"], "r13", "{}", spec.name);
    assert!(
        harness_reports(model).is_empty(),
        "{}: {:?}",
        spec.name,
        harness_reports(model)
    );
    if let Some(paths) = expected.get("activeFiles") {
        assert_eq!(&model["activeFiles"], paths, "{}", spec.name);
    }
    for (i, size) in expected["querySizes"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        assert_eq!(
            model["queries"][i].as_array().unwrap().len() as u64,
            size.as_u64().unwrap(),
            "{}",
            spec.name
        );
    }
    for (s, page) in expected["navigate"].as_object().into_iter().flatten() {
        assert_eq!(&model["navigate"][s], page, "{}: {s}", spec.name);
    }
    for (i, operation) in model["renames"].as_array().unwrap().iter().enumerate() {
        let candidates = operation["candidates"].as_array().unwrap();
        let offers = operation["offers"].as_array().unwrap();
        assert_eq!(
            operation["cancel"], model["physicalFiles"],
            "cancel is byte-exact"
        );
        assert_eq!(
            operation["automatic"].is_null(),
            offers.len() != 1,
            "unattended cardinality"
        );
        let ids: BTreeSet<_> = candidates
            .iter()
            .map(|c| c["candidate"].to_string())
            .collect();
        assert_eq!(
            ids.len(),
            candidates.len(),
            "deduplicate literal/member identities"
        );
        for candidate in candidates {
            let checks = candidate["checks"].as_object().unwrap();
            assert_eq!(checks.len(), 7);
            assert_eq!(candidate["safe"], checks.values().all(|v| v == true));
            assert_eq!(candidate["answerAllowed"], candidate["safe"]);
            assert_eq!(
                offers.contains(&candidate["candidate"]),
                candidate["safe"] == true
            );
            assert_eq!(
                candidate["favoritesPage"],
                json!(spec
                    .favorites_page
                    .clone()
                    .unwrap_or_else(|| NO_FAVORITES.to_owned()))
            );
            for row in candidate["images"].as_array().unwrap() {
                for image in row["after"].as_array().unwrap() {
                    assert!(
                        candidate["files"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|f| f["path"] == image[0]),
                        "origin images name retained physical outputs"
                    );
                }
            }
            assert!(
                !candidate.to_string().contains("__missing_path__"),
                "{}: missing codec input",
                spec.name
            );
        }
        if let Some(e) = expected["operations"].get(i) {
            if let Some(pages) = e.get("afterPages") {
                for candidate in candidates {
                    assert_eq!(&candidate["meaning"]["pages"], pages, "{}", spec.name);
                }
            }
            if let Some(v) = e.get("offers") {
                assert_eq!(&operation["offers"], v, "{}", spec.name);
            }
            if let Some(v) = e.get("preferred") {
                assert_eq!(&operation["preferred"], v, "{}", spec.name);
            }
            if let Some(block) = e.get("unchangedPreview") {
                let row = candidates[0]["preview"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|row| row["before"] == *block)
                    .expect("unchanged spelling must appear in the preview");
                assert_eq!(
                    row["beforeSignature"]["spellings"],
                    row["afterSignature"]["spellings"]
                );
                assert_ne!(
                    row["beforeSignature"]["openings"],
                    row["afterSignature"]["openings"]
                );
            }
            for (j, failed) in e["failedChecks"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                for name in failed.as_array().unwrap() {
                    assert_eq!(
                        candidates[j]["checks"][name.as_str().unwrap()],
                        false,
                        "{}: {name}",
                        spec.name
                    );
                }
            }
        }
    }
}

fn pair(entry: &super::super::store_crate::InventoryEntry) -> Page {
    (
        if entry.is_journal { "journal" } else { "page" }.to_owned(),
        page_key(&entry.name),
    )
}

pub fn compare(view: &WholeGraph, spec: &GraphSpec, input: &Value, model: &Value) -> Vec<Diff> {
    let ctx = Ctx::new(view, spec.journal_format(), input, model);
    let mut diffs = harness_reports(model);
    let inventory = view.inventory(InventoryScope::All);
    let entries = &inventory.0;
    let actual: BTreeSet<_> = entries
        .iter()
        .filter(|e| !matches!(e.target, Resolved::Alias { .. }))
        .map(pair)
        .collect();
    for p in ctx.pages.symmetric_difference(&actual) {
        diffs.push(
            Diff::new(
                "pages",
                format!(
                    "page {p:?}: model={}, Tine={}",
                    ctx.pages.contains(p),
                    actual.contains(p)
                ),
            )
            .at_key(&p.1),
        );
    }
    for owner in model["owners"].as_array().unwrap() {
        let p = page_pair(&owner["page"]);
        let actual = entries
            .iter()
            .find(|e| pair(e) == p)
            .and_then(|e| match &e.target {
                Resolved::Existing { id, .. } => Some(id.as_str()),
                _ => None,
            });
        if owner["path"].as_str() != actual {
            diffs.push(
                Diff::new(
                    "owners",
                    format!("{p:?}: model {}, Tine {actual:?}", owner["path"]),
                )
                .at_key(&p.1),
            );
        }
    }
    for (name, page) in model["navigate"].as_object().unwrap() {
        let link = view.link_target(name);
        let path = match view.resolve(&link.name, link.is_journal) {
            Resolved::Existing { id, .. } => Some(id),
            Resolved::Alias { owners } => owners.into_iter().next(),
            Resolved::Absent { .. } => None,
        };
        let p = path
            .and_then(|id| {
                view.inventory(InventoryScope::File(&id))
                    .0
                    .first()
                    .map(pair)
            })
            .unwrap_or_else(|| {
                spec.journal_format()
                    .parse(name)
                    .map(|d| {
                        (
                            "journal".to_owned(),
                            page_key(&spec.journal_format().title(d)),
                        )
                    })
                    .unwrap_or_else(|| ("page".to_owned(), page_key(name)))
            });
        if json!([p.0, p.1]) != *page {
            let real = model["targets"][name] == *page
                && model["owners"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|o| o["page"] == *page && o["path"].is_string());
            diffs.push(
                Diff::new(
                    if real {
                        "navigation real file"
                    } else {
                        "navigation"
                    },
                    format!("{name:?}: model {page}, Tine {p:?}"),
                )
                .at_key(name)
                .at_key(&p.1),
            );
        }
    }
    let counts: BTreeMap<_, _> = view.page_backlink_counts().0.into_iter().collect();
    for panel in model["panels"].as_array().unwrap() {
        let p = page_pair(&panel["page"]);
        let name = ctx
            .owner_of_page
            .get(&p)
            .map(|path| ctx.files[path].name.as_str())
            .unwrap_or(&p.1);
        let actual = group_blocks(
            &ctx,
            &view.backlinks(name).unwrap(),
            &mut diffs,
            "r13 panel",
        );
        let expected: BTreeSet<_> = panel["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .map(block_pair)
            .collect();
        if actual != expected {
            let mut diff = Diff::new(
                "linked refs",
                format!("{p:?}: model {expected:?}, Tine {actual:?}"),
            )
            .at_key(&p.1);
            for block in actual.symmetric_difference(&expected) {
                diff = diff.at_block(block);
            }
            diffs.push(diff);
        }
        if let Some(path) = ctx.owner_of_page.get(&p) {
            if counts.get(path).copied() != Some(expected.len()) {
                diffs.push(
                    Diff::new(
                        "counts",
                        format!(
                            "{p:?} ({path}): model {}, Tine {:?}",
                            expected.len(),
                            counts.get(path)
                        ),
                    )
                    .at_key(&p.1),
                );
            }
        }
    }
    diffs.extend(compare_queries(&ctx, &spec.queries));
    diffs
}

/// Keep concrete discrepancies rather than collapsing all duplicate graphs.
fn classify(input: &Value, model: &Value, mut diff: Diff) -> Diff {
    if diff.is_harness() {
        return diff;
    }
    let files = input["files"].as_array().unwrap();
    let name = |f: &Value| {
        page_key(
            f["title"]
                .as_str()
                .filter(|_| f["dir"] == "pages")
                .unwrap_or_else(|| f["stem"].as_str().unwrap()),
        )
    };
    let mut twin_keys = BTreeSet::new();
    let mut losing = BTreeSet::new();
    for f in files {
        if !model["activeFiles"]
            .as_array()
            .unwrap()
            .contains(&f["path"])
        {
            losing.insert(f["path"].as_str().unwrap());
            twin_keys.insert(name(f));
            // A losing file's alias/reference/filename-parent contribution.
            for s in strings(&f["aliases"]).into_iter().chain(
                f["blocks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .flat_map(|b| b["occ"].as_array().unwrap().iter())
                    .map(|o| o[1].as_str().unwrap().to_owned()),
            ) {
                twin_keys.insert(page_key(&s));
            }
        }
    }
    let mut date_keys: BTreeSet<_> = input["canonicalDates"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, value)| !value.is_null())
        .map(|(s, _)| page_key(s))
        .collect();
    for (s, target) in model["targets"].as_object().unwrap() {
        if target[0] == "journal" {
            date_keys.insert(page_key(s));
            date_keys.insert(target[1].as_str().unwrap().to_owned());
        }
    }
    // Follow alias connectivity, including declarations from inactive files.
    let edges: Vec<_> = files
        .iter()
        .flat_map(|f| {
            strings(&f["aliases"])
                .into_iter()
                .map(|s| (name(f), page_key(&s)))
                .collect::<Vec<_>>()
        })
        .collect();
    for _ in 0..=edges.len() {
        for (a, b) in &edges {
            for keys in [&mut twin_keys, &mut date_keys] {
                if keys.contains(a) || keys.contains(b) {
                    keys.extend([a.clone(), b.clone()]);
                }
            }
        }
    }
    let supported_names: BTreeSet<_> = files
        .iter()
        .flat_map(|f| {
            let mut names = strings(&f["aliases"]);
            if !f["text"].as_str().unwrap().trim().is_empty() {
                names.push(name(f));
            }
            names.extend(f["blocks"].as_array().unwrap().iter().flat_map(|b| {
                b["occ"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|o| page_key(o[1].as_str().unwrap()))
            }));
            names.into_iter().map(|s| page_key(&s)).collect::<Vec<_>>()
        })
        .collect();
    let parents: Vec<_> = twin_keys
        .iter()
        .filter(|key| supported_names.contains(*key))
        .flat_map(|key| tine_core::refs::namespace_refs(key).map(str::to_owned))
        .collect();
    twin_keys.extend(parents);
    for (literal, canonical) in input["canonicalDates"].as_object().into_iter().flatten() {
        if twin_keys.contains(literal) {
            twin_keys.insert(canonical.as_str().unwrap().to_owned());
        }
    }
    let losing_block = diff
        .blocks
        .iter()
        .any(|(path, _)| losing.contains(path.as_str()));
    for (path, i) in diff
        .blocks
        .iter()
        .filter(|_| diff.class.starts_with("query"))
    {
        if let Some(f) = files.iter().find(|f| f["path"] == *path) {
            diff.keys.insert(name(f));
            let mut level = u64::MAX;
            for j in (0..=*i).rev() {
                let block = &f["blocks"][j];
                let next = block["level"].as_u64().unwrap();
                if next < level {
                    for occ in block["occ"].as_array().unwrap() {
                        diff.keys.insert(page_key(occ[1].as_str().unwrap()));
                    }
                    level = next;
                }
            }
        }
    }
    let empty_redirect = diff.class == "navigation real file"
        && diff.keys.iter().any(|key| {
            files.iter().any(|f| {
                name(f) == *key
                    && model["activeFiles"]
                        .as_array()
                        .unwrap()
                        .contains(&f["path"])
                    && f["text"]
                        .as_str()
                        .unwrap()
                        .lines()
                        .all(|line| line.trim().is_empty() || matches!(line.trim(), "-" | "*"))
            }) && files.iter().any(|f| {
                model["activeFiles"]
                    .as_array()
                    .unwrap()
                    .contains(&f["path"])
                    && name(f) != *key
                    && diff.keys.contains(&name(f))
                    && strings(&f["aliases"])
                        .iter()
                        .any(|alias| page_key(alias) == *key)
            })
        });
    // Active ordinary date files retain their own spelling (KD/P2). Their
    // link relation can still expand the same day's journal spellings (G11),
    // even when an unrelated losing twin shares the source's key.
    let real_date_link = diff.class == "links"
        && files.iter().any(|f| {
            f["dir"] == "pages"
                && model["activeFiles"]
                    .as_array()
                    .unwrap()
                    .contains(&f["path"])
                && input["canonicalDates"].as_object().is_some_and(|dates| {
                    dates.iter().any(|(literal, canonical)| {
                        name(f) == page_key(literal)
                            && canonical
                                .as_str()
                                .is_some_and(|key| diff.keys.contains(key))
                            && files.iter().any(|source| {
                                model["activeFiles"]
                                    .as_array()
                                    .unwrap()
                                    .contains(&source["path"])
                                    && diff.keys.contains(&name(source))
                                    && source["blocks"].as_array().unwrap().iter().any(|block| {
                                        block["occ"].as_array().unwrap().iter().any(|occ| {
                                            page_key(occ[1].as_str().unwrap()) == page_key(literal)
                                        })
                                    })
                            })
                    })
                })
        });
    let gap = if losing_block {
        "G1/B1"
    } else if empty_redirect {
        "G2/B2"
    } else if real_date_link {
        "G11/B2a"
    } else if !diff.keys.is_disjoint(&twin_keys) {
        "G1/B1"
    } else if !diff.keys.is_disjoint(&date_keys) {
        "G11/B2a"
    } else {
        "FINDING"
    };
    diff.class = format!("{gap}: {}", diff.class);
    diff
}

#[test]
fn attribution_requires_the_differing_item_to_involve_the_gap() {
    let input = json!({"files": [
        {"path":"pages/A.md", "stem":"A", "title":null, "dir":"pages", "text":"- visible", "aliases":[], "blocks":[]},
        {"path":"pages/a.org", "stem":"a", "title":null, "dir":"pages", "text":"* hidden", "aliases":["Hidden"], "blocks":[]},
        {"path":"pages/B.md", "stem":"B", "title":null, "dir":"pages", "text":"-\n", "aliases":[], "blocks":[]},
        {"path":"pages/Owner.md", "stem":"Owner", "title":null, "dir":"pages", "text":"alias:: B", "aliases":["B"], "blocks":[]}
    ], "canonicalDates":{"2026-10-26":"oct 26th, 2026"}});
    let model = json!({"activeFiles":["pages/A.md", "pages/B.md", "pages/Owner.md"],
        "targets":{"2026-10-26":["journal","oct 26th, 2026"]}});
    let classify_key = |class, key| {
        classify(
            &input,
            &model,
            Diff::new(class, "injected discrepancy").at_key(key),
        )
        .class
    };
    assert!(classify_key("pages", "Unrelated").starts_with("FINDING"));
    assert!(classify_key("pages", "Hidden").starts_with("G1/B1"));
    assert!(classify_key("pages", "2026-10-26").starts_with("G11/B2a"));
    assert!(classify(
        &input,
        &model,
        Diff::new("navigation real file", "blank file opens alias owner")
            .at_key("B")
            .at_key("Owner")
    )
    .class
    .starts_with("G2/B2"));
    assert!(classify_key("navigation real file", "B").starts_with("FINDING"));
    assert!(classify_key("navigation real file", "Owner").starts_with("FINDING"));
    let empty_twins = json!({"files":[
        {"path":"pages/N%2Fy.md", "stem":"N/y", "dir":"pages", "text":"", "aliases":[], "blocks":[]},
        {"path":"pages/n%2Fy.md", "stem":"n/y", "dir":"pages", "text":"", "aliases":[], "blocks":[]}
    ]});
    let active = json!({"activeFiles":["pages/N%2Fy.md"], "targets":{}});
    assert!(classify(
        &empty_twins,
        &active,
        Diff::new("pages", "no text seeds this parent").at_key("n")
    )
    .class
    .starts_with("FINDING"));
}

pub fn store_conformance(
    bridge: &Path,
    legacy: Vec<GraphSpec>,
    manifest: &str,
    extra: impl Fn(&WholeGraph, &Value, &Value) -> Vec<Diff>,
) {
    let mut section = Section::default();
    let mut report = BTreeMap::new();
    let n = std::env::var("CM_RANDOM")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    let specs = legacy
        .into_iter()
        .map(|s| (s, Value::Null))
        .chain(fixtures(manifest))
        .chain((0..n).map(|i| (random_graph(i), Value::Null)));
    let mut runs = 0;
    let mut closers = Vec::new();
    for (spec, expected) in specs {
        let dir = tempfile::tempdir().unwrap();
        spec.write(dir.path());
        let store = super::super::store_crate::Store::open(dir.path(), Default::default())
            .unwrap()
            .0;
        let view = store.whole_graph().unwrap();
        let input = model_input_r13(&spec);
        let model = run_bridge(bridge, &input).expect("r13 bridge executes for every graph");
        assert_contract(&spec, &model, &expected);
        let mut found = compare(&view, &spec, &input, &model);
        found.extend(extra(&view, &input, &model));
        let diffs: Vec<_> = found
            .into_iter()
            .map(|d| classify(&input, &model, d))
            .collect();
        assert!(
            !diffs.iter().any(Diff::is_harness),
            "{}: {diffs:?}",
            spec.name
        );
        report.insert(spec.name.clone(), json!({"differences": diffs.iter().map(|d| json!({"class": d.class, "detail": d.detail, "keys": d.keys, "blocks": d.blocks})).collect::<Vec<_>>(),
            "input": input, "model": model, "operations": model["renames"]}));
        section.fixtures.insert(
            spec.name.clone(),
            diffs
                .iter()
                .map(|d| format!("{}: {}", d.class, d.detail))
                .collect(),
        );
        runs += 1;
        drop(view);
        closers.push(std::thread::spawn(move || {
            store.close();
            drop(dir);
        }));
    }
    for closer in closers {
        closer.join().unwrap();
    }
    assert!(runs > n, "r13 fixtures actually ran");
    assert!(section.harness_skipped.is_empty());
    if let Ok(dest) = std::env::var("CM_REPORT") {
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
        "unexplained r13 discrepancies: {findings:#?}"
    );
    check_expected(
        manifest,
        "storeR13",
        &section,
        "CM_REFRESH=1 CM_BRIDGE=<r13> cargo test -p tine-store --lib cm_conformance",
    );
    eprintln!("r13 bridge executed {runs} graphs; zero skipped");
}
