//! What a rename or merge reports: its outcome, the files it wrote, the
//! identities it moved, the sites it could not spell (model r6 O7) and its
//! settings step (O8).

use std::io;
use std::sync::Arc;

use tine_core::refs;

/// What [`super::rename_or_merge_page`] did, and the page files it wrote.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RenameReport {
    pub outcome: RenameOutcome,
    /// Every page file moved, trashed or rewritten; empty when `Unchanged`.
    pub touched: Vec<TouchedPage>,
    /// Every page identity the rename moved, from its old name to the name it
    /// wrote: the renamed page (a reference-only one too), each file-backed
    /// namespace descendant, and each reference-only namespace member whose
    /// references it renamed (`path: None`). Empty when `Unchanged`. The
    /// window's navigation follows exactly this map; it never re-derives a
    /// moved name.
    pub renamed: Vec<super::RenamedPage>,
    /// Paths of referrers carrying VCS conflict markers whose references the
    /// rename left untouched (R-VCS-MARKERS); the UI says so. Reported once
    /// per file.
    pub skipped_conflicted_referrers: Vec<String>,
    /// O7: every site the rename left as it was because it cannot spell the
    /// new name there, and every property key it wrote in key spelling that
    /// differs from the new name, at its typed location; in file order.
    pub site_exceptions: Vec<SiteException>,
    /// The new `:default-home` page name when this rename moved the home page
    /// with it (in the rename's config.edn step, its own transaction after
    /// the page operation); `None` otherwise.
    pub home_page: Option<String>,
    /// The favorites list and arrangement page name after the rename, when
    /// the same config.edn step changed them; the frontend adopts it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favorites: Option<crate::config::FavoritesAfter>,
    /// Why the config.edn step skipped a move the rename called for
    /// (config.edn unreadable, or a value it cannot edit safely); the rename
    /// itself succeeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_skipped: Option<String>,
}

impl RenameReport {
    /// Page changes already published before a later, separate home-page
    /// configuration transaction failed. Callers must refresh these pages
    /// even though the overall operation returned an error. Other errors
    /// carry no such proof and may still represent partial publication.
    pub fn applied_before_error(error: &io::Error) -> Option<&Self> {
        error
            .get_ref()?
            .downcast_ref::<IncompleteRename>()
            .map(|error| &error.report)
    }

    /// [`Self::applied_before_error`] for a caller that has a window to refresh
    /// and no command line to inspect (the app's rename): the same report with
    /// the failed config write's cause in `config_skipped`. The rename happened;
    /// home and favorites did not follow, and the window says so.
    pub fn with_config_failure(error: &io::Error) -> Option<Self> {
        let incomplete = error.get_ref()?.downcast_ref::<IncompleteRename>()?;
        Some(Self {
            config_skipped: Some(incomplete.source.to_string()),
            ..incomplete.report.clone()
        })
    }

    pub(super) fn unchanged() -> Self {
        Self {
            outcome: RenameOutcome::Unchanged,
            touched: Vec::new(),
            renamed: Vec::new(),
            skipped_conflicted_referrers: Vec::new(),
            site_exceptions: Vec::new(),
            home_page: None,
            favorites: None,
            config_skipped: None,
        }
    }
}

#[derive(Debug)]
pub(super) struct IncompleteRename {
    pub(super) report: RenameReport,
    pub(super) source: io::Error,
}

impl std::fmt::Display for IncompleteRename {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Page files renamed, but the config.edn update (home page, favorites) failed: {}",
            self.source
        )
    }
}

impl std::error::Error for IncompleteRename {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// One page file a rename or merge wrote.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TouchedPage {
    /// Graph-relative path before the operation (the frontend's page id).
    pub path: String,
    /// The file left this path (moved, or trashed by a merge); otherwise it
    /// was rewritten in place.
    pub moved: bool,
}

/// What [`super::rename_or_merge_page`] did; serialized lowercase for the
/// frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RenameOutcome {
    /// Files moved and/or references rewritten.
    Renamed,
    /// Merged into the confirmed page (for a file-less source: its references
    /// were repointed).
    Merged,
    /// Nothing was written: identical spelling, an empty `old`, or a name
    /// that no file or reference uses.
    Unchanged,
}

/// One of O7's site exceptions (model r6; Lean `Report`), at its typed
/// location: a name site the rename left as it was because it cannot spell
/// the new name where it stands, or a property key it wrote in key spelling,
/// which differs from the new name.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SiteException {
    /// Graph-relative path of the file before the rename.
    pub path: String,
    /// The site's byte span in that file's text before the rename.
    pub start: usize,
    pub end: usize,
    pub kind: refs::SiteKind,
    pub disposition: Disposition,
}

/// What the rename did at a [`SiteException`] (the model's `Disposition`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Disposition {
    /// Left as it was: the site cannot spell the new name.
    Unspellable,
    /// A property key written in key spelling (lower-cased, whitespace
    /// hyphenated), which differs from the new name; the key was rewritten.
    NormalizedKey,
}

/// The site exceptions of one file's proposal (`sites`, in `path`'s text).
pub(super) fn site_exceptions<'a>(
    path: &'a str,
    sites: impl IntoIterator<Item = &'a refs::Site> + 'a,
) -> impl Iterator<Item = SiteException> + 'a {
    sites.into_iter().map(move |site| SiteException {
        path: path.to_owned(),
        start: site.range.start,
        end: site.range.end,
        kind: site.kind,
        disposition: match site.reason {
            refs::Unrealized::KeySpelling => Disposition::NormalizedKey,
            refs::Unrealized::HeadlineTag
            | refs::Unrealized::PropertyKey
            | refs::Unrealized::StoredQuery
            | refs::Unrealized::CrossPiece
            | refs::Unrealized::MacroLink
            | refs::Unrealized::LinkLabel
            | refs::Unrealized::NativeSpelling => Disposition::Unspellable,
        },
    })
}

/// A rename or merge the conceptual model's check refused before any write
/// (R-RENAME-CHECK): the file `file` (graph-relative, where it is before the
/// rename) breaks `guarantee`. Carried inside the `io::Error` it returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckRefused {
    pub guarantee: tine_store::Guarantee,
    pub file: String,
}

impl std::fmt::Display for CheckRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let file = &self.file;
        match self.guarantee {
            tine_store::Guarantee::P3 => write!(
                f,
                "cannot rename or merge: the next time Tine saves {file}, it would change that file's layout, so this request cannot write it as it is; nothing was renamed or merged; open {file} in the Tine app and save it once, then repeat the request"
            ),
            tine_store::Guarantee::P2 => write!(
                f,
                "cannot rename or merge: Tine's rewrite of {file} did not read back as intended, so nothing was renamed or merged; this is a bug in Tine, please report it with the page names at https://github.com/martinkoutecky/tine/issues"
            ),
        }
    }
}

impl std::error::Error for CheckRefused {}

impl CheckRefused {
    pub(crate) fn error(self) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, self)
    }

    /// The check's refusal inside a rename's or merge's error, if it is one.
    pub fn of(error: &io::Error) -> Option<&CheckRefused> {
        error.get_ref().and_then(|cause| cause.downcast_ref())
    }
}

/// The model's checks of one rename's steps: the one rename the request
/// names (its destination is the source's planned identity), each step's
/// page and whether it keeps its path read from the planning view.
pub(super) struct StepChecks<'g> {
    pub(super) plan: Arc<tine_store::CheckPlan>,
    graph: &'g tine_store::WholeGraph,
}

impl<'g> StepChecks<'g> {
    pub(super) fn new(
        graph: &'g tine_store::WholeGraph,
        old: &str,
        renamed: &[super::RenamedPage],
        days: &tine_core::date::JournalFormat,
        names: tine_core::config::FileNameFormat,
    ) -> io::Result<Self> {
        let destination = renamed
            .iter()
            .find(|page| refs::normalize(&page.from) == refs::normalize(old))
            .map(|page| page.to.clone())
            .ok_or_else(|| io::Error::other("rename plan names no destination"))?;
        let plan = tine_store::CheckPlan {
            old: Some(old.to_owned()),
            destination,
            days: days.clone(),
            names,
        };
        Ok(Self {
            plan: Arc::new(plan),
            graph,
        })
    }

    /// The file's page name before the rename, and whether it is a journal.
    pub(super) fn entry(&self, id: &tine_core::model::PageId) -> (String, bool) {
        let scope = tine_store::InventoryScope::File(id);
        let inventory = self.graph.inventory(scope);
        let first = inventory.0.first();
        first
            .map(|e| (e.name.clone(), e.is_journal))
            .unwrap_or_default()
    }

    /// The check of a referrer's, rebound page's or moved page's step.
    pub(super) fn step(
        &self,
        id: &tine_core::model::PageId,
        observed: Option<Arc<Observed>>,
    ) -> tine_store::RenameCheck {
        let (page, fixed) = self.entry(id);
        let plan = Arc::clone(&self.plan);
        let source = None;
        tine_store::RenameCheck {
            plan,
            page,
            fixed,
            observed,
            source,
        }
    }
}

/// The model's check of a merge's survivor under `plan`: it keeps its path,
/// and its expected reading is the merge of the source's and its own.
pub(super) fn survivor_check(
    plan: &Arc<tine_store::CheckPlan>,
    observed: &(Arc<Observed>, Arc<Observed>),
    page: String,
) -> tine_store::RenameCheck {
    tine_store::RenameCheck {
        plan: Arc::clone(plan),
        page,
        fixed: true,
        observed: Some(Arc::clone(&observed.1)),
        source: Some(Arc::clone(&observed.0)),
    }
}

type Observed = tine_core::cm_check::Content;
