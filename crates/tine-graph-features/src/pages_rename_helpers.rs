//! The rename dialog's helpers (D-20261115-HELP, D-20261116-TAGH): what the
//! interface may offer around a single rename or merge request, read without
//! writing. A helper is a sequence of ordinary requests and ordinary page
//! edits that the window performs through its normal save path; nothing here
//! writes, and the model's decision of each request is the planner's
//! (`rename_plan`), never re-derived. Split from `pages_rename_offer.rs`.
use std::collections::{HashMap, HashSet};
use std::io;

use tine_core::cm_check;
use tine_core::doc::native::{NativeAlias, NativeDocument};
use tine_core::model::Format;
use tine_core::refs;
use tine_store::{PageId, Store};

use super::rename_plan::{name_of, RenamePlan};

/// How many page names an effect lists; the count says how many there are.
const LISTED: usize = 8;

/// The tag union a merge's helper performs before the merge
/// (D-20261116-TAGH): `united` replaces the kept page's `tags::` value
/// `survivor`, and the source's `tags::` line (value `source`) is removed.
/// The values are the files' own text, so the window checks the loaded pages
/// still read them before editing (a cheap comparison, no re-read).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TagUnion {
    pub survivor: String,
    pub source: String,
    pub united: String,
}

/// One page edit of the alias dialog's "use the name for this page": the
/// `key` line of `page` drops the alias, reading `after` (None: the line
/// goes). `before` is the line's value as the file has it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AliasEdit {
    pub page: String,
    pub path: String,
    pub journal: bool,
    pub key: String,
    pub before: String,
    pub after: Option<String>,
}

/// What one request changes, from its plan: the pages whose text changes
/// (the moved or merged files and the pages whose links are rewritten), and
/// the links that open another page with their text unchanged.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Effect {
    pub pages: Vec<String>,
    pub page_count: usize,
    pub retargeted: Vec<Retarget>,
}

/// Links to `name` in `pages` open `after` instead of `before`; their text
/// stays.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Retarget {
    pub name: String,
    pub before: String,
    pub after: String,
    pub pages: Vec<String>,
    pub page_count: usize,
}

/// A file's member line of one key family, read by the merge's own header
/// reader ([`NativeDocument`]): exactly one line of the family, whose key
/// occurs once (the window's property write replaces one line per key), all
/// members with source coordinates, and no other text in the value than the
/// members and their commas. Anything else is no line the helper may edit.
struct MemberLine {
    key: String,
    value: String,
    /// (page key, the member's own source text), in order.
    members: Vec<(String, String)>,
}

fn read_text(store: &Store, id: &PageId) -> io::Result<Option<String>> {
    match store.read(&id.file(), Some(tine_store::PARSE_INPUT_MAX_BYTES)) {
        Ok((bytes, _)) => Ok(String::from_utf8(bytes).ok()),
        Err(tine_store::StoreError::NotFound(_)) => Ok(None),
        Err(e) => Err(crate::store_error(e)),
    }
}

fn is_org(id: &PageId) -> bool {
    Format::from_path(id.as_str().as_ref()) == Format::Org
}

/// The one line of `text` that `pick` selects, as a [`MemberLine`], or None.
fn member_line(
    text: &str,
    org: bool,
    pick: impl Fn(&tine_core::doc::native::NativeLine) -> Option<&NativeAlias>,
) -> Option<MemberLine> {
    let header = NativeDocument::read_with_tags(text, org);
    let mut picked = header
        .lines
        .iter()
        .filter_map(|line| pick(line).map(|members| (line, members)));
    let (line, members) = picked.next()?;
    if picked.next().is_some() || !members.movable {
        return None;
    }
    let key = text[line.key.clone()].to_owned();
    let same_key = header
        .lines
        .iter()
        .filter(|other| other.property.is_some())
        .filter(|other| text[other.key.clone()].eq_ignore_ascii_case(&key))
        .count();
    if same_key != 1 {
        return None;
    }
    let value = text[line.value.clone()].to_owned();
    // Nothing but the members and their separators: removing or adding a
    // member then loses no other text. The gaps between tokens are read in
    // place, once each (linear in the value).
    let mut at = line.value.start;
    for gap in members
        .tokens
        .iter()
        .map(|token| (token.start, token.end))
        .chain([(line.value.end, line.value.end)])
    {
        if gap.0 < at
            || text[at..gap.0]
                .chars()
                .any(|c| c != ',' && !c.is_whitespace())
        {
            return None;
        }
        at = gap.1;
    }
    let members = members
        .names
        .iter()
        .zip(&members.tokens)
        .map(|((name, _), token)| (refs::page_key(name), text[token.clone()].to_owned()))
        .collect();
    Some(MemberLine {
        key,
        value,
        members,
    })
}

/// Whether `value` on a `key` line reads as exactly `expected` (page keys,
/// in order): the written value is checked by the same reader before it is
/// offered. O(value bytes).
fn reads_as(key: &str, value: &str, org: bool, expected: &[String]) -> bool {
    let line = if org {
        format!("#+{key}: {value}\n")
    } else {
        format!("{key}:: {value}\n")
    };
    let tags = key.eq_ignore_ascii_case("tags");
    member_line(&line, org, |l| {
        if tags {
            l.tags.as_ref()
        } else {
            l.alias.as_ref()
        }
    })
    .is_some_and(|read| read.members.iter().map(|(k, _)| k).eq(expected.iter()))
}

/// D-20261116-TAGH: when both merged files have a `tags::` line, the union
/// the window writes before the merge. None when either file has no such
/// line, or a line the helper cannot edit without losing text (the merge
/// then proceeds as the model says: the source's line becomes a leftover
/// line). Cost: two file reads and header parses.
pub(super) fn tag_union(
    store: &Store,
    survivor: &PageId,
    source: &PageId,
) -> io::Result<Option<TagUnion>> {
    let org = is_org(survivor);
    if org != is_org(source) {
        return Ok(None);
    }
    let (Some(kept), Some(gone)) = (read_text(store, survivor)?, read_text(store, source)?) else {
        return Ok(None);
    };
    Ok(unite(&kept, &gone, org))
}

/// [`tag_union`] over the two files' texts. Linear in the tags: each line is
/// read once, membership is a set lookup. O(header bytes).
fn unite(kept: &str, gone: &str, org: bool) -> Option<TagUnion> {
    let tags = |text: &str| member_line(text, org, |line| line.tags.as_ref());
    let (Some(kept), Some(gone)) = (tags(kept), tags(gone)) else {
        return None;
    };
    let mut keys: Vec<String> = kept.members.iter().map(|(k, _)| k.clone()).collect();
    // Membership in a set, order in `keys`: linear in the tags.
    let mut seen: HashSet<String> = keys.iter().cloned().collect();
    let mut united = kept.value.trim_end().to_owned();
    for (key, token) in &gone.members {
        if !seen.insert(key.clone()) {
            continue;
        }
        keys.push(key.clone());
        if !united.trim().is_empty() {
            united.push_str(", ");
        }
        united.push_str(token);
    }
    reads_as(&kept.key, &united, org, &keys).then(|| TagUnion {
        survivor: kept.value,
        source: gone.value,
        united,
    })
}

/// The alias dialog's first step for one declaring page: drop `alias` from
/// its alias line. None when the page has no single editable line holding
/// it. Cost: one file read and header parse.
pub(super) fn alias_release(
    store: &Store,
    graph: &tine_store::WholeGraph,
    declarer: &PageId,
    alias: &str,
) -> io::Result<Option<AliasEdit>> {
    let Some(text) = read_text(store, declarer)? else {
        return Ok(None);
    };
    let org = is_org(declarer);
    let key = refs::page_key(alias);
    let holds =
        |members: &&NativeAlias| members.names.iter().any(|(n, _)| refs::page_key(n) == key);
    let Some(line) = member_line(&text, org, |line| line.alias.as_ref().filter(holds)) else {
        return Ok(None);
    };
    let kept: Vec<&(String, String)> = line.members.iter().filter(|(k, _)| *k != key).collect();
    let after = (!kept.is_empty()).then(|| {
        kept.iter()
            .map(|(_, token)| token.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    });
    if let Some(after) = &after {
        let keys: Vec<String> = kept.iter().map(|(k, _)| k.clone()).collect();
        if !reads_as(&line.key, after, org, &keys) {
            return Ok(None);
        }
    }
    let entry = graph
        .inventory(tine_store::InventoryScope::File(declarer))
        .0
        .first()
        .cloned();
    Ok(entry.map(|entry| AliasEdit {
        page: entry.name,
        path: declarer.as_str().to_owned(),
        journal: entry.is_journal,
        key: line.key,
        before: line.value,
        after,
    }))
}

/// The pages a plan changes (moved, merged and rewritten files), by name.
pub(super) fn effect(plan: &RenamePlan, retargeted: Vec<Retarget>) -> Effect {
    let mut ids: Vec<&PageId> = plan
        .moves
        .keys()
        .chain(&plan.candidates)
        .chain(&plan.moved_titles_to_rebind)
        .chain(plan.merge.iter().flat_map(|(a, b)| [a, b]))
        .collect();
    ids.sort_unstable_by(|a, b| a.as_str().cmp(b.as_str()));
    ids.dedup();
    let shown = ids
        .into_iter()
        .map(|id| name_of(&plan.graph, id).unwrap_or_else(|| id.as_str().to_owned()));
    let (pages, page_count) = names(shown);
    Effect {
        pages,
        page_count,
        retargeted,
    }
}

/// The links that open another page once the planned request is done, their
/// text unchanged: one row per name whose owner the plan changes. The plan
/// changes a name's owner inputs only through its files: a moved file
/// (`plan.moves`, children included) claims its new name and declares its
/// aliases from its new path, the merge's source goes and its declarations
/// join the survivor, a declaration of a renamed name (`plan.renamed`, a
/// page with no file included) is respelled to declare the new name (unless
/// the executor skips its file, [`RenamePlan::skips`]), and
/// `released` (the "use the name" edits) stop declaring `typed`. So the
/// affected names are the aliases the moved and merged files declare, the
/// names the plan gives, and `typed` when released; each one's
/// [`tine_store::NameOwners`] is patched and asked the store's one owner rule
/// before and after (no owner after: the link opens the name's page with no
/// file, shown by the name). A name the plan rewrites is skipped (its sites
/// are rewritten). The sites are the reference index's referrers of the
/// name's spellings (a day: every rendering); a page's alias declaration is
/// the owner input, not a link, so a declarer or the merge source counts
/// only when the parser reads a site of the name elsewhere in it. Cost: per
/// affected name one `name_owners` (plus one per renamed name it is given
/// from), two rule calls and one referrer lookup over its spellings; one
/// file read and parse per distinct declaring referrer.
pub(super) fn retargets(
    store: &Store,
    plan: &RenamePlan,
    typed: &str,
    released: &[PageId],
) -> io::Result<Vec<Retarget>> {
    let graph = &plan.graph;
    let (source, survivor) = match &plan.merge {
        Some((source, survivor)) => (Some(source), Some(survivor)),
        None => (None, None),
    };
    let moved = |id: &PageId| plan.moves.get(id).map(|to| PageId::from(to.as_str()));
    // Where a page's text is once the request is done.
    let lands = |id: &PageId| match survivor.filter(|_| Some(id) == source) {
        Some(survivor) => survivor.clone(),
        None => moved(id).unwrap_or_else(|| id.clone()),
    };
    let shown = |id: &PageId| {
        let renamed = plan.renamed.iter().find(|page| {
            page.path
                .as_ref()
                .is_some_and(|path| moved(&PageId::from(path.as_str())).as_ref() == Some(id))
        });
        renamed.map_or_else(
            || name_of(graph, id).unwrap_or_else(|| id.as_str().to_owned()),
            |page| page.to.clone(),
        )
    };
    let aliases = |id: &PageId| {
        graph
            .inventory(tine_store::InventoryScope::AliasesOf(id))
            .0
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<Vec<_>>()
    };
    let gained: Vec<(&str, PageId)> = (plan.renamed.iter())
        .filter_map(|page| {
            Some((
                page.to.as_str(),
                moved(&PageId::from(page.path.as_deref()?))?,
            ))
        })
        .collect();
    // The names each new name is given from: their declarations are
    // respelled to declare it.
    let mut given: HashMap<String, Vec<&str>> = HashMap::new();
    for page in &plan.renamed {
        given
            .entry(refs::page_key(&page.to))
            .or_default()
            .push(&page.from);
    }
    let affected = (plan.moves.keys().chain(source))
        .flat_map(aliases)
        .chain(plan.renamed.iter().map(|page| page.to.clone()))
        .chain((!released.is_empty()).then(|| typed.to_owned()));
    let (mut seen, mut rows, mut outside) = (HashSet::new(), Vec::new(), HashMap::new());
    for name in affected {
        if refs::renamed_page_name(&name, &plan.lookup).is_some() {
            continue;
        }
        let before = graph.name_owners(&name);
        if !seen.insert(before.spellings[0].clone()) {
            continue;
        }
        let mut after = before.clone();
        after.claimants.retain(|entry| {
            entry
                .rel_path
                .as_ref()
                .is_none_or(|id| !plan.moves.contains_key(id) && Some(id) != source)
        });
        for (to, id) in gained.iter().filter(|(to, _)| refs::same_page(to, &name)) {
            after.claimants.push(tine_core::model::PageEntry {
                name: (*to).to_owned(),
                kind: tine_core::model::PageKind::Page,
                date_key: None,
                rel_path: Some(id.clone()),
                path: Default::default(),
            });
        }
        let dropped = refs::same_page(&name, typed);
        // The declarations the request rewrites: a file the executor skips
        // (`RenamePlan::skips`) keeps declaring the old name.
        let mut respelled = Vec::new();
        for from in given.get(&refs::page_key(&name)).into_iter().flatten() {
            for id in graph.name_owners(from).declarers {
                if !sites_outside_aliases(store, plan, &mut outside, &id)?.skipped {
                    respelled.push(id);
                }
            }
        }
        after.declarers = (before.declarers.iter().cloned())
            .filter(|id| !(dropped && released.contains(id)))
            .chain(respelled)
            .map(|id| lands(&id))
            .collect();
        after.declarers.sort_unstable();
        after.declarers.dedup();
        let Some(was) = before.opens() else {
            continue;
        };
        let will = after.opens();
        // A moved page is the same page; a merged source is not.
        if will.is_some_and(|will| moved(was).as_ref().unwrap_or(was) == will) {
            continue;
        }
        let mut pages = std::collections::BTreeSet::new();
        for id in graph.explicit_referrers(&before.spellings) {
            let declares = before.declarers.contains(&id) || Some(&id) == source;
            if !declares || {
                let named = sites_outside_aliases(store, plan, &mut outside, &id)?;
                before.spellings.iter().any(|key| named.keys.contains(key))
            } {
                pages.insert(lands(&id));
            }
        }
        let (listed, page_count) = names(pages.iter().map(shown));
        if page_count > 0 {
            rows.push(Retarget {
                before: name_of(graph, was).unwrap_or_else(|| was.as_str().to_owned()),
                after: will.map_or_else(|| name.clone(), shown),
                name,
                pages: listed,
                page_count,
            });
        }
    }
    Ok(rows)
}

/// The day move's one row (D-20261111-MOA): `name` is declared by `day`
/// alone, and the move takes the declaration off the day's alias line before
/// the new page `name` exists, so the links to `name` stop opening the day.
/// The day is the only declarer (the caller refuses others), so its own
/// links and alias line are not listed; the owner and the link spellings are
/// the store's ([`tine_store::NameOwners`], one rule, the same
/// [`WholeGraph::explicit_referrers`] lookup as [`retargets`]). No plan
/// exists for a day (its rename is refused), so the row is built from the
/// owner inputs alone. Cost: one `name_owners` and one referrer lookup.
pub(super) fn released_retarget(
    graph: &tine_store::WholeGraph,
    name: &str,
    day: &PageId,
) -> Option<Retarget> {
    let before = graph.name_owners(name);
    if before.opens() != Some(day) {
        return None;
    }
    let mut referrers = graph.explicit_referrers(&before.spellings);
    referrers.retain(|id| id != day);
    let (pages, page_count) = names(
        referrers
            .iter()
            .map(|id| name_of(graph, id).unwrap_or_else(|| id.as_str().to_owned())),
    );
    (page_count > 0).then(|| Retarget {
        name: name.to_owned(),
        before: name_of(graph, day).unwrap_or_else(|| day.as_str().to_owned()),
        after: name.to_owned(),
        pages,
        page_count,
    })
}

#[cfg(test)]
thread_local! {
    /// Files [`sites_outside_aliases`] read and parsed on this thread.
    static SITE_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// What one file holds for the preview: the page keys of its sites outside
/// its alias declarations, and whether the executor leaves it unwritten.
struct FileSites {
    keys: HashSet<String>,
    skipped: bool,
}

/// The page keys of every site the parser reads in `id`'s file outside its
/// alias declarations (the model's reading, `cm_check::observe`: links,
/// tags, Org headline tags, embeds, property values, query and macro
/// arguments). Read and parsed once per file; `outside` keeps the answer for
/// the request's other names. A file with no text names nothing. O(file
/// bytes) the first time, a lookup after.
fn sites_outside_aliases<'a>(
    store: &Store,
    plan: &RenamePlan,
    outside: &'a mut HashMap<PageId, FileSites>,
    id: &PageId,
) -> io::Result<&'a FileSites> {
    if !outside.contains_key(id) {
        #[cfg(test)]
        SITE_READS.with(|n| n.set(n.get() + 1));
        let mut keys = HashSet::new();
        let mut skipped = false;
        if let Some(text) = read_text(store, id)? {
            skipped = plan.skips(id, &text);
            let format = Format::from_path(id.as_str().as_ref());
            let content = cm_check::observe(&text, format, plan.config.file_name_format);
            let headers = (content.header.iter())
                .filter(|header| header.role != cm_check::HeaderRole::Aliases)
                .map(|header| &header.text);
            for atom in headers
                .chain(content.blocks.iter().map(|block| &block.body))
                .flatten()
            {
                match atom {
                    cm_check::Atom::Text(_) => {}
                    cm_check::Atom::Site(s) => site_keys(s, &mut keys),
                    cm_check::Atom::Title(s, parts) => {
                        site_keys(s, &mut keys);
                        for part in parts {
                            if let cm_check::Part::Site(s) = part {
                                site_keys(s, &mut keys);
                            }
                        }
                    }
                }
            }
        }
        outside.insert(id.clone(), FileSites { keys, skipped });
    }
    Ok(&outside[id])
}

/// A site's page key and those of the sites the parser reads inside it.
fn site_keys(site: &cm_check::Site, keys: &mut HashSet<String>) {
    keys.insert(refs::page_key(&site.name));
    for (_, inner) in &site.inner {
        site_keys(inner, keys);
    }
}

pub(super) fn names(shown: impl Iterator<Item = String>) -> (Vec<String>, usize) {
    let mut count = 0;
    let mut listed = Vec::new();
    for name in shown {
        count += 1;
        if listed.len() < LISTED {
            listed.push(name);
        }
    }
    (listed, count)
}

#[cfg(test)]
mod tests {
    use super::unite;
    use crate::pages::{rename_offer, RenameOffer};

    fn store(files: &[(String, String)]) -> (tempfile::TempDir, tine_store::Store) {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("pages")).unwrap();
        for (rel, body) in files {
            std::fs::write(root.path().join(rel), body).unwrap();
        }
        let store = tine_store::Store::open(root.path(), Default::default())
            .unwrap()
            .0;
        (root, store)
    }

    /// PC-1 (review of 0a3d95eae, finding 3): the alias dialog reads and
    /// parses each declaring file once, however many of its aliases the
    /// request moves: N aliases on the source once cost N rereads and
    /// parses (69/261/1,029 outline reads at 32/128/512).
    #[test]
    fn the_dialogs_site_reads_are_per_file_not_per_alias() {
        let mut reads = Vec::new();
        for n in [32, 128, 512] {
            let aliases: Vec<String> = (0..n).map(|i| format!("A{i}")).collect();
            let (_root, store) = store(&[
                (
                    "pages/Source.md".into(),
                    format!("alias:: {}\n\n- s\n", aliases.join(", ")),
                ),
                ("pages/Z.md".into(), "alias:: ML\n\n- z\n".into()),
            ]);
            super::SITE_READS.with(|c| c.set(0));
            let RenameOffer::Alias(dialog) = rename_offer(&store, "Source", "ML", None).unwrap()
            else {
                panic!("the alias dialog");
            };
            assert_eq!(dialog.merges.len(), 1);
            assert!(dialog.own.is_some());
            reads.push(super::SITE_READS.with(|c| c.get()));
            store.close();
        }
        // The source, once per request (its merge and "use the name").
        assert_eq!(reads, [2, 2, 2], "site reads at 32/128/512 aliases");
    }

    /// PC-2, PC-4 (review of 0a3d95eae, finding 4): one graph-wide stamp
    /// walk per dialog, the typed request's own, whatever the number of merge
    /// choices (each choice once asked the whole rename question again).
    #[test]
    fn the_dialog_walks_the_graph_once_for_any_number_of_merge_choices() {
        let mut walks = Vec::new();
        for c in [1, 4, 16] {
            let mut files: Vec<(String, String)> = (0..c)
                .map(|i| {
                    (
                        format!("pages/P{i:02}.md"),
                        "alias:: ML\n\n- p\n".to_owned(),
                    )
                })
                .collect();
            files.push(("pages/Source.md".into(), "- s\n".into()));
            let (_root, store) = store(&files);
            crate::pages::PAGE_STAMP_WALKS.with(|n| n.set(0));
            let RenameOffer::Alias(dialog) = rename_offer(&store, "Source", "ML", None).unwrap()
            else {
                panic!("the alias dialog");
            };
            assert_eq!(dialog.merges.len(), c, "{:?}", dialog.merges_refused);
            walks.push(crate::pages::PAGE_STAMP_WALKS.with(|n| n.get()));
            store.close();
        }
        assert_eq!(walks, [1, 1, 1], "walks for 1/4/16 merge choices");
    }

    /// PC-1: the union and its line checks are linear in the tags. At
    /// 16,384 tags per page the quadratic version (token removal that
    /// shifted the rest of the value, membership by vector scan) took about
    /// 2.7 s in a debug build; the linear one about 0.3 s.
    #[test]
    fn the_tag_union_is_linear_in_the_tags() {
        let n = 16_384;
        let line = |prefix: &str| {
            (0..n)
                .map(|i| format!("{prefix}{i}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let kept = format!("tags:: {}, shared\n\n- kept\n", line("k"));
        let gone = format!("tags:: shared, {}\n\n- gone\n", line("g"));
        let started = std::time::Instant::now();
        let united = unite(&kept, &gone, false).expect("a union").united;
        let elapsed = started.elapsed();
        assert_eq!(united.matches(", ").count(), 2 * n);
        assert!(
            elapsed < std::time::Duration::from_millis(1_200),
            "{elapsed:?} for {n} tags per page"
        );
    }
}
