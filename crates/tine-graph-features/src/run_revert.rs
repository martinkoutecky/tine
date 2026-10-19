//! Reverting a command-line run on one page (CLI slice 3, D-20261019-CL3).
//!
//! A run's agent commits are undone newest first. Each step is a Concord
//! three-way merge with the agent's result as the base, the page as it is now
//! as "mine" and the text before the agent's commit as "theirs":
//!
//! - a row only the agent changed (verdict `TheirsOnly`) takes the old text,
//!   so the agent's edit, insertion or deletion is undone;
//! - a row the user changed since (`MineOnly`) stays as it is;
//! - a row both changed (`BothChanged`), or one the base cannot place, keeps
//!   the user's text and is listed in [`RevertedPage::kept`].
//!
//! Nothing is written here: the caller saves [`RevertedPage::doc`] through
//! the guarded save, once per page, so the whole revert of a page is one
//! change.

use std::collections::HashMap;

use tine_core::doc::Document;
use tine_core::model::{Format, PageDto};
use tine_core::sync_diff::{self, Diff3Verdict, DiffRow, RowKind};
use tine_store::{PageId, Store};

use crate::conflicts::{dto, parse};

/// One agent commit's effect on the page: its text before and after.
/// An absent side (the commit created or deleted the file) is empty text.
pub struct RevertStep<'a> {
    pub before: &'a str,
    pub after: &'a str,
}

/// A block the revert left as the user has it, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptBlock {
    /// The block's current first line (or the page properties).
    pub text: String,
    pub reason: KeptReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeptReason {
    /// The agent changed it and the user changed it since.
    ChangedSince,
    /// The merge base could not place the block, so the safe choice is the
    /// user's text.
    NoBase,
}

impl KeptReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::ChangedSince => "changed-since",
            Self::NoBase => "no-base",
        }
    }
}

pub struct RevertedPage {
    pub doc: PageDto,
    /// True when the revert changes the page at all.
    pub changed: bool,
    pub kept: Vec<KeptBlock>,
}

/// Undo `steps` (newest first) on the page whose current text is `current`.
/// `Err` is a merge the Concord rules refuse; the caller reports it and
/// writes nothing for this page.
pub fn revert_page(
    store: &Store,
    page: &PageId,
    current: &str,
    steps: &[RevertStep<'_>],
) -> Result<RevertedPage, sync_diff::MergeRefused> {
    let fmt = Format::from_path(page.as_str().as_ref());
    let (doc, changed, kept) = revert_doc(current, steps, fmt)?;
    Ok(RevertedPage {
        doc: dto(store, page, doc),
        changed,
        kept,
    })
}

/// [`revert_page`] on documents: the merged page, whether it differs from
/// `current`, and the blocks kept as the user has them.
pub(crate) fn revert_doc(
    current: &str,
    steps: &[RevertStep<'_>],
    fmt: Format,
) -> Result<(Document, bool, Vec<KeptBlock>), sync_diff::MergeRefused> {
    let start = parse(current, fmt);
    let mut doc = start.clone();
    let mut kept = Vec::new();
    for step in steps {
        let base = parse(step.after, fmt);
        let theirs = parse(step.before, fmt);
        let diff = sync_diff::diff3_docs(&base, &doc, &theirs);
        let partners = Partners::new(&base, &doc, &theirs);
        let mut decisions = HashMap::new();
        decide(&diff.rows, &partners, &mut decisions, &mut kept);
        let roots = sync_diff::merge_blocks3(
            Some(&base.roots),
            &doc.roots,
            &theirs.roots,
            None,
            &decisions,
        )?;
        let pre_block = revert_pre(&base, &doc, &theirs, &mut kept);
        doc = Document { pre_block, roots };
    }
    let changed = !same_doc(&start, &doc);
    Ok((doc, changed, kept))
}

type Key = (String, String);

fn key(view: &sync_diff::BlockView) -> Key {
    (view.uuid.clone(), view.text.clone())
}

/// Where an old block went. Concord pairs a block across two versions only
/// when the texts are close, so an agent's edit and a later edit by the user
/// can show up as an unpaired old block plus an unpaired current one. To tell
/// "the agent deleted it" from "the agent changed it", each old ("theirs")
/// block is looked up in the base, and that base block in the current page.
struct Partners {
    /// Old block -> its base (agent's result) block, if the base kept it.
    base_of_old: HashMap<Key, Key>,
    /// Base block -> the current page's version (`None`: unchanged).
    now_of_base: HashMap<Key, Option<String>>,
    /// Current block -> the base block it continues.
    base_of_now: HashMap<Key, Key>,
    /// Base blocks the agent's commit did not touch.
    untouched: std::collections::HashSet<Key>,
}

impl Partners {
    fn new(base: &Document, now: &Document, old: &Document) -> Self {
        fn pairs(rows: &[DiffRow], out: &mut Vec<(Key, Key, RowKind)>) {
            for row in rows {
                if let (Some(a), Some(b)) = (&row.mine, &row.theirs) {
                    out.push((key(a), key(b), row.kind));
                }
                pairs(&row.children, out);
            }
        }
        let mut base_old = Vec::new();
        pairs(
            &sync_diff::diff_blocks(&base.roots, &old.roots),
            &mut base_old,
        );
        let mut base_now = Vec::new();
        pairs(
            &sync_diff::diff_blocks(&base.roots, &now.roots),
            &mut base_now,
        );
        Partners {
            untouched: base_old
                .iter()
                .filter(|(_, _, kind)| *kind == RowKind::Unchanged)
                .map(|(b, _, _)| b.clone())
                .collect(),
            base_of_old: base_old.into_iter().map(|(b, o, _)| (o, b)).collect(),
            base_of_now: base_now
                .iter()
                .map(|(b, n, _)| (n.clone(), b.clone()))
                .collect(),
            now_of_base: base_now
                .into_iter()
                .map(|(b, n, kind)| (b, (kind != RowKind::Unchanged).then_some(n.1)))
                .collect(),
        }
    }

    /// The user changed, since the commit, a block the commit wrote.
    fn user_changed_agent_block(&self, now: &sync_diff::BlockView) -> bool {
        self.base_of_now
            .get(&key(now))
            .is_some_and(|base| !self.untouched.contains(base))
    }
}

fn decide(
    rows: &[DiffRow],
    partners: &Partners,
    decisions: &mut HashMap<String, String>,
    kept: &mut Vec<KeptBlock>,
) {
    for row in rows {
        if row.kind != RowKind::Unchanged {
            match row.verdict {
                Some(Diff3Verdict::TheirsOnly) if row.kind == RowKind::Removed => {
                    // An old block missing from the page now. Restore it if the
                    // agent deleted it, or changed it and the user has left the
                    // agent's version alone; otherwise the user's state wins.
                    let old = row.theirs.as_ref().map(key);
                    match old.and_then(|old| partners.base_of_old.get(&old)) {
                        None => {
                            decisions.insert(row.id.clone(), "theirs".into());
                        }
                        Some(base) => match partners.now_of_base.get(base) {
                            Some(None) => {
                                decisions.insert(row.id.clone(), "theirs".into());
                            }
                            Some(Some(now)) => kept.push(KeptBlock {
                                text: first_line(now),
                                reason: KeptReason::ChangedSince,
                            }),
                            None => kept.push(kept_row(row, KeptReason::ChangedSince)),
                        },
                    }
                }
                Some(Diff3Verdict::TheirsOnly) => {
                    decisions.insert(row.id.clone(), "theirs".into());
                }
                Some(Diff3Verdict::MineOnly) => {
                    if row
                        .mine
                        .as_ref()
                        .is_some_and(|m| partners.user_changed_agent_block(m))
                    {
                        kept.push(kept_row(row, KeptReason::ChangedSince));
                    }
                }
                Some(Diff3Verdict::BothChanged) => {
                    kept.push(kept_row(row, KeptReason::ChangedSince))
                }
                None => kept.push(kept_row(row, KeptReason::NoBase)),
            }
        }
        decide(&row.children, partners, decisions, kept);
    }
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").to_string()
}

fn kept_row(row: &DiffRow, reason: KeptReason) -> KeptBlock {
    let view = row.mine.as_ref().or(row.theirs.as_ref());
    let text = view.map(|v| first_line(&v.text)).unwrap_or_default();
    KeptBlock { text, reason }
}

fn norm_pre(pre: Option<&str>) -> &str {
    pre.map(str::trim_end).unwrap_or("")
}

/// Page properties follow the same rule as a row, as one unit.
fn revert_pre(
    base: &Document,
    mine: &Document,
    theirs: &Document,
    kept: &mut Vec<KeptBlock>,
) -> Option<String> {
    let (b, m, t) = (
        norm_pre(base.pre_block.as_deref()),
        norm_pre(mine.pre_block.as_deref()),
        norm_pre(theirs.pre_block.as_deref()),
    );
    if b == t || m == t {
        return mine.pre_block.clone();
    }
    if m == b {
        return theirs.pre_block.clone();
    }
    kept.push(KeptBlock {
        text: "page properties".into(),
        reason: KeptReason::ChangedSince,
    });
    mine.pre_block.clone()
}

fn same_doc(a: &Document, b: &Document) -> bool {
    norm_pre(a.pre_block.as_deref()) == norm_pre(b.pre_block.as_deref())
        && sync_diff::diff_blocks(&a.roots, &b.roots)
            .iter()
            .all(|row| row.kind == RowKind::Unchanged)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(doc: &Document) -> Vec<String> {
        fn walk(blocks: &[tine_core::doc::DocBlock], depth: usize, out: &mut Vec<String>) {
            for b in blocks {
                out.push(format!("{}{}", "  ".repeat(depth), b.raw().trim_end()));
                walk(&b.children, depth + 1, out);
            }
        }
        let mut out = Vec::new();
        walk(&doc.roots, 0, &mut out);
        out
    }

    fn revert(current: &str, steps: &[(&str, &str)]) -> (Vec<String>, bool, Vec<KeptBlock>) {
        let steps: Vec<RevertStep<'_>> = steps
            .iter()
            .map(|(before, after)| RevertStep { before, after })
            .collect();
        let (doc, changed, kept) = revert_doc(current, &steps, Format::Md).unwrap();
        (texts(&doc), changed, kept)
    }

    #[test]
    fn an_untouched_agent_edit_is_undone() {
        let before = "- alpha\n- beta\n- gamma\n";
        let after = "- alpha\n- BETA by agent\n- gamma\n";
        let (rows, changed, kept) = revert(after, &[(before, after)]);
        assert_eq!(rows, ["alpha", "beta", "gamma"]);
        assert!(changed);
        assert!(kept.is_empty());
    }

    #[test]
    fn an_agent_insertion_is_removed_and_a_deletion_restored() {
        let before = "- alpha\n- beta\n";
        let after = "- alpha\n- new from agent\n";
        let (rows, _, kept) = revert(after, &[(before, after)]);
        assert_eq!(rows, ["alpha", "beta"]);
        assert!(kept.is_empty());
    }

    #[test]
    fn the_users_later_edits_elsewhere_survive() {
        let before = "- alpha\n- beta\n- gamma\n";
        let after = "- alpha\n- BETA by agent\n- gamma\n";
        let current = "- alpha typed by you\n- BETA by agent\n- gamma\n- added by you\n";
        let (rows, _, kept) = revert(current, &[(before, after)]);
        assert_eq!(
            rows,
            ["alpha typed by you", "beta", "gamma", "added by you"]
        );
        assert!(kept.is_empty());
    }

    #[test]
    fn an_unpaired_block_both_changed_loses_nothing() {
        let before = "- alpha\n- beta\n";
        let after = "- alpha\n- beta, agent\n";
        let current = "- alpha\n- beta, agent, and then you\n";
        let (rows, _, kept) = revert(current, &[(before, after)]);
        // Short texts this far apart are not paired by Concord, at either
        // step, so this reads as "the agent deleted beta" and "you wrote a
        // new block": beta comes back and your block stays. Nothing of yours
        // is lost; the paired case below is the one that lists the block.
        assert_eq!(rows, ["alpha", "beta, agent, and then you", "beta"]);
        assert!(kept.is_empty());
    }

    #[test]
    fn a_paired_block_both_changed_keeps_only_the_users_text() {
        let before = "- alpha\n- the quick brown fox jumps over the lazy dog\n";
        let after = "- alpha\n- the quick brown fox jumps over the lazy cat\n";
        let current = "- alpha\n- the quick brown fox leaps over the lazy cat\n";
        let (rows, changed, kept) = revert(current, &[(before, after)]);
        assert_eq!(
            rows,
            ["alpha", "the quick brown fox leaps over the lazy cat"]
        );
        assert!(!changed);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].text, "the quick brown fox leaps over the lazy cat");
    }

    #[test]
    fn an_unpaired_agent_rewrite_is_undone() {
        let before = "- alpha\n- short\n";
        let after = "- alpha\n- an entirely different sentence by the agent\n";
        let (rows, _, kept) = revert(after, &[(before, after)]);
        assert_eq!(rows, ["alpha", "short"]);
        assert!(kept.is_empty());
    }

    #[test]
    fn a_block_the_user_deleted_since_is_not_brought_back() {
        let before = "- alpha\n- beta\n";
        let after = "- alpha\n- beta, agent\n";
        let current = "- alpha\n";
        let (rows, _, kept) = revert(current, &[(before, after)]);
        // Unpaired (see above): the agent's insertion is gone already, its
        // deletion of "beta" is undone.
        assert_eq!(rows, ["alpha", "beta"]);
        assert!(kept.is_empty());
    }

    #[test]
    fn steps_apply_newest_first() {
        let v0 = "- alpha\n";
        let v1 = "- alpha\n- one\n";
        let v2 = "- alpha\n- one\n- two\n";
        let (rows, _, kept) = revert(v2, &[(v1, v2), (v0, v1)]);
        assert_eq!(rows, ["alpha"]);
        assert!(kept.is_empty());
    }

    #[test]
    fn a_page_the_run_created_ends_empty() {
        let after = "- made by the agent\n";
        let (rows, changed, _) = revert(after, &[("", after)]);
        assert!(rows.is_empty());
        assert!(changed);
    }

    #[test]
    fn page_properties_follow_the_same_rule() {
        let before = "tags:: a\n\n- x\n";
        let after = "tags:: a, agent\n\n- x\n";
        let steps = [RevertStep { before, after }];
        let (doc, _, kept) = revert_doc(after, &steps, Format::Md).unwrap();
        assert_eq!(
            doc.pre_block.as_deref().map(str::trim_end),
            Some("tags:: a")
        );
        assert!(kept.is_empty());
        let current = "tags:: mine\n\n- x\n";
        let (doc, _, kept) = revert_doc(current, &steps, Format::Md).unwrap();
        assert_eq!(
            doc.pre_block.as_deref().map(str::trim_end),
            Some("tags:: mine")
        );
        assert_eq!(kept.len(), 1);
    }
}
