//! Live equals fresh under real delivery (storage conformance audit
//! 2026-10-19, F2 and F5). Seeded mixed sequences drive a real `Store` and a
//! real `PageHost` (its driver thread, publication consumer and the store's
//! watcher in Notify or Poll mode): window opens, submits, keep-mine,
//! discards and closes; host renames (with referrers) and deletes; retained
//! transactions under a reservation; outside writers rewriting, creating and
//! deleting held and unheld pages and journals and editing `config.edn`
//! (journal formats); duplicate title claimants; failed index publications
//! and their retries. Each step runs one to three actions without a barrier
//! between them, then settles: the watcher drained (every notification of a
//! write so far reconciled) and the host settled (no queued, applying or
//! saving work, no timed work due, no owed observation, unread release or
//! publication retry, each held page clean unless a page is in conflict),
//! repeated until a whole round leaves
//! the driver's wake sequence unchanged.
//!
//! Then the image of the live publication, built incrementally from the
//! one before (whose lazy indexes the previous image forced), must equal the
//! image of a store opened cold on a copy of the files. The image is
//! [`snapshot_image`] (projections, raws, preblocks, references, signatures,
//! aliases, name keys and owners, block hints and counts, icons, old-DSL
//! queries, backlinks, unlinked references) plus the listing, name
//! resolution (claimants), journal days, config, asset references, block
//! referrers, explicit referrers and TQL queries including `links`,
//! `linked_by` and `near`.
//!
//! The reference is the files, each owned page's file replaced by its
//! owner's last published bytes: a held page's index is that publication,
//! not disk (contract, invariant (G)). Unless a page is in conflict, the
//! barrier also waits until those bytes, and each clean held page's buffer,
//! equal the file (the host observes every change). A dirty host's unsaved
//! buffer is never in an index. Listing names, duplicate-day journal strays
//! and icons of names with several file claimants are compared too.
//!
//! `TINE_CONFORMANCE_SEEDS=<n>` (or `a..b`, or a comma list) runs more
//! seeds; `TINE_CONFORMANCE_STEPS=<n>` longer sequences.
use super::*;
use crate::model::index_oracle_tests::{first_difference, snapshot_image};
use std::collections::BTreeMap;

struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn below(&mut self, n: usize) -> usize {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x % n as u64) as usize
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

const PAGES: &[&str] = &["Alpha", "Beta", "Gamma", "Delta", "中文"];
const ALIASES: &[&str] = &["al", "AL", "beta alias", "汉字", "Gamma"];
const TAGS: &[&str] = &["t0", "t1"];
const ICONS: &[&str] = &["🚀", "📚"];
const UUIDS: &[&str] = &[
    "64d7c5e0-0000-4000-8000-00000000000a",
    "64d7c5e0-0000-4000-8000-00000000000b",
    "64d7c5e0-0000-4000-8000-00000000000c",
];
const ASSETS: &[&str] = &["pic", "doc"];
const CONFIGS: &[&str] = &[
    "{}\n",
    "{:journal/page-title-format \"yyyy-MM-dd\"}\n",
    "{:journal/file-name-format \"yyyy-MM-dd\"}\n",
    "{:journal/page-title-format \"EEEE, dd-MM-yyyy\" :preferred-format :org}\n",
];
/// Names every image asks about: pages, aliases, journal titles in each
/// configured format, a namespace and names no file has.
const PROBES: &[&str] = &[
    "Alpha",
    "Beta",
    "Gamma",
    "Delta",
    "中文",
    "al",
    "beta alias",
    "汉字",
    "ns/leaf",
    "ns",
    "Oct 1st, 2026",
    "2026-10-02",
    "Ren1",
    "Ren2",
    "New1",
    "Nowhere",
];
const QUERIES: &[&str] = &[
    "(task TODO)",
    "(task TODO DONE)",
    "(page-tags t0)",
    "(property status open)",
    "(page-property icon)",
    "[[Alpha]]",
    "[[al]]",
    "[[中文]]",
    "(and (task TODO) [[Beta]])",
    "(or [[Gamma]] #t1)",
    "(content shipped)",
];
const TQL: &[&str] = &[
    "@page and any(links, name = 'alpha')",
    "@page and any(linked_by, name = 'beta')",
    "@page and near(name = 'alpha', 1, 'in')",
    "@page and near(name = 'gamma', 2)",
    "@page and near(name = 'ns/leaf', 1, 'in')",
    "@page and none(linked_by, true)",
    "@page and any(links, journal = true)",
    "@page and journal = true",
    "@block and task = 'TODO'",
];

/// One block of a page, top-level, sometimes with a child.
fn block(r: &mut Rng) -> String {
    let p = r.pick(PAGES);
    let uuid = r.pick(UUIDS);
    match r.below(12) {
        0 => format!("- TODO work on [[{p}]] #{}\n", r.pick(TAGS)),
        1 => format!("- DONE [[{p}]] shipped\n  SCHEDULED: <2026-10-02 Fri>\n"),
        2 => format!("- plain mention of {p} without a link\n"),
        3 => format!("- see (({uuid}))\n"),
        4 => format!("- anchor block\n  id:: {uuid}\n"),
        5 => format!("- ![{0}](../assets/{0}.png)\n", r.pick(ASSETS)),
        6 => "- [[ns/leaf]] and [[Oct 1st, 2026]] and [[2026-10-02]]\n".into(),
        7 => format!("- task\n  status:: {}\n", ["open", "closed"][r.below(2)]),
        8 => format!("- alias ref [[{}]]\n", r.pick(ALIASES)),
        9 => format!("- parent [[{p}]]\n  - child #{}\n", r.pick(TAGS)),
        10 => format!("- [[{p}]] then [[{}]]\n", r.pick(PAGES)),
        _ => "- just text\n".into(),
    }
}

/// A Markdown page body; `page` adds the properties only a page carries.
fn body(r: &mut Rng, page: bool) -> String {
    let mut pre = Vec::new();
    if r.below(3) == 0 {
        pre.push(format!("alias:: {}", r.pick(ALIASES)));
    }
    if r.below(4) == 0 {
        pre.push(format!("tags:: {}", r.pick(TAGS)));
    }
    if page && r.below(5) == 0 {
        pre.push(format!("icon:: {}", r.pick(ICONS)));
    }
    if page && r.below(8) == 0 {
        // A second claimant of an existing page's name.
        pre.push(format!("title:: {}", r.pick(PAGES)));
    }
    let mut out = String::new();
    if !pre.is_empty() {
        out.push_str(&pre.join("\n"));
        out.push_str("\n\n");
    }
    for _ in 0..1 + r.below(3) {
        out.push_str(&block(r));
    }
    out
}

fn org_body(r: &mut Rng) -> String {
    format!(
        "#+title: Org Notes\n* TODO [[{}]] :{}:\n** child [[{}]]\n",
        r.pick(PAGES),
        r.pick(TAGS),
        r.pick(ALIASES)
    )
}

/// Graph text files on disk, relative and sorted.
fn files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for area in ["pages", "journals"] {
        let Ok(entries) = fs::read_dir(root.join(area)) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".md") || name.ends_with(".org") {
                out.push(format!("{area}/{name}"));
            }
        }
    }
    out.sort();
    out
}

fn copy_graph(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".tine-watch-drain-") {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type().unwrap().is_dir() {
            copy_graph(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// One query through the IR front door, the whole result serialized.
fn tql(view: &crate::WholeGraph, source: &str) -> String {
    use tine_core::query::{parse_query_input, QueryInput};
    let (query, plan) = parse_query_input(
        source,
        QueryInput::Tql,
        tine_core::date::JournalDate::today(),
        tine_core::query::registry::Registry::none(),
    );
    let answer = view
        .query_ir(
            crate::IrRequest::Run {
                query: &query,
                view: &plan,
                context: &tine_core::query::ir::ExecutionContext::default(),
            },
            &crate::Cancel::default(),
        )
        .unwrap()
        .expect("never cancelled");
    match answer {
        crate::IrAnswer::Result(result) => serde_json::to_string(&result).unwrap(),
        _ => panic!("query_ir(Run) returns a result"),
    }
}

fn resolved(resolved: crate::Resolved) -> String {
    let ids = |ids: &[PageId]| ids.iter().map(PageId::as_str).collect::<Vec<_>>().join(",");
    match resolved {
        crate::Resolved::Existing { id, others } => {
            format!("existing {} others [{}]", id.as_str(), ids(&others))
        }
        crate::Resolved::Alias { owners } => format!("alias [{}]", ids(&owners)),
        crate::Resolved::Absent { id } => format!("absent {}", id.as_str()),
    }
}

/// The view's whole image: [`snapshot_image`] and the answers it does not
/// ask. Only `view`'s own state: an older view must keep it.
fn view_image(view: &crate::WholeGraph) -> String {
    let mut out = snapshot_image(&view.graph, PROBES);
    let mut listing: Vec<_> = (view.list.iter())
        .map(|entry| format!("{:?} {:?} {:?}", entry.rel_path, entry.kind, entry.name))
        .collect();
    listing.sort();
    for line in listing {
        out.push_str(&format!("list {line}\n"));
    }
    let mut names: Vec<String> = view.list.iter().map(|entry| entry.name.clone()).collect();
    names.extend(PROBES.iter().map(|name| (*name).to_owned()));
    names.sort();
    names.dedup();
    for name in &names {
        for journal in [false, true] {
            out.push_str(&format!(
                "resolve {name:?} {journal} {}\n",
                resolved(view.resolve(name, journal))
            ));
        }
        out.push_str(&format!(
            "explicit_referrers {name:?} {:?}\n",
            view.explicit_referrers(std::slice::from_ref(name))
        ));
    }
    // In page-slot order, which carries no meaning (the calendar marks a
    // set of days).
    let mut days: Vec<_> = view
        .journal_content_days()
        .iter()
        .map(|day| day.0)
        .collect();
    days.sort();
    out.push_str(&format!("journal_days {days:?}\n"));
    out.push_str(&format!("config {:?}\n", view.config.config));
    let mut assets: Vec<_> = view.referenced_assets().iter().cloned().collect();
    assets.sort();
    out.push_str(&format!("assets {assets:?}\n"));
    for uuid in UUIDS {
        // At most two pages, the first in page-slot order: two means
        // "ambiguous", whichever two (`owner/write.rs` refuses either way).
        let mut pages = view.block_pages(uuid);
        pages.sort();
        if pages.len() == 2 {
            pages = vec!["<ambiguous>".into()];
        }
        out.push_str(&format!(
            "block {uuid} pages {pages:?} referrers {}\n",
            serde_json::to_string(view.block_referrers(uuid).unwrap().as_ref()).unwrap()
        ));
    }
    let mut counts: Vec<_> = view
        .block_ref_counts()
        .as_ref()
        .clone()
        .into_iter()
        .collect();
    counts.sort();
    out.push_str(&format!("view block_ref_counts {counts:?}\n"));
    let mut icons: Vec<_> = view.page_icons(&names).into_iter().collect();
    icons.sort();
    out.push_str(&format!("view icons {icons:?}\n"));
    out.push_str(&format!("unreadable {:?}\n", view.unreadable_files()));
    for query in QUERIES {
        let (rows, bytes) = (
            crate::store::RESULT_BRIDGE_MAX_ROWS,
            crate::store::RESULT_BRIDGE_MAX_BYTES,
        );
        let result = view.graph.run_query_bounded(query, rows, bytes);
        out.push_str(&format!(
            "dsl {query:?} {} {} {}\n",
            result.total,
            result.exceeded,
            serde_json::to_string(result.groups.as_ref()).unwrap()
        ));
    }
    for query in TQL {
        out.push_str(&format!("tql {query:?} {}\n", tql(view, query)));
    }
    out
}

/// The store's image: its current view's, then the live cache's own
/// listing and journal order. Absolute paths read `<root>`.
fn store_image(store: &Store) -> (crate::WholeGraph, String) {
    let view = store.whole_graph().unwrap();
    let mut out = view_image(&view);
    let mut listing: Vec<_> = (store.graph.list_pages().iter())
        .map(|entry| {
            format!(
                "{:?} {:?} {:?} {:?}",
                entry.rel_path, entry.kind, entry.date_key, entry.name
            )
        })
        .collect();
    listing.sort();
    for line in listing {
        out.push_str(&format!("cache list {line}\n"));
    }
    let journals: Vec<_> = (store.graph.journals_desc().into_iter())
        .map(|entry| entry.rel_path)
        .collect();
    out.push_str(&format!("cache journals_desc {journals:?}\n"));
    (view, normalized(&out, &store.graph.root))
}

/// Absolute paths read `<root>`, in the spelling the image holds them: plain
/// (`{}`) and `Debug`-escaped (`{:?}` of a path string, where a Windows
/// separator is a doubled backslash). The referenced-name index keeps the first
/// case variant of a name it met, which differs between a patched
/// generation and a rebuilt one (`random_alias_edits_equal_a_fresh_build`):
/// those lines compare case-folded.
fn normalized(image: &str, root: &Path) -> String {
    let plain = root.to_string_lossy();
    let image = image
        .replace(&*plain, "<root>")
        .replace(&plain.replace('\\', "\\\\"), "<root>");
    let mut out = String::with_capacity(image.len());
    for line in image.lines() {
        let folded = [
            "lazy referenced_page_names ",
            "lazy referenced_index ",
            "lazy editor ",
        ]
        .iter()
        .any(|prefix| line.starts_with(prefix));
        match (folded, line.find('['), line.rfind(']')) {
            (true, Some(open), Some(close)) if open < close => {
                let mut names: Vec<_> = (line[open + 1..close].split(", "))
                    .map(str::to_lowercase)
                    .collect();
                names.sort();
                out.push_str(&format!(
                    "{}[{}]{}",
                    &line[..open],
                    names.join(", "),
                    &line[close + 1..]
                ));
            }
            _ => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

/// What one run counted, so a run is never vacuously green.
#[derive(Default, Debug)]
struct Counts {
    actions: BTreeMap<&'static str, usize>,
    checks: usize,
    held_at_check: usize,
    conflicted_at_check: usize,
}

struct Run<'a> {
    live: &'a Live,
    r: Rng,
    /// Pages the window has open (a closed dirty one may stay held).
    open: Vec<PageKey>,
    created: usize,
    renamed: usize,
    log: Vec<String>,
    counts: Counts,
}

impl Run<'_> {
    fn note(&mut self, action: &'static str, what: String) {
        *self.counts.actions.entry(action).or_default() += 1;
        self.log.push(what);
    }

    fn page(&self, key: &str) -> Option<(u64, bool, Text)> {
        let state = self.live.host.driver.shared.state_for_tests();
        let page = state.progress.host.pages.get(key)?;
        Some((page.version, page.conflict, page.obs.clone().flatten()))
    }

    fn disk_files(&self) -> Vec<String> {
        files(&self.live.root)
    }

    fn act(&mut self) {
        let live = self.live;
        let root = live.root.clone();
        match self.r.below(25) {
            0..=2 => {
                let candidates: Vec<_> = (self.disk_files().into_iter())
                    .filter(|rel| !self.open.contains(rel) && rel.ends_with(".md"))
                    .collect();
                if candidates.is_empty() {
                    return;
                }
                let rel = self.r.pick(&candidates).clone();
                let (key, page) = live.open(&rel);
                self.note(
                    "open",
                    format!("open {key} v{} conflict={}", page.version, page.conflict),
                );
                self.open.push(key);
            }
            3..=7 if !self.open.is_empty() => {
                let key = self.r.pick(&self.open).clone();
                let Some((version, conflict, obs)) = self.page(&key) else {
                    return;
                };
                let text = body(&mut self.r, key.starts_with("pages/"));
                let keep = conflict && self.r.below(2) == 0;
                // Input typed on an older version: the host's stale-input
                // conflict.
                let stale = !keep && version > 1 && self.r.below(5) == 0;
                let resolve = keep.then(|| DiskToken::of(&obs));
                let based = if stale { version - 1 } else { version };
                let id = live.submit(&key, &text, based, resolve.as_ref());
                let outcome = id.map(|id| live.answer(&key, id).answer.map(|a| a.version));
                let action = match (keep, stale) {
                    (true, _) => "keep mine",
                    (_, true) => "stale submit",
                    _ => "submit",
                };
                self.note(
                    action,
                    format!("{action} {key} v{version} {text:?}: {outcome:?}"),
                );
            }
            8 if !self.open.is_empty() => {
                let key = self.r.pick(&self.open).clone();
                let generation = live.host.session();
                let outcome = live.host.discard(generation, live.id(), &key);
                self.note("discard", format!("discard {key}: {outcome:?}"));
            }
            9 if !self.open.is_empty() => {
                let index = self.r.below(self.open.len());
                let key = self.open.remove(index);
                live.close(&key);
                self.note("close", format!("close {key}"));
            }
            10..=12 | 23 | 24 => {
                let mut all = self.disk_files();
                if !self.open.is_empty() && self.r.below(2) == 0 {
                    // An outside writer racing the window's typing.
                    all.retain(|rel| self.open.contains(rel));
                }
                if all.is_empty() {
                    return;
                }
                let rel = self.r.pick(&all).clone();
                let text = if rel.ends_with(".org") {
                    org_body(&mut self.r)
                } else {
                    body(&mut self.r, rel.starts_with("pages/"))
                };
                let held = self.page(&rel).is_some();
                if self.r.below(2) == 0 {
                    fs::write(root.join(&rel), &text).unwrap();
                } else {
                    // An atomic replace, as sync tools and editors write.
                    let temp = root.join(format!(".outside-{}", self.log.len()));
                    fs::write(&temp, &text).unwrap();
                    fs::rename(&temp, root.join(&rel)).unwrap();
                }
                self.note(
                    "external rewrite",
                    format!("external rewrite {rel} (held={held}) {text:?}"),
                );
            }
            13 => {
                self.created += 1;
                let n = self.created;
                let day = 3 + n % 26;
                let rel = match self.r.below(3) {
                    0 => format!("journals/2026_10_{day:02}.md"),
                    1 => format!("journals/2026-10-{day:02}.md"),
                    _ => format!("pages/New{n}.md"),
                };
                if root.join(&rel).exists() {
                    return;
                }
                let text = body(&mut self.r, rel.starts_with("pages/"));
                fs::write(root.join(&rel), &text).unwrap();
                self.note("external create", format!("external create {rel} {text:?}"));
            }
            14 => {
                let all = self.disk_files();
                if all.len() <= 3 {
                    return;
                }
                let rel = self.r.pick(&all).clone();
                let held = self.page(&rel).is_some();
                fs::remove_file(root.join(&rel)).unwrap();
                self.note(
                    "external delete",
                    format!("external delete {rel} (held={held})"),
                );
            }
            15 => {
                let all: Vec<_> = (self.disk_files().into_iter())
                    .filter(|rel| rel.starts_with("pages/") && rel.ends_with(".md"))
                    .collect();
                if all.len() <= 2 {
                    return;
                }
                let rel = self.r.pick(&all).clone();
                let Ok(checked) = fs::read(root.join(&rel)) else {
                    return;
                };
                let deadline = Instant::now() + Duration::from_secs(3);
                let outcome = loop {
                    let outcome = live.host.delete(
                        live.host.session(),
                        &PageId::from(rel.as_str()),
                        Some(&checked[..]),
                    );
                    if outcome != PageOperation::Waiting || Instant::now() > deadline {
                        break outcome;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                };
                if outcome == PageOperation::Applied {
                    self.open.retain(|key| *key != rel);
                }
                self.note("host delete", format!("host delete {rel}: {outcome:?}"));
            }
            16 | 17 => self.rename(),
            18 | 19 => {
                let all = self.disk_files();
                if all.is_empty() {
                    return;
                }
                let rel = self.r.pick(&all).clone();
                let id = PageId::from(rel.as_str());
                let text = if rel.ends_with(".org") {
                    org_body(&mut self.r)
                } else {
                    body(&mut self.r, rel.starts_with("pages/"))
                };
                let reserved = live.host.reserve(|| vec![id.clone()], Input::Flush);
                let outcome = match &reserved {
                    Ok(_) => match fs::read(root.join(&rel)) {
                        Ok(old) => {
                            let dto = live.dto(&rel, &text);
                            let mut tx = live.store.transaction(None);
                            let base = crate::SaveBase::Existing(FileRev::from_bytes(&old));
                            tx.save_page(&[EditKind::SaveBlock], &id, base, &dto);
                            format!("{:?}", tx.commit())
                        }
                        Err(error) => format!("no file: {error}"),
                    },
                    Err(refusal) => format!("reservation refused: {refusal:?}"),
                };
                drop(reserved);
                self.note(
                    "retained transaction",
                    format!("retained transaction {rel} {text:?}: {outcome}"),
                );
            }
            20 => {
                let n = 1 + self.r.below(2) as u32;
                live.index_faults(n);
                self.note(
                    "index faults",
                    format!("fail the next {n} index publications"),
                );
            }
            21 => {
                let config = *self.r.pick(CONFIGS);
                fs::create_dir_all(root.join("logseq")).unwrap();
                fs::write(root.join("logseq/config.edn"), config).unwrap();
                self.note("config edit", format!("config.edn {config:?}"));
            }
            _ => {
                if root.join("pages/Org.org").exists() {
                    return;
                }
                let text = org_body(&mut self.r);
                fs::write(root.join("pages/Org.org"), &text).unwrap();
                self.note(
                    "external create",
                    format!("external create pages/Org.org {text:?}"),
                );
            }
        }
    }

    /// A host rename of a page named by its file, with the referrers its
    /// view finds, replanned in a new view when another publication made it
    /// stale (B2), as `pages.rs` drives it.
    fn rename(&mut self) {
        let live = self.live;
        let candidates: Vec<_> = (self.disk_files().into_iter())
            .filter(|rel| rel.starts_with("pages/") && rel.ends_with(".md"))
            .filter(|rel| {
                fs::read_to_string(live.root.join(rel)).is_ok_and(|text| !text.contains("title::"))
            })
            .collect();
        if candidates.is_empty() {
            return;
        }
        let rel = self.r.pick(&candidates).clone();
        let name = rel["pages/".len()..rel.len() - ".md".len()].to_owned();
        self.renamed += 1;
        let new = format!("Ren{}", self.renamed);
        let source = PageId::from(rel.as_str());
        let target = PageId::from(format!("pages/{new}.md"));
        let map = crate::RenameMap(vec![(name.clone(), new.clone())]);
        let mut outcome = Err(RenameRefusal::Refused);
        for _ in 0..20 {
            let view = live.store.whole_graph().unwrap();
            let mut referrers = view.explicit_referrers(std::slice::from_ref(&name));
            referrers.retain(|id| *id != source);
            outcome = live.host.rename(
                &source,
                &target,
                &referrers,
                &map,
                &view,
                Arc::new(|_: &PageId, _: &[u8], _| None),
            );
            if outcome != Err(RenameRefusal::Refused) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if outcome.is_ok() {
            self.open.retain(|key| *key != rel);
        }
        self.note(
            "host rename",
            format!("host rename {rel} -> {}: {outcome:?}", target.as_str()),
        );
    }

    /// The barrier: watcher drained and host settled, until a whole round
    /// leaves the driver's wake sequence unchanged.
    fn settle(&self) {
        let live = self.live;
        let deadline = Instant::now() + Duration::from_secs(60);
        let left = || deadline.saturating_duration_since(Instant::now());
        let mut last = None;
        loop {
            assert!(
                live.store.watcher_drained_for_tests(left()),
                "the watcher never drained\n{}",
                self.log.join("\n")
            );
            loop {
                let (unsettled, wake) = {
                    let state = live.host.driver.shared.state_for_tests();
                    let host = &state.progress.host;
                    let mut unsettled = Vec::new();
                    if host.job.is_some() || host.worker.is_some() {
                        unsettled.push(format!(
                            "job {:?} worker {}",
                            host.job,
                            host.worker.is_some()
                        ));
                    }
                    if !host.queue.is_empty() || host.applying.is_some() {
                        unsettled.push(format!(
                            "queue {:?} applying {:?}",
                            host.queue, host.applying
                        ));
                    }
                    if !state.observe.is_empty() {
                        unsettled.push(format!("observe {:?}", state.observe));
                    }
                    // Timed work (a debounced save, a draft refresh): work
                    // a conflict blocks contributes no deadline.
                    if let Some(due) = state.progress.next_deadline() {
                        unsettled.push(format!("timed work due at {due} ms"));
                    }
                    // A retained transaction's index stands, and later
                    // publications of its page wait, until the host has
                    // read the released bytes (V2).
                    if !state.book.handover.is_empty() {
                        unsettled.push(format!("handover {:?}", state.book.handover));
                    }
                    if !state.book.retry.is_empty() || state.book.delivering {
                        let retry: Vec<_> = state.book.retry.keys().collect();
                        unsettled.push(format!(
                            "publication retry {retry:?} delivering {}",
                            state.book.delivering
                        ));
                    }
                    // An admitted operation keeps custody of its pages while
                    // one of them is in conflict, and completes once it can be
                    // written (`RenameRefusal::Unwritten`): with a conflict, a
                    // dirty page may wait for the user, and the pages in the
                    // operation's custody are observed only after it.
                    if !host.pages.values().any(|page| page.conflict) {
                        for (key, page) in &host.pages {
                            if !page.clean() {
                                unsettled.push(format!("dirty {key}: {page:?}"));
                            }
                        }
                        unsettled.extend(self.lags(&state.book, &host.pages));
                    }
                    (unsettled, state.wake_seq())
                };
                let settled = unsettled.is_empty();
                if settled {
                    if last == Some(wake) {
                        return;
                    }
                    last = Some(wake);
                    break;
                }
                assert!(
                    !left().is_zero(),
                    "the host never settled: {}\n--- log\n{}",
                    unsettled.join("\n"),
                    self.log.join("\n")
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    /// Where the host's pages and their index lag the files: an owned
    /// page's last published bytes, or a clean held page's buffer, that are
    /// not its file's. Unless a conflict holds an operation, the barrier
    /// waits until there are none (the host observes every change).
    fn lags(&self, book: &Book, pages: &BTreeMap<PageKey, crate::page_host::Page>) -> Vec<String> {
        let disk = |key: &str| fs::read(self.live.root.join(key)).ok();
        let text = |bytes: Option<&[u8]>| bytes.map(|b| String::from_utf8_lossy(b).into_owned());
        let mut lags = Vec::new();
        for (key, (bytes, _)) in &book.index {
            let file = disk(key);
            if bytes.as_deref() != file.as_deref() {
                lags.push(format!(
                    "{key}: published {:?}, disk {:?}",
                    text(bytes.as_deref()),
                    text(file.as_deref())
                ));
            }
        }
        for (key, page) in pages {
            let file = disk(key);
            if page.clean() && page.buf.as_deref() != file.as_deref() {
                lags.push(format!(
                    "{key}: clean buffer {:?}, disk {:?}",
                    text(page.buf.as_deref()),
                    text(file.as_deref())
                ));
            }
        }
        lags
    }

    /// Each held page's owner's last published bytes ((G): its index). The
    /// book keeps a row for a page no longer held until it evicts the key;
    /// while a conflict holds the host, that row can predate a retained
    /// transaction's write, whose index stands (V2): an unheld page's
    /// reference is its file.
    fn owned(&self) -> BTreeMap<String, Text> {
        let state = self.live.host.driver.shared.state_for_tests();
        (state.book.index.iter())
            .filter(|(key, _)| state.progress.host.pages.contains_key(*key))
            .map(|(key, (bytes, _))| (key.to_string(), bytes.clone()))
            .collect()
    }

    /// For a failure: each held page's state and whether its owner
    /// published bytes, and which equal its file.
    fn host_summary(&self, owned: &BTreeMap<String, Text>) -> String {
        let state = self.live.host.driver.shared.state_for_tests();
        let disk = |key: &str| fs::read(self.live.root.join(key)).ok();
        let mut out = String::from("held pages:");
        for (key, page) in &state.progress.host.pages {
            out.push_str(&format!(
                "\n  {key}: clean {} conflict {} typed {} buf=disk {} owned {:?}",
                page.clean(),
                page.conflict,
                page.typed,
                page.buf.as_deref() == disk(key).as_deref(),
                owned
                    .get(key.as_str())
                    .map(|bytes| bytes.as_deref() == disk(key).as_deref()),
            ));
        }
        let unheld: Vec<_> = (state.book.index.keys())
            .filter(|key| !state.progress.host.pages.contains_key(*key))
            .collect();
        out.push_str(&format!("\nowned, not held: {unheld:?}"));
        out.push_str(&format!("\nhandover: {:?}", state.book.handover));
        out
    }

    fn held(&self) -> (usize, usize) {
        let state = self.live.host.driver.shared.state_for_tests();
        let pages = &state.progress.host.pages;
        (pages.len(), pages.values().filter(|p| p.conflict).count())
    }
}

/// The image of a store opened cold on a copy of `root`'s files, each owned
/// page's file replaced by its owner's published bytes (`None`: no file).
fn fresh_image(root: &Path, owned: &BTreeMap<String, Text>) -> String {
    let copy = tempfile::tempdir().unwrap();
    let graph = copy.path().join("graph");
    copy_graph(root, &graph);
    for (key, bytes) in owned {
        match bytes {
            Some(bytes) => fs::write(graph.join(key), &bytes[..]).unwrap(),
            None => {
                let _ = fs::remove_file(graph.join(key));
            }
        }
    }
    let store = Store::open(&graph, Default::default()).unwrap().0;
    store.whole_graph_reconciled().unwrap();
    let (view, image) = store_image(&store);
    drop(view);
    store.close();
    image
}

fn initial_files(r: &mut Rng) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = PAGES
        .iter()
        .map(|name| (format!("pages/{name}.md"), body(r, true)))
        .collect();
    for day in ["2026_10_01", "2026_10_02"] {
        files.push((format!("journals/{day}.md"), body(r, false)));
    }
    files.push(("pages/Org.org".into(), org_body(r)));
    files.push(("logseq/config.edn".into(), CONFIGS[0].into()));
    files
}

fn run_seed(seed: u64, steps: usize, watch: crate::WatchMode) -> Counts {
    let mut r = Rng::new(seed);
    let files = initial_files(&mut r);
    let borrowed: Vec<(&str, &str)> = (files.iter())
        .map(|(rel, text)| (rel.as_str(), text.as_str()))
        .collect();
    let live = Live::watching(&borrowed, watch);
    if seed % 2 == 0 {
        // Window submits keep the edit debounce: typing stays unsaved
        // while outside writers and operations interleave.
        live.submits_wait();
    }
    let mut run = Run {
        live: &live,
        r,
        open: Vec::new(),
        created: 0,
        renamed: 0,
        log: vec![format!("seed {seed} {watch:?}")],
        counts: Counts::default(),
    };
    let mut previous: Option<(crate::WholeGraph, String)> = None;
    for step in 0..=steps {
        if step > 0 {
            run.log.push(format!("-- step {step}"));
            for _ in 0..1 + run.r.below(3) {
                run.act();
            }
        }
        run.settle();
        let label = format!("seed {seed} {watch:?} step {step}");
        let (held, conflicted) = run.held();
        run.counts.held_at_check += held;
        run.counts.conflicted_at_check += conflicted;
        let (view, live_image) = store_image(&live.store);
        let owned = run.owned();
        let fresh = fresh_image(&live.root, &owned);
        if let Some(difference) = first_difference(&live_image, &fresh) {
            panic!(
                "{label}: live vs a store opened on the same files (owned pages: their \
                 published bytes): {difference}\n{}\n--- log\n{}",
                run.host_summary(&owned),
                run.log.join("\n")
            );
        }
        if let Some((old, image)) = previous.take() {
            let again = normalized(&view_image(&old), &live.root);
            let before: String = (image.lines())
                .filter(|line| !line.starts_with("cache "))
                .flat_map(|line| [line, "\n"])
                .collect();
            if let Some(difference) = first_difference(&again, &before) {
                panic!(
                    "{label}: the previous view changed after later publications: {difference}\n--- log\n{}",
                    run.log.join("\n")
                );
            }
        }
        previous = Some((view, live_image));
        run.counts.checks += 1;
    }
    drop(previous);
    let counts = std::mem::take(&mut run.counts);
    drop(run);
    live.index_faults(0);
    live.host.stop();
    counts
}

/// Seeds from `TINE_CONFORMANCE_SEEDS` (`n`, `a..b` or `a,b,c`), else
/// `default`.
fn seeds(default: &[u64]) -> Vec<u64> {
    let Ok(spec) = std::env::var("TINE_CONFORMANCE_SEEDS") else {
        return default.to_vec();
    };
    if let Some((a, b)) = spec.split_once("..") {
        return (a.trim().parse().unwrap()..b.trim().parse().unwrap()).collect();
    }
    if spec.contains(',') {
        return spec.split(',').map(|s| s.trim().parse().unwrap()).collect();
    }
    (1..=spec.trim().parse().unwrap()).collect()
}

fn steps(default: usize) -> usize {
    std::env::var("TINE_CONFORMANCE_STEPS")
        .ok()
        .map_or(default, |steps| steps.parse().unwrap())
}

fn run_all(watch: crate::WatchMode, default_seeds: &[u64], default_steps: usize) {
    let mut total: BTreeMap<&'static str, usize> = BTreeMap::new();
    let (mut checks, mut held, mut conflicted) = (0, 0, 0);
    for seed in seeds(default_seeds) {
        let started = Instant::now();
        let counts = run_seed(seed, steps(default_steps), watch);
        eprintln!(
            "live=fresh {watch:?} seed {seed}: {} checks in {:?}, {:?}",
            counts.checks,
            started.elapsed(),
            counts.actions
        );
        for (action, n) in counts.actions {
            *total.entry(action).or_default() += n;
        }
        checks += counts.checks;
        held += counts.held_at_check;
        conflicted += counts.conflicted_at_check;
    }
    eprintln!(
        "live=fresh {watch:?}: {checks} checks, {held} held and {conflicted} conflicted pages at checks, {total:?}"
    );
    // Not vacuous: windows held pages at the barriers, and the run reached
    // the host's own writers and outside writers alike.
    assert!(held > 0, "no page was held at a barrier");
    for action in ["open", "submit", "external rewrite"] {
        assert!(total.contains_key(action), "no {action} ran");
    }
}

/// A Windows root (verbatim prefix and backslashes) reads `<root>` wherever
/// the image holds it: plain, or inside a `Debug`-printed path or path list,
/// where every backslash is doubled. The temp directory differs between the
/// live store and the cold copy, so a path that survives fails the comparison
/// on Windows only (hosted run 39687809556).
#[test]
fn a_windows_root_reads_root_in_plain_and_debug_escaped_paths() {
    let root = Path::new(r"\\?\C:\Users\runneradmin\AppData\Local\Temp\.tmpHdiv69\graph");
    let page = format!("{}\\pages\\Beta.md", root.display());
    let image = format!("plain {page}\nexplicit 1 \"delta\" {:?}\n", [&page]);
    assert_eq!(
        normalized(&image, root),
        "plain <root>\\pages\\Beta.md\n\
         explicit 1 \"delta\" [\"<root>\\\\pages\\\\Beta.md\"]\n"
    );
}

/// F2/F5 with the default Notify watcher.
#[test]
fn live_equals_fresh_under_mixed_host_and_watcher_sequences() {
    run_all(crate::WatchMode::Notify, &[1, 2, 3], 10);
}

/// F2/F5 with the forced Poll watcher (a drain wakes its cycle).
#[test]
fn live_equals_fresh_under_mixed_host_and_poll_sequences() {
    run_all(crate::WatchMode::Poll, &[4, 5, 6], 10);
}

/// Found by the harness above (seed 4, Notify and Poll): a store save that
/// retitles an unheld page while the page cache is warm leaves the graph's
/// page-list memo at the old name. The name lookup and the twin guard are
/// right (`observe_name_entry` drops the lookup memo); the published view is
/// right. No production reader of the stale names was found: the listing's
/// warm-cache readers (journal-day recompute) read paths, kinds and days,
/// and a cold build relists (an unwarmed cache moves past the memo).
#[test]
fn a_store_retitle_moves_the_live_page_list() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("pages")).unwrap();
    fs::write(temp.path().join("pages/a.md"), "- a\n").unwrap();
    fs::write(temp.path().join("pages/b.md"), "- b [[a]]\n").unwrap();
    let store = Store::open(temp.path(), Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    let names = |store: &Store| {
        let mut names: Vec<_> = (store.graph.list_pages().iter())
            .map(|entry| entry.name.clone())
            .collect();
        names.sort();
        names
    };
    // Warm both memos at the current generation.
    assert_eq!(names(&store), ["a", "b"]);
    assert!(store
        .graph
        .find_entry("a", tine_core::model::PageKind::Page)
        .is_some());

    let save = |rel: &str, base: crate::SaveBase, body: &str| {
        let path = store.graph.root.join(rel);
        let dto = store
            .graph
            .page_dto_for_bytes(&path, body.as_bytes())
            .unwrap()
            .unwrap();
        let mut tx = store.transaction(None);
        tx.save_page(&[EditKind::SaveBlock], &PageId::from(rel), base, &dto);
        tx.commit()
    };
    let old = crate::SaveBase::Existing(FileRev::from_bytes(b"- a\n"));
    let retitled = save("pages/a.md", old, "title:: Renamed\n\n- a\n");
    assert!(matches!(retitled, crate::TxOutcome::Committed { .. }));

    // The published view names the page by its title, as a fresh store does.
    let view = store.whole_graph().unwrap();
    let mut listed: Vec<_> = view.list.iter().map(|entry| entry.name.clone()).collect();
    listed.sort();
    assert_eq!(listed, ["Renamed", "b"]);
    assert!(
        store
            .graph
            .find_entry("a", tine_core::model::PageKind::Page)
            .is_none(),
        "the old name still finds the retitled page"
    );
    assert_eq!(
        (store
            .graph
            .find_entry("Renamed", tine_core::model::PageKind::Page))
        .map(|entry| entry.path),
        Some(store.graph.root.join("pages/a.md")),
        "the new name finds nothing"
    );
    // A new file claiming the title is the retitled page's twin.
    let twin = save("pages/renamed.md", crate::SaveBase::CreateNew, "- twin\n");
    assert!(
        matches!(
            twin,
            crate::TxOutcome::NotCommitted {
                why: crate::Why::Refused(crate::Refusal::Twin { .. }),
                ..
            }
        ),
        "a twin of the retitled page was created"
    );
    // The live listing must agree with the view.
    assert_eq!(
        names(&store),
        ["Renamed", "b"],
        "live listing keeps the old name"
    );
}

/// Found by the harness above (seeds 21, 42, 85, 103, 122, 164; Notify
/// and Poll): a second file of a journal day (sync delivery, or an editor
/// writing the other file-name format) is a duplicate-day stray. A cold
/// build sets the stray aside (it does not read it; the build lists
/// through the day collapse), but the watcher's reconcile parses a stray
/// that is created, or present at launch and then edited, into the live
/// snapshot. While the session runs, the stray's blocks answer references,
/// backlinks and queries; after a restart they do not.
#[test]
fn a_duplicate_day_stray_is_indexed_as_a_cold_build_does() {
    let image = |store: &Store| {
        let view = store.whole_graph().unwrap();
        let mut slots: Vec<_> = (view.graph.pages.slots())
            .map(|(_, (entry, _))| format!("{:?}", entry.rel_path))
            .collect();
        slots.sort();
        let referrers = view.explicit_referrers(&["a".to_owned()]);
        format!("pages {slots:?}\nreferrers of a {referrers:?}")
    };
    for at_launch in [false, true] {
        let mut files = vec![
            ("pages/a.md", "- a\n"),
            ("journals/2026_10_02.md", "- the day\n"),
        ];
        if at_launch {
            files.push(("journals/2026-10-02.md", "- stray\n"));
        }
        for watch in [crate::WatchMode::Notify, crate::WatchMode::Poll] {
            let live = Live::watching(&files, watch);
            live.store.whole_graph().unwrap();
            fs::write(live.root.join("journals/2026-10-02.md"), "- stray [[a]]\n").unwrap();
            assert!(live
                .store
                .watcher_drained_for_tests(Duration::from_secs(20)));
            let running = image(&live.store);
            let title = live.store.graph.current_journal_format().title(
                tine_core::date::JournalDate::from_ordinal(
                    live.store
                        .graph
                        .entry_for_path(&live.root.join("journals/2026_10_02.md"))
                        .unwrap()
                        .date_key
                        .unwrap(),
                ),
            );
            assert!(
                matches!(live.store.whole_graph().unwrap().resolve(&title, true),
                crate::Resolved::Existing { others, .. } if others.len() == 1),
                "the duplicate must remain detectable through the store API"
            );
            live.host.stop();

            let copy = tempfile::tempdir().unwrap();
            copy_graph(&live.root, copy.path());
            let cold = Store::open(copy.path(), Default::default()).unwrap().0;
            cold.whole_graph_reconciled().unwrap();
            let label = format!("{watch:?}, stray at launch: {at_launch}");
            assert_eq!(running, image(&cold), "{label}: running vs restarted");
        }
    }
}

#[test]
fn a_journal_claimant_is_promoted_after_arrival_removal_and_held_reload() {
    for watch in [crate::WatchMode::Notify, crate::WatchMode::Poll] {
        let live = Live::watching(
            &[
                ("pages/a.md", "- a\n"),
                ("journals/2026-10-02.md", "- fallback [[a]]\n"),
                ("logseq/config.edn", "{}\n"),
            ],
            watch,
        );
        let referrers = |store: &Store| {
            store
                .whole_graph()
                .unwrap()
                .explicit_referrers(&["a".to_owned()])
        };
        let fallback = PageId::from("journals/2026-10-02.md");
        let preferred = PageId::from("journals/2026_10_02.md");
        assert_eq!(referrers(&live.store), [fallback.clone()]);
        fs::write(live.root.join(preferred.as_str()), "- preferred [[a]]\n").unwrap();
        assert!(live
            .store
            .watcher_drained_for_tests(Duration::from_secs(20)));
        assert_eq!(referrers(&live.store), [preferred.clone()]);
        live.open(fallback.as_str());
        fs::write(
            live.root.join("logseq/config.edn"),
            "{:preferred-format :org}\n",
        )
        .unwrap();
        assert!(live
            .store
            .watcher_drained_for_tests(Duration::from_secs(20)));
        assert_eq!(referrers(&live.store), [preferred.clone()]);
        fs::remove_file(live.root.join(preferred.as_str())).unwrap();
        assert!(live
            .store
            .watcher_drained_for_tests(Duration::from_secs(20)));
        assert_eq!(referrers(&live.store), [fallback.clone()]);
        let copy = tempfile::tempdir().unwrap();
        copy_graph(&live.root, copy.path());
        let cold = Store::open(copy.path(), Default::default()).unwrap().0;
        assert_eq!(referrers(&live.store), referrers(&cold));
        live.host.stop();
    }
}

/// Found by the harness above (seed 42, Notify and Poll): two pages claim
/// one name (`pages/Delta.md` and a page titled `Delta`, as sync or an
/// external editor can deliver). `IconIndex::get` takes the icon of the
/// claimant in the lowest page slot. A config reload discards the parsed
/// pages; while a page is held its rebuilt slot comes after the others, so
/// the name's icon flips to the other claimant's, although `resolve` still
/// names the same canonical file. A restart shows the first icon again.
#[test]
fn a_twin_claimed_name_keeps_its_icon_across_a_config_reload() {
    let image = |store: &Store| {
        let view = store.whole_graph().unwrap();
        let canonical = resolved(view.resolve("Delta", false));
        format!("{canonical} {:?}", view.page_icons(&["Delta".to_owned()]))
    };
    for watch in [crate::WatchMode::Notify, crate::WatchMode::Poll] {
        let files = [
            ("pages/Delta.md", "icon:: 🚀\n\n- d\n"),
            ("pages/twin.md", "icon:: 📚\ntitle:: Delta\n\n- t\n"),
            ("logseq/config.edn", "{}\n"),
        ];
        let live = Live::watching(&files, watch);
        let before = image(&live.store);
        live.open("pages/Delta.md");
        let config = "{:journal/file-name-format \"yyyy-MM-dd\"}\n";
        fs::write(live.root.join("logseq/config.edn"), config).unwrap();
        let moved = std::time::Instant::now();
        let format = |store: &Store| {
            let view = store.whole_graph().unwrap();
            view.config.config.journal_file_name_format.clone()
        };
        while format(&live.store).as_deref() != Some("yyyy-MM-dd") {
            assert!(
                moved.elapsed() < Duration::from_secs(20),
                "config not taken in"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(live
            .store
            .watcher_drained_for_tests(Duration::from_secs(20)));
        let running = image(&live.store);
        live.host.stop();

        let copy = tempfile::tempdir().unwrap();
        copy_graph(&live.root, copy.path());
        let cold = Store::open(copy.path(), Default::default()).unwrap().0;
        cold.whole_graph_reconciled().unwrap();
        assert_eq!(
            before,
            image(&cold),
            "{watch:?}: the config changes no icon"
        );
        assert_eq!(running, image(&cold), "{watch:?}: running vs restarted");
    }
}

#[test]
fn a_twin_icon_uses_filename_precedence_and_preserves_an_absent_icon() {
    for icon in ["", "icon:: 🚀\n"] {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("pages")).unwrap();
        // The title-only claimant sorts earlier and has a Markdown extension;
        // the matching filename still wins, even without an icon of its own.
        fs::write(
            temp.path().join("pages/AAA.md"),
            "title:: Delta\nicon:: 📚\n\n- other\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("pages/Delta.org"),
            format!("{}* winner\n", icon.replace("icon::", "#+icon:")),
        )
        .unwrap();
        let store = Store::open(temp.path(), Default::default()).unwrap().0;
        let view = store.whole_graph().unwrap();
        assert!(
            matches!(view.resolve("Delta", false), crate::Resolved::Existing { id, others }
            if id.as_str() == "pages/Delta.org" && others.len() == 1)
        );
        let icons = view.page_icons(&["Delta".to_owned()]);
        assert_eq!(
            icons.get("Delta").map(String::as_str),
            (!icon.is_empty()).then_some("🚀")
        );
    }
}
