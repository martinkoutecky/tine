//! CM step 2: the static comparison against the confirmed model (round `k`).
//!
//! The store's static conformance test hands each graph to the bridge over
//! the confirmed model (path in `CM_STATIC_BRIDGE`; its answers carry
//! `"round": "k"`) and compares the static answers Tine gives: pages,
//! conflicts, active files, owners, navigation, Linked References and their
//! counts, raw query rows, the link table, the namespace tree (`tree`,
//! `namespaceTree`) and the alias classes (`aliasEdges`). Real operation
//! witnesses run in `tine-graph-features/tests/cm_witness.rs` without an
//! operation bridge.
//!
//! What the model needs beyond the older protocol:
//! - **Day values, not labels.** A date is the identity `(journal, day)`
//!   (SPEC §2.1). `canonicalDates` maps each spelling's literal key to an
//!   injective day token ([`day_token`]), never to the day's rendered title,
//!   and Tine's own journal answers are projected to the same token (Tine's
//!   `InventoryEntry::day`, else the native parse of the name).
//! - **The filename-day table.** The bridge reads a journals-directory file's
//!   day from its raw file stem (`rawStem`) through `canonicalDates`, apart
//!   from its displayed title; every raw stem is therefore keyed, and parsed
//!   with the graph's native journal parse (`JournalFormat::parse`, the parse
//!   Tine's reader applies to a journal file name).
//! - **Only queries the bridge can evaluate.** The bridge has no evaluations
//!   for opaque `given` leaves, so a query containing one is not requested; it
//!   is recorded as a difference line of its own, never dropped silently.
//!
//! Every difference is attributed to a named class ([`attribute`]); a
//! difference no class explains is a `FINDING` and fails the test.

use super::*;

/// An injective token for a calendar day. Upper case, so no ordinary page
/// key (`page_key` folds case) can equal it.
pub fn day_token(day: JournalDate) -> String {
    format!("Day {:04}-{:02}-{:02}", day.year, day.month, day.day)
}

/// The day token of Tine's own ordinal day (`Day`, `yyyymmdd`).
pub fn ordinal_token(ordinal: i64) -> String {
    day_token(JournalDate::from_ordinal(ordinal))
}

/// The model's key of spelling `s`: its day's token when the graph's native
/// journal parse accepts it, else Tine's ordinary key. This is the function
/// the input's `canonicalDates` and `literalKeys` describe to the bridge.
pub fn key(journal: &JournalFormat, s: &str) -> String {
    journal
        .parse(s)
        .map(day_token)
        .unwrap_or_else(|| page_key(s))
}

fn has_given(q: &Value) -> bool {
    q.get("given").is_some()
        || q.get("page").is_some_and(has_given)
        || q.get("not").is_some_and(has_given)
        || ["and", "or"].iter().any(|op| {
            q.get(op)
                .and_then(Value::as_array)
                .is_some_and(|a| a.iter().any(has_given))
        })
}

/// The queries of `spec` the bridge can evaluate, and the ones it cannot.
pub fn requested_queries(spec: &GraphSpec) -> (Vec<QuerySpec>, Vec<QuerySpec>) {
    spec.queries.iter().cloned().partition(|q| !has_given(&q.q))
}

/// The bridge's input: Tine's parser and configuration answers only. No
/// owner, no rename and no operation field is supplied.
pub fn model_input(spec: &GraphSpec) -> Value {
    let journal = spec.journal_format();
    let config = spec.config();
    let mut input = json!({
        "files": spec.files.iter().filter(|(rel, _)| !is_sync_copy(rel))
            .map(|(rel, text)| file_json(rel, text, alias_spellings, spec.name_format(), &config)).collect::<Vec<_>>(),
        "navigate": spec.navigate,
        "favoritesPage": spec.favorites_page.clone().unwrap_or_else(|| NO_FAVORITES.to_owned()),
    });
    let (queries, _) = requested_queries(spec);
    input["favorites"] = json!(config.favorites);
    input["queries"] = json!(queries
        .iter()
        .map(|q| {
            let dialect = if q.dialect == "tql" {
                QueryDialect::Tql
            } else {
                QueryDialect::Og
            };
            let (parsed, _) = parse_query_text(&q.tine, dialect, JournalDate::from_days(0));
            json!({"anchor": q.anchor, "q": q.q,
            "showHeaders": parsed.anchor == tine_core::query::ir::Anchor::Block,
            "removeBlockChildren": parsed.removes_block_children()})
        })
        .collect::<Vec<_>>());
    let mut raw_stems = Vec::new();
    for file in input["files"].as_array_mut().unwrap() {
        let rel = file["path"].as_str().unwrap().to_owned();
        let raw = Path::new(&rel).file_stem().unwrap().to_str().unwrap();
        file["rawStem"] = json!(raw);
        raw_stems.push(raw.to_owned());
    }
    let mut extra = spec.navigate.clone();
    extra.extend(config.favorites.iter().cloned());
    extra.extend(spec.favorites_page.iter().cloned());
    extra.extend(raw_stems);
    for q in &queries {
        query_args(&q.q, &mut extra);
    }
    for file in input["files"].as_array().unwrap() {
        for b in file["blocks"].as_array().unwrap() {
            for q in b["stored"].as_array().unwrap() {
                query_args(q, &mut extra);
            }
        }
    }
    let mut all = spellings(&input, &extra);
    // Namespace parents of every spelling and every spelling of each day in
    // play are keyed too: the bridge keys names it derives itself.
    let more: Vec<_> = all
        .iter()
        .flat_map(|s| {
            let mut names = tine_core::refs::namespace_refs(s)
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if let Some(day) = journal.parse(s) {
                names.extend(journal.spellings(day));
                names.push(journal.title(day));
            }
            names
        })
        .collect();
    all.extend(more);
    input["literalKeys"] = json!(all
        .iter()
        .map(|s| (s, page_key(s)))
        .collect::<BTreeMap<_, _>>());
    input["keys"] = input["literalKeys"].clone();
    input["canonicalDates"] = json!(all
        .iter()
        .filter_map(|s| journal.parse(s).map(|d| (page_key(s), day_token(d))))
        .collect::<BTreeMap<_, _>>());
    input["dateKeys"] = json!(all
        .iter()
        .filter_map(|s| journal.parse(s).map(day_token))
        .collect::<BTreeSet<_>>());
    input["claimantConfig"] = json!({
        "kindPref": "page", "order": "unsigned-utf8-relative-path",
    });
    input["urls"] = json!(all
        .iter()
        .filter(|s| tine_core::refs::url_has_origin(s))
        .collect::<Vec<_>>());
    input
}

/// Tine's page for an inventory entry, as the model's (kind, key).
fn entry_page(journal: &JournalFormat, e: &super::super::store_crate::InventoryEntry) -> Page {
    if e.is_journal {
        let key = match e.day {
            Some(d) => ordinal_token(d.0),
            None => key(journal, &e.name),
        };
        ("journal".to_owned(), key)
    } else {
        ("page".to_owned(), page_key(&e.name))
    }
}

/// Tine's page for a resolution, as the model's (kind, key).
pub fn resolved_page(
    view: &WholeGraph,
    journal: &JournalFormat,
    r: &Resolved,
    name: &str,
    journal_kind: bool,
) -> Page {
    let entry_of = |id: &PageId| {
        let inv = view.inventory(InventoryScope::File(id));
        let e = inv.0.first().expect("a claimant has an inventory entry");
        entry_page(journal, e)
    };
    match r {
        Resolved::Existing { id, .. } => entry_of(id),
        Resolved::Alias { owners } => entry_of(&owners[0]),
        Resolved::Absent { .. } if journal_kind => ("journal".to_owned(), key(journal, name)),
        Resolved::Absent { .. } => ("page".to_owned(), page_key(name)),
    }
}

/// A spelling of model page `p` that Tine's name-keyed APIs accept. A day
/// is asked by a spelling that identifies it: under a year-less title format
/// (`MM-dd`) its displayed name is a label other days and an ordinary page
/// share, and "typing the ambiguous, non-identifying label cannot select
/// one" (SPEC §2.1). With a year in the title that spelling is the title.
pub fn spelling(ctx: &Ctx, p: &Page) -> String {
    let day = (p.0 == "journal")
        .then(|| p.1.strip_prefix("Day "))
        .flatten()
        .and_then(|day| JournalFormat::new(None, None).parse(day));
    if let Some(path) = ctx.owner_of_page.get(p) {
        if let Some(e) = ctx
            .view
            .inventory(InventoryScope::File(&PageId::from(path.as_str())))
            .0
            .first()
        {
            return match day {
                Some(d) if ctx.journal.parse(&e.name) != Some(d) => {
                    ctx.journal.identifying_spelling(d)
                }
                _ => e.name.clone(),
            };
        }
    }
    if let Some(d) = day {
        return ctx.journal.identifying_spelling(d);
    }
    p.1.clone()
}

/// Every static answer Tine gives against the model's.
pub fn compare(view: &WholeGraph, spec: &GraphSpec, input: &Value, model: &Value) -> Vec<Diff> {
    let journal = spec.journal_format();
    let ctx = Ctx::new(view, spec.journal_format(), input, model);
    let mut diffs = Vec::new();
    for s in model["nonLocal"].as_array().unwrap() {
        diffs.push(Diff::new(
            "harness: nonLocal",
            format!("harness: nonLocal {s}"),
        ));
    }
    let inventory = view.inventory(InventoryScope::All);
    let entries = &inventory.0;
    let real: Vec<_> = entries
        .iter()
        .filter(|e| !matches!(e.target, Resolved::Alias { .. }))
        .collect();
    // Pages (SPEC §2.2 page universe).
    let actual: BTreeSet<Page> = real.iter().map(|e| entry_page(&journal, e)).collect();
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
    // Conflicts and active files (SPEC §2.2 claimants).
    let mut tine_conflicts = BTreeSet::new();
    let mut tine_active = BTreeSet::new();
    for e in &real {
        if let Resolved::Existing { id, others } = &e.target {
            tine_active.insert(id.as_str().to_owned());
            if !others.is_empty() {
                tine_conflicts.insert(entry_page(&journal, e).1);
            }
        }
    }
    let model_conflicts: BTreeSet<String> = strings(&model["conflicts"]).into_iter().collect();
    for k in model_conflicts.symmetric_difference(&tine_conflicts) {
        diffs.push(
            Diff::new(
                "conflicts",
                format!(
                    "{k:?} conflicted: model={}, Tine={}",
                    model_conflicts.contains(k),
                    tine_conflicts.contains(k)
                ),
            )
            .at_key(k),
        );
    }
    let model_active: BTreeSet<String> = strings(&model["activeFiles"]).into_iter().collect();
    for path in model_active.symmetric_difference(&tine_active) {
        let mut diff = Diff::new(
            "active files",
            format!(
                "{path}: model={}, Tine={}",
                model_active.contains(path),
                tine_active.contains(path)
            ),
        );
        if let Some(f) = ctx.files.get(path) {
            diff = diff.at_key(&f.name);
        }
        // The file itself, as a block witness past its last position.
        diffs.push(diff.at_block(&(path.clone(), usize::MAX)));
    }
    // Owners.
    for owner in model["owners"].as_array().unwrap() {
        let p = page_pair(&owner["page"]);
        let actual = real
            .iter()
            .find(|e| entry_page(&journal, e) == p)
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
    // Navigation: what `[[name]]` opens (SPEC §2.3).
    for (name, page) in model["navigate"].as_object().unwrap() {
        let link = view.link_target(name);
        let p = resolved_page(
            view,
            &journal,
            &view.resolve(&link.name, link.is_journal),
            &link.name,
            link.is_journal,
        );
        if json!([p.0, p.1]) != *page {
            diffs.push(
                Diff::new("navigation", format!("{name:?}: model {page}, Tine {p:?}"))
                    .at_key(name)
                    .at_key(&p.1)
                    .at_key(page[1].as_str().unwrap()),
            );
        }
    }
    // Linked References and their counts (SPEC §6.1).
    let counts: BTreeMap<_, _> = view.page_backlink_counts().0.into_iter().collect();
    for panel in model["panels"].as_array().unwrap() {
        let p = page_pair(&panel["page"]);
        let name = spelling(&ctx, &p);
        let actual = match view.backlinks(&name) {
            Ok(groups) => group_blocks(&ctx, &groups, &mut diffs, "k panel"),
            Err(e) => {
                diffs.push(Diff::new(
                    "harness: backlinks error",
                    format!("backlinks({name:?}): {e:?}"),
                ));
                continue;
            }
        };
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
    // The namespace tree and the alias classes (SPEC §6.3 views).
    diffs.extend(compare_trees(&journal, &real, &actual, model));
    diffs.extend(compare_alias_classes(&ctx, view, model));
    // Raw query rows (SPEC §6.2), requested queries only.
    let (queries, unrequested) = requested_queries(spec);
    diffs.extend(compare_queries(&ctx, &queries));
    for q in unrequested {
        diffs.push(Diff::new(
            "not requested",
            format!(
                "query {:?}: a given leaf the bridge cannot evaluate",
                q.tine
            ),
        ));
    }
    diffs
}

/// The page an inventory row's parent link names, as the model's (kind, key).
fn link_page(journal: &JournalFormat, link: &super::super::store_crate::LinkTarget) -> Page {
    if link.is_journal {
        ("journal".to_owned(), key(journal, &link.name))
    } else {
        ("page".to_owned(), page_key(&link.name))
    }
}

/// The model's tree rows (a page and its children) as a map.
fn tree_rows(rows: &Value) -> BTreeMap<Page, BTreeSet<Page>> {
    rows.as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                page_pair(&row["page"]),
                row["children"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(page_pair)
                    .collect(),
            )
        })
        .collect()
}

/// The namespace tree as the inventory gives it: a row's parent is its typed
/// `parent` (set when it is not the textual prefix's own page), else the page
/// of its immediate namespace prefix; the edge counts only when the parent is
/// a listed page. Against the model's `tree` (every page with its children)
/// and `namespaceTree` (the rows with a parent or a child).
fn compare_trees(
    journal: &JournalFormat,
    real: &[&super::super::store_crate::InventoryEntry],
    pages: &BTreeSet<Page>,
    model: &Value,
) -> Vec<Diff> {
    let mut tine: BTreeMap<Page, BTreeSet<Page>> =
        pages.iter().map(|p| (p.clone(), BTreeSet::new())).collect();
    for e in real {
        let page = entry_page(journal, e);
        let parent = match &e.parent {
            Some(link) => Some(link_page(journal, link)),
            None => tine_core::refs::namespace_refs(&e.name)
                .filter(|prefix| prefix.len() < e.name.len())
                .map(page_key)
                .filter(|key| !key.is_empty())
                .last()
                .map(|key| ("page".to_owned(), key)),
        };
        if let Some(parent) = parent.filter(|parent| pages.contains(parent)) {
            tine.entry(parent).or_default().insert(page);
        }
    }
    let has_edge = |tree: &BTreeMap<Page, BTreeSet<Page>>, page: &Page| {
        !tree[page].is_empty() || tree.values().any(|children| children.contains(page))
    };
    let namespace = |tree: &BTreeMap<Page, BTreeSet<Page>>| -> BTreeMap<Page, BTreeSet<Page>> {
        tree.iter()
            .filter(|(page, _)| has_edge(tree, page))
            .map(|(page, children)| (page.clone(), children.clone()))
            .collect()
    };
    let mut diffs = Vec::new();
    for (class, expected, actual) in [
        ("tree", tree_rows(&model["tree"]), tine.clone()),
        (
            "namespace tree",
            tree_rows(&model["namespaceTree"]),
            namespace(&tine),
        ),
    ] {
        for page in expected
            .keys()
            .chain(actual.keys())
            .collect::<BTreeSet<_>>()
        {
            let (model_children, tine_children) = (expected.get(page), actual.get(page));
            if model_children != tine_children {
                let mut diff = Diff::new(
                    class,
                    format!("{page:?}: model {model_children:?}, Tine {tine_children:?}"),
                )
                .at_key(&page.1);
                for child in model_children.into_iter().chain(tine_children).flatten() {
                    diff = diff.at_key(&child.1);
                }
                diffs.push(diff);
            }
        }
    }
    diffs
}

/// Alias classes: the model's alias edges (a declaring page and each page its
/// aliases name) close to classes; the inventory's alias group of a name is
/// Tine's class. Compared per page that an edge names.
fn compare_alias_classes(ctx: &Ctx, view: &WholeGraph, model: &Value) -> Vec<Diff> {
    let typed = model["typedAliasEdges"].as_array().unwrap();
    let mut diffs = Vec::new();
    let mut class: BTreeMap<Page, BTreeSet<Page>> = BTreeMap::new();
    for edge in typed {
        let (a, b) = (page_pair(&edge[0]), page_pair(&edge[1]));
        let mut merged = BTreeSet::from([a.clone(), b.clone()]);
        for member in [&a, &b] {
            merged.extend(class.get(member).cloned().unwrap_or_default());
        }
        for member in merged.clone() {
            class.insert(member, merged.clone());
        }
    }
    for (page, expected) in &class {
        let name = spelling(ctx, page);
        let mut actual = BTreeSet::from([page.clone()]);
        actual.extend(
            view.inventory(InventoryScope::AliasGroup(&name))
                .0
                .iter()
                .map(|e| entry_page(&ctx.journal, e)),
        );
        if &actual != expected {
            let mut diff = Diff::new(
                "alias classes",
                format!("{page:?}: model {expected:?}, Tine {actual:?}"),
            );
            for member in expected.iter().chain(&actual) {
                diff = diff.at_key(&member.1);
            }
            diffs.push(diff);
        }
    }
    diffs
}

/// The model's directed links (SPEC §6.3) as typed pairs.
pub fn model_links(model: &Value) -> BTreeSet<(Page, Page)> {
    model["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| (page_pair(&edge[0]), page_pair(&edge[1])))
        .collect()
}

/// Tine's link table against the model's links.
pub fn compare_links(actual: &BTreeSet<(Page, Page)>, model: &Value) -> Vec<Diff> {
    let expected = model_links(model);
    actual
        .symmetric_difference(&expected)
        .map(|(source, target)| {
            Diff::new(
                "links",
                format!(
                    "link {source:?} -> {target:?}: model={}, Tine={}",
                    expected.contains(&(source.clone(), target.clone())),
                    actual.contains(&(source.clone(), target.clone()))
                ),
            )
            .at_key(&source.1)
            .at_key(&target.1)
        })
        .collect()
}

/// What the attribution of one graph's differences needs. Every key is folded
/// as `Diff::at_key` folds a difference's keys.
pub struct Facts {
    /// Files whose claim Tine reads as another identity than the model's
    /// (SPEC §2.1): path -> (Tine's page, the model's claim).
    pub reclaimed: BTreeMap<String, (Option<Page>, Page)>,
    /// Reclaimed files that lose their claim in the model (SPEC §2.2:
    /// inactive).
    pub losing: BTreeSet<String>,
    /// Journals-directory files the model selects and Tine does not (SPEC
    /// §2.2: the least claimant is journals-directory first).
    pub journal_first: BTreeSet<String>,
    /// The claims of reclaimed files under either reading, with every
    /// spelling of a claimed day.
    claim_keys: BTreeSet<String>,
    /// Those claims closed under the graph's alias declarations.
    claim_component: BTreeSet<String>,
    /// Blocks with an occurrence that names a name of `claim_component`.
    claim_blocks: BTreeSet<BlockId>,
    /// What reclaimed files name: aliases, occurrences, their namespace
    /// prefixes, closed under the graph's alias declarations.
    reclaimed_keys: BTreeSet<String>,
    /// The same for losing files.
    losing_keys: BTreeSet<String>,
}

/// The model's claim of an input file (SPEC §2.1), as the bridge reads it
/// from the supplied givens: a journals-directory file by its raw file stem,
/// any other file by its effective name.
fn model_claim(journal: &JournalFormat, f: &Value) -> Page {
    let name = f["title"]
        .as_str()
        .filter(|_| f["dir"] == "pages")
        .unwrap_or_else(|| f["stem"].as_str().unwrap());
    let day = if f["dir"] == "journals" {
        journal.parse(f["rawStem"].as_str().unwrap())
    } else {
        journal.parse(name)
    };
    match day {
        Some(d) => ("journal".to_owned(), day_token(d)),
        None => ("page".to_owned(), page_key(name)),
    }
}

pub fn facts(view: &WholeGraph, journal: &JournalFormat, input: &Value, model: &Value) -> Facts {
    let active: BTreeSet<String> = strings(&model["activeFiles"]).into_iter().collect();
    let mut facts = Facts {
        reclaimed: BTreeMap::new(),
        losing: BTreeSet::new(),
        journal_first: BTreeSet::new(),
        claim_keys: BTreeSet::new(),
        claim_component: BTreeSet::new(),
        claim_blocks: BTreeSet::new(),
        reclaimed_keys: BTreeSet::new(),
        losing_keys: BTreeSet::new(),
    };
    let files = input["files"].as_array().unwrap();
    let name_of = |f: &Value| {
        f["title"]
            .as_str()
            .filter(|_| f["dir"] == "pages")
            .unwrap_or_else(|| f["stem"].as_str().unwrap())
            .to_owned()
    };
    let contributions = |f: &Value| {
        let mut spelled = vec![name_of(f)];
        spelled.extend(strings(&f["aliases"]));
        for b in f["blocks"].as_array().unwrap() {
            for o in b["occ"].as_array().unwrap() {
                spelled.push(o[1].as_str().unwrap().to_owned());
            }
        }
        let mut keys = BTreeSet::new();
        for s in &spelled {
            for u in tine_core::refs::namespace_refs(s) {
                keys.insert(page_key(u));
                keys.insert(page_key(&key(journal, u)));
            }
            keys.insert(page_key(s));
            keys.insert(page_key(&key(journal, s)));
        }
        keys
    };
    let day_spellings = |token: &str| {
        input["canonicalDates"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, t)| t.as_str() == Some(token))
            .map(|(literal, _)| page_key(literal))
            .collect::<Vec<_>>()
    };
    for f in files {
        let path = f["path"].as_str().unwrap().to_owned();
        let claim = model_claim(journal, f);
        let entry = view
            .inventory(InventoryScope::File(&PageId::from(path.as_str())))
            .0
            .first()
            .cloned();
        let tine_holds = matches!(
            entry.as_ref().map(|e| &e.target),
            Some(Resolved::Existing { id, .. }) if id.as_str() == path
        );
        if f["dir"] == "journals" && active.contains(&path) && !tine_holds {
            facts.journal_first.insert(path.clone());
        }
        let tine = entry.as_ref().map(|e| entry_page(journal, e));
        if tine.as_ref() != Some(&claim) {
            for p in tine.iter().chain([&claim]) {
                facts.claim_keys.insert(page_key(&p.1));
                facts.claim_keys.extend(day_spellings(&p.1));
            }
            facts.reclaimed_keys.extend(contributions(f));
            facts.reclaimed.insert(path.clone(), (tine, claim));
        }
        // Only a reclaimed file loses for a reason Tine does not share: Tine
        // reads every other claimant contest as the model does.
        if !active.contains(&path) && facts.reclaimed.contains_key(&path) {
            facts.losing_keys.extend(contributions(f));
            facts.losing.insert(path);
        }
    }
    // Alias components: a declaration joins the declarer (its name and its
    // model claim) and the declared name, in either reading.
    let edges: Vec<(String, String)> = files
        .iter()
        .flat_map(|f| {
            let claim = page_key(&model_claim(journal, f).1);
            strings(&f["aliases"])
                .into_iter()
                .flat_map(|a| {
                    let (literal, keyed) = (page_key(&a), page_key(&key(journal, &a)));
                    [
                        (page_key(&name_of(f)), literal.clone()),
                        (claim.clone(), literal.clone()),
                        (literal, keyed),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .collect();
    facts.claim_component = facts.claim_keys.clone();
    for _ in 0..=edges.len() {
        for (a, b) in &edges {
            for keys in [
                &mut facts.claim_component,
                &mut facts.reclaimed_keys,
                &mut facts.losing_keys,
            ] {
                if keys.contains(a) || keys.contains(b) {
                    keys.extend([a.clone(), b.clone()]);
                }
            }
        }
    }
    // A reference spelled as a reclaimed claim (or a name of its alias
    // component) names another identity in each reading.
    for f in files {
        let path = f["path"].as_str().unwrap();
        for (i, b) in f["blocks"].as_array().unwrap().iter().enumerate() {
            let names = b["occ"].as_array().unwrap().iter().any(|o| {
                let s = o[1].as_str().unwrap();
                facts.claim_component.contains(&page_key(s))
                    || facts.claim_component.contains(&page_key(&key(journal, s)))
            });
            if names {
                facts.claim_blocks.insert((path.to_owned(), i));
            }
        }
    }
    facts
}

/// The class a difference belongs to. Each class is one rule of the
/// confirmed model that Tine's reading does not follow yet (see the
/// receipt of CM step 2's static retarget); `FINDING` is a difference no
/// class explains.
///
/// - `K1 (§2.1 file claim)`: the difference is about a file Tine reads as
///   another identity than the model's claim (an effective name that is an
///   accepted date spelling is the day `(journal, day)` wherever the file
///   sits; a journal file's day comes from its file name), or about a block
///   of one, or about a name it claims under either reading.
/// - `K2 (§2.2 inactive claimant)`: the difference is about a file that loses
///   its claim in the model (one active holder per identity, journals
///   directory first; an inactive file contributes no alias, block,
///   reference, namespace seed or query row), or about a name it contributes.
/// - `K1 (§2.1 file claim, contributed name)`: about a name a reclaimed file
///   contributes (an alias, an occurrence, their namespace prefixes and
///   alias components).
/// - `K3 (§2.2 journals directory first)`: a row naming which file holds a
///   claim (active files, owners, counts), where the model selects a
///   journals-directory file and Tine selects another claimant.
/// - `K1 (§2.1 file claim, reference)`: about a block whose reference is
///   spelled as a reclaimed claim, or a name of its alias component (the
///   reference names the day in the model and the pages-dir file's page in
///   Tine), or about a name of that component.
pub fn attribute(facts: &Facts, mut diff: Diff) -> Diff {
    if diff.is_harness() || diff.class == "not requested" {
        return diff;
    }
    let paths: BTreeSet<&str> = diff.blocks.iter().map(|(p, _)| p.as_str()).collect();
    let about = |keys: &BTreeSet<String>| diff.keys.iter().any(|k| keys.contains(k));
    let reclaimed_active = paths
        .iter()
        .any(|p| facts.reclaimed.contains_key(*p) && !facts.losing.contains(*p));
    // Which file holds a claim: the active-file, owner and count rows name
    // the holder's path in their detail.
    let holder_row = ["active files", "owners", "counts"].contains(&diff.class.as_str());
    let class = if holder_row && facts.journal_first.iter().any(|p| diff.detail.contains(p)) {
        "K3 (§2.2 journals directory first)"
    } else if about(&facts.claim_keys) || reclaimed_active {
        "K1 (§2.1 file claim)"
    } else if paths.iter().any(|p| facts.losing.contains(*p)) {
        "K2 (§2.2 inactive claimant)"
    } else if diff.blocks.iter().any(|b| facts.claim_blocks.contains(b))
        || about(&facts.claim_component)
    {
        "K1 (§2.1 file claim, reference)"
    } else if about(&facts.losing_keys) {
        "K2 (§2.2 inactive claimant)"
    } else if about(&facts.reclaimed_keys) {
        "K1 (§2.1 file claim, contributed name)"
    } else {
        "FINDING"
    };
    diff.class = format!("{class}: {}", diff.class);
    diff
}

#[test]
fn attribution_names_a_class_only_for_differences_that_involve_it() {
    let set = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
    let page = |k: &str| ("page".to_owned(), k.to_owned());
    let facts = Facts {
        reclaimed: BTreeMap::from([(
            "pages/2026-10-26.md".to_owned(),
            (
                Some(page("2026-10-26")),
                ("journal".to_owned(), "Day 2026-10-26".to_owned()),
            ),
        )]),
        losing: set(&["pages/2026-10-26.md"]),
        journal_first: set(&["journals/2026_10_26.md"]),
        claim_keys: set(&["2026-10-26", "day 2026-10-26"]),
        claim_component: set(&["2026-10-26", "day 2026-10-26", "b"]),
        claim_blocks: BTreeSet::from([("pages/R.md".to_owned(), 0)]),
        reclaimed_keys: set(&["2026-10-26", "a"]),
        losing_keys: set(&["2026-10-26", "a"]),
    };
    let class = |d: Diff| attribute(&facts, d).class;
    // Unrelated keys and blocks stay unexplained: the test fails on them.
    assert_eq!(
        class(Diff::new("pages", "x").at_key("other")),
        "FINDING: pages"
    );
    assert_eq!(
        class(Diff::new("linked refs", "x").at_block(&("pages/Q.md".to_owned(), 1))),
        "FINDING: linked refs"
    );
    assert!(
        class(Diff::new("pages", "x").at_key("2026-10-26")).starts_with("K1 (§2.1 file claim):")
    );
    assert!(class(Diff::new("pages", "x").at_key("a")).starts_with("K2 "));
    assert!(
        class(Diff::new("linked refs", "x").at_block(&("pages/2026-10-26.md".to_owned(), 1)))
            .starts_with("K2 ")
    );
    assert!(
        class(Diff::new("linked refs", "x").at_block(&("pages/R.md".to_owned(), 0)))
            .starts_with("K1 (§2.1 file claim, reference)")
    );
    // The holder rule applies only to rows that name a holder's path.
    assert!(class(Diff::new(
        "active files",
        "journals/2026_10_26.md: model=true, Tine=false"
    ))
    .starts_with("K3 "));
    assert_eq!(
        class(Diff::new("pages", "journals/2026_10_26.md").at_key("other")),
        "FINDING: pages"
    );
    // Harness differences are never attributed.
    assert_eq!(
        class(Diff::new("harness: nonLocal", "x").at_key("2026-10-26")),
        "harness: nonLocal"
    );
}

/// The graphs the static comparison runs: the root fixtures, the typed
/// boundary graphs of `r13/cases.json` (their graphs only: the expectations
/// recorded there are an older model's answers) and `n` seeded random graphs.
pub fn corpus(roots: Vec<GraphSpec>, manifest: &str, n: u64) -> Vec<GraphSpec> {
    let path = fixture_dir(manifest).join("r13/cases.json");
    let cases: Vec<Value> = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    roots
        .into_iter()
        .chain(
            cases
                .iter()
                .map(|v| GraphSpec::from_fixture(v["name"].as_str().unwrap(), v)),
        )
        .chain((0..n).map(random_graph))
        .collect()
}

/// The static comparison against the confirmed model. `links` reads Tine's
/// link table (crate private in the store) as typed model pairs.
pub fn store_conformance(
    bridge: &Path,
    roots: Vec<GraphSpec>,
    manifest: &str,
    links: impl Fn(&WholeGraph, &GraphSpec, &Value, &mut Vec<Diff>) -> BTreeSet<(Page, Page)>,
    mut consumers: impl FnMut(
        &super::super::store_crate::Store,
        &WholeGraph,
        &GraphSpec,
        &Value,
        &Value,
    ) -> Vec<Diff>,
) {
    let mut section = Section::default();
    let mut report = BTreeMap::new();
    let n = std::env::var("CM_RANDOM")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    let specs = corpus(roots, manifest, n);
    let total = specs.len();
    let mut runs = 0;
    let mut closers = Vec::new();
    for spec in specs {
        let dir = tempfile::tempdir().unwrap();
        spec.write(dir.path());
        let store = super::super::store_crate::Store::open(dir.path(), Default::default())
            .unwrap()
            .0;
        let view = store.whole_graph().unwrap();
        let input = model_input(&spec);
        let model = run_bridge(bridge, &input)
            .unwrap_or_else(|e| panic!("{}: the confirmed-model bridge failed: {e}", spec.name));
        assert_eq!(
            model["round"], "k",
            "{}: the static comparison runs against the confirmed model",
            spec.name
        );
        let mut found = compare(&view, &spec, &input, &model);
        found.extend(consumers(&store, &view, &spec, &input, &model));
        let mut link_diffs = Vec::new();
        let actual = links(&view, &spec, &input, &mut link_diffs);
        found.extend(link_diffs);
        found.extend(compare_links(&actual, &model));
        let facts = facts(&view, &spec.journal_format(), &input, &model);
        let mut diffs: Vec<_> = found.into_iter().map(|d| attribute(&facts, d)).collect();
        diffs.sort();
        diffs.dedup();
        assert!(
            !diffs.iter().any(Diff::is_harness),
            "{}: {diffs:?}",
            spec.name
        );
        report.insert(spec.name.clone(), json!({"differences": diffs.iter().map(|d| json!({"class": d.class, "detail": d.detail, "keys": d.keys, "blocks": d.blocks})).collect::<Vec<_>>(),
            "input": input, "model": model}));
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
    assert!(
        runs == total && runs as u64 > n,
        "the confirmed-model comparison ran {runs} of {total} graphs"
    );
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
        "differences from the confirmed model that no class explains: {findings:#?}"
    );
    check_expected(
        manifest,
        "storeK",
        &section,
        "CM_REFRESH=1 CM_STATIC_BRIDGE=<cm-bridge-k> cargo test -p tine-store --lib conceptual_model_conformance",
    );
    eprintln!("confirmed-model bridge executed {runs} graphs; zero skipped");
}
