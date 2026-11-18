//! The rename dialog's helpers (D-20261115-HELP, D-20261116-TAGH): what the
//! interface may offer around a single rename or merge request, read without
//! writing. A helper is a sequence of ordinary requests and ordinary page
//! edits that the window performs through its normal save path; nothing here
//! writes, and the model's decision of each request is the planner's
//! (`rename_plan`), never re-derived. Split from `pages_rename_offer.rs`.
use std::collections::HashSet;
use std::io;

use tine_core::doc::native::{NativeAlias, NativeDocument};
use tine_core::model::Format;
use tine_core::refs;
use tine_store::{PageId, Resolved, Store};

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
    let (pages, page_count) = names(&plan.graph, ids.into_iter());
    Effect {
        pages,
        page_count,
        retargeted,
    }
}

/// What a planned request does to the files a name's alias declarations
/// can give it (r6 "Outside the model": each helper step is one request).
pub(super) enum Request<'a> {
    /// Merge `source` into `survivor` (named `name`): the source's file goes
    /// and its aliases join the survivor.
    Merge {
        source: &'a PageId,
        survivor: &'a PageId,
        name: &'a str,
    },
    /// "Use the name": every other declarer drops `name` from its alias
    /// line, then `source` is renamed to `name`, its file moving to `to`.
    Own {
        source: &'a PageId,
        to: &'a PageId,
        name: &'a str,
    },
}

/// Links to `name` that open another page once `request` is done, their
/// text unchanged; None when the page `name` opens does not change, or no
/// page links it. The page before is what `resolve` answers now; after, a
/// real file claiming `name` still wins (the requests here move or remove
/// only the source, whose own name they rewrite), the own rename's moved
/// file claims `name`, and otherwise the declarer with the least path wins,
/// the alias relation's rule (`owner_path`), over the declarers as the
/// request leaves them. A page declaring `name` is not listed: its
/// reference is that declaration, which the request keeps, moves or removes
/// (a declarer that also links the name in its body is not listed either).
/// Cost: one resolve and one referrers lookup.
pub(super) fn retarget(
    graph: &tine_store::WholeGraph,
    name: &str,
    request: &Request<'_>,
) -> Option<Retarget> {
    let Resolved::Alias { owners } = graph.resolve(name, false) else {
        return None;
    };
    let before = owners.first()?;
    let (source, mine, mine_name) = match request {
        Request::Merge {
            source,
            survivor,
            name,
        } => (*source, *survivor, *name),
        Request::Own { source, to, name } => (*source, *to, *name),
    };
    let after = match request {
        Request::Own { name: own, .. } if refs::same_page(name, own) => mine.clone(),
        Request::Own { source, to, .. } => owners
            .iter()
            .map(|id| if id == *source { *to } else { id })
            .min_by(|a, b| a.as_str().cmp(b.as_str()))?
            .clone(),
        Request::Merge {
            source, survivor, ..
        } => owners
            .iter()
            .filter(|id| *id != *source)
            .chain([*survivor])
            .min_by(|a, b| a.as_str().cmp(b.as_str()))?
            .clone(),
    };
    // The own rename moves the source (the same page); a merge's source goes.
    let own = matches!(request, Request::Own { .. });
    let still = if own && before == source {
        mine
    } else {
        before
    };
    if after == *still {
        return None;
    }
    let shown = |id: &PageId| {
        if id == mine {
            mine_name.to_owned()
        } else {
            name_of(graph, id).unwrap_or_else(|| id.as_str().to_owned())
        }
    };
    let mut referrers = graph.explicit_referrers(&[name.to_owned()]);
    referrers.retain(|id| id != source && !owners.contains(id));
    let (pages, page_count) = names(graph, referrers.iter());
    (page_count > 0).then(|| Retarget {
        name: name.to_owned(),
        before: name_of(graph, before).unwrap_or_else(|| before.as_str().to_owned()),
        after: shown(&after),
        pages,
        page_count,
    })
}

fn names<'a>(
    graph: &tine_store::WholeGraph,
    ids: impl Iterator<Item = &'a PageId>,
) -> (Vec<String>, usize) {
    let mut count = 0;
    let mut listed = Vec::new();
    for id in ids {
        count += 1;
        if listed.len() < LISTED {
            listed.push(name_of(graph, id).unwrap_or_else(|| id.as_str().to_owned()));
        }
    }
    (listed, count)
}

#[cfg(test)]
mod tests {
    use super::unite;

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
