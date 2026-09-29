//! Concord (og family 8) — the conflict queue's data model, the VCS-marker
//! scanner and the VCS-marker parser.
//!
//! A **conflict object** is an entirely DERIVED inventory item: nothing here is
//! stored in the graph and nothing is stored outside it either, so the queue
//! survives a restart for free — it is recomputed from what is on disk
//! (`tine_graph_features::conflicts::conflict_inventory`). Two sources feed it:
//!
//! - **conflict copies** left by a sync transport (Syncthing `*.sync-conflict-*`,
//!   Dropbox/Seafile conflicted copies) paired with the winner page they shadow;
//! - **marker-bearing pages** left by a VCS merge (git/Fossil).
//!
//! The model deliberately does NOT assume exactly two sides: a diff3/Fossil
//! marker region carries three (ours, common ancestor, theirs).
//! [`ConflictObject::sides`] is therefore a list, not a pair.
//!
//! ## Marker scanning and parsing
//!
//! [`scan_vcs_conflict_markers`] is THE marker scanner. The store's save
//! refusal (`tine-store` transaction preflight) and [`parse_vcs_marker_sides`]
//! both derive from it, so the code that REFUSES to rewrite a conflicted file
//! and the code that RESOLVES one can never disagree about what a marker is.
//! [`parse_vcs_marker_sides`] turns a marker-bearing file into two or three
//! COMPLETE page texts — the file as each side would have written it — so the
//! ordinary block-level machinery (`sync_diff::diff_docs` / `diff3_docs`,
//! `sync_diff::merge_blocks3`) applies unchanged.

use crate::model::PageKind;
use serde::{Deserialize, Serialize};

/// Distinct VCS merge-conflict marker kinds present in `content`, in order of
/// first appearance — empty when the file is not merge-conflicted.
///
/// Recognizes the column-0 markers git writes (`<<<<<<< ours`, diff3
/// `||||||| base`, `=======`, `>>>>>>> theirs`) and Fossil's verbose variants
/// (`<<<<<<< BEGIN MERGE CONFLICT: …`, `####### SUGGESTED CONFLICT RESOLUTION
/// follows …`, `||||||| COMMON ANCESTOR content follows …`, `======= MERGED IN
/// content follows …`, `>>>>>>> END MERGE CONFLICT …` — the `mergeMarker` table
/// in fossil's `src/merge3.c`). Both tools write markers at column 0 only, so
/// indented lines never count.
///
/// Two guards keep a page that merely DOCUMENTS merge conflicts from being
/// flagged:
/// - lines inside column-0 fenced code blocks (``` / ~~~) are ignored; markers
///   quoted in an indented fence inside a bullet are not at column 0 anyway;
/// - the file counts as conflicted only if an anchor line (`<<<<<<< ` or
///   `>>>>>>> `) is present — a lone `=======` (e.g. a setext-style divider)
///   never quarantines a page.
pub fn vcs_conflict_markers(content: &str) -> Vec<&'static str> {
    let scan = scan_vcs_conflict_markers(content);
    if !scan
        .iter()
        .any(|(_, kind)| matches!(kind, ConflictMarkerKind::Ours | ConflictMarkerKind::Theirs))
    {
        // No anchor line → not a conflicted file (a lone `=======` is a divider).
        return Vec::new();
    }
    let mut seen: Vec<&'static str> = Vec::new();
    for (_, kind) in scan {
        let token = kind.token();
        if !seen.contains(&token) {
            seen.push(token);
        }
    }
    seen
}

/// One recognized column-0 VCS merge-conflict marker line.
///
/// Ordering inside a git/Fossil conflict region is
/// `Ours` → [`Suggested` (Fossil only)] → [`Base`] → `Divider` → `Theirs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictMarkerKind {
    /// `<<<<<<< ours` — opens the region; our/local side follows.
    Ours,
    /// `||||||| base` (git diff3) / `||||||| COMMON ANCESTOR …` (Fossil).
    Base,
    /// `=======` / `======= MERGED IN …` — their side follows.
    Divider,
    /// `####### SUGGESTED CONFLICT RESOLUTION follows …` (Fossil only).
    Suggested,
    /// `>>>>>>> theirs` — closes the region.
    Theirs,
}

impl ConflictMarkerKind {
    /// The bare marker token, as reported to the user.
    pub fn token(self) -> &'static str {
        match self {
            ConflictMarkerKind::Ours => "<<<<<<<",
            ConflictMarkerKind::Base => "|||||||",
            ConflictMarkerKind::Divider => "=======",
            ConflictMarkerKind::Suggested => "#######",
            ConflictMarkerKind::Theirs => ">>>>>>>",
        }
    }
}

/// THE scanner for VCS merge-conflict marker lines: `(line index, kind)` for
/// every column-0 marker outside a column-0 fenced code block, in file order.
///
/// Single source of truth — [`vcs_conflict_markers`] (detection/quarantine) and
/// the marker parser ([`parse_vcs_marker_sides`]) both derive from it, so
/// "what counts as a marker" can never diverge between the code that REFUSES to
/// rewrite a file and the code that RESOLVES it. See [`vcs_conflict_markers`]
/// for the recognized dialects and the two false-positive guards.
pub fn scan_vcs_conflict_markers(content: &str) -> Vec<(usize, ConflictMarkerKind)> {
    let mut fence: Option<char> = None;
    let mut out = Vec::new();
    for (index, line) in content.lines().enumerate() {
        if let Some(delimiter) = fence {
            if line.chars().take_while(|&c| c == delimiter).count() >= 3 {
                fence = None;
            }
            continue;
        }
        if line.starts_with("```") {
            fence = Some('`');
            continue;
        }
        if line.starts_with("~~~") {
            fence = Some('~');
            continue;
        }
        let kind = if line.starts_with("<<<<<<< ") {
            Some(ConflictMarkerKind::Ours)
        } else if line.starts_with(">>>>>>> ") {
            Some(ConflictMarkerKind::Theirs)
        } else if line.starts_with("||||||| ") {
            Some(ConflictMarkerKind::Base)
        } else if line == "=======" || line.starts_with("======= ") {
            Some(ConflictMarkerKind::Divider)
        } else if line.starts_with("####### ") {
            Some(ConflictMarkerKind::Suggested)
        } else {
            None
        };
        if let Some(kind) = kind {
            out.push((index, kind));
        }
    }
    out
}

/// Where a conflict object came from. Not a rendering hint — the two sources
/// resolve through different backend paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictSource {
    /// A sync transport left a conflict copy beside the winner page.
    SyncCopy,
    /// A VCS merge left conflict markers inside the page file itself.
    VcsMarkers,
}

/// Which version of the page a side is. Three roles, not two — the base is a
/// first-class side wherever one exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SideRole {
    /// The version Tine/this device has (the winner page, or the `<<<<<<<` half).
    Mine,
    /// The incoming version (the conflict copy, or the `>>>>>>>` half).
    Theirs,
    /// The common ancestor, when one is known (the `|||||||` half).
    Base,
}

/// One version of the page participating in a conflict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictSide {
    pub role: SideRole,
    /// Human label for the side ("This device", the copy's device/timestamp tag,
    /// or the marker's own label line).
    pub label: String,
    /// Graph-root-relative path, when the side IS a file of its own. `None` for
    /// sides that live inside a marker-bearing file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// One item in the conflict queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictObject {
    /// Stable derived identity — `"copy:<conflict path>"` / `"markers:<path>"`.
    /// Recomputing the queue reproduces the same id for the same on-disk state.
    pub id: String,
    pub source: ConflictSource,
    /// Display name of the page in conflict.
    pub page_name: String,
    /// Graph-root-relative path of the page to NAVIGATE to (the winner for a
    /// conflict copy, the marker file itself for a VCS conflict).
    pub page_path: String,
    pub kind: PageKind,
    /// Every known version of the page (2 or 3 entries; never assumed to be 2).
    pub sides: Vec<ConflictSide>,
    /// Number of block rows that need a decision, when it was cheap to compute.
    /// `None` means "not computed" — never "zero".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_conflicts: Option<usize>,
    /// Marker tokens present, for a `VcsMarkers` object (empty otherwise).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<String>,
}

/// A page whose file carries unresolved VCS merge conflict markers. It stays
/// a real, readable page; the store refuses to rewrite it (R-VCS-MARKERS).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VcsMarkerConflict {
    /// Graph-root-relative path of the marker-bearing file.
    pub path: String,
    /// Display name of the page (decoded page name / journal title).
    pub name: String,
    pub kind: PageKind,
    /// Distinct marker tokens found, e.g. `["<<<<<<<", "=======", ">>>>>>>"]`.
    pub markers: Vec<String>,
}

/// One answer for every conflict surface: the two listings and the queue
/// derived from them come from ONE walk, so they cannot disagree and the page
/// files are read once per refresh.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictInventory {
    pub sync_conflicts: Vec<crate::model::SyncConflict>,
    pub vcs_markers: Vec<VcsMarkerConflict>,
    pub queue: Vec<ConflictObject>,
}

/// A marker-bearing page's own conflict, ready for the in-page resolver: the
/// ordinary block diff plus the labels the VCS wrote on the marker lines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkerConflictDiff {
    /// Label from the `<<<<<<<` line (git: the ref; Fossil: a sentence).
    pub mine_label: String,
    /// Label from the `>>>>>>>` line.
    pub theirs_label: String,
    /// How many marker regions the file carries.
    pub regions: usize,
    /// The diff itself — 3-way (with per-row suggestions) whenever the markers
    /// carried a common ancestor.
    pub diff: crate::sync_diff::SyncConflictDiff,
}

/// The whole-page texts reconstructed from a marker-bearing file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerSides {
    /// The file as the `<<<<<<<` (ours/local) side wrote it — markers removed.
    pub mine: String,
    /// The file as the `>>>>>>>` (theirs/merged-in) side wrote it.
    pub theirs: String,
    /// The common ancestor, present only when EVERY region carried a `|||||||`
    /// section (git's `diff3`/`zdiff3` style, or Fossil). A partial ancestor
    /// would make baseless regions look like both sides added text, so it is
    /// all-or-nothing.
    pub base: Option<String>,
    /// The page as the merge tool's own `#######` SUGGESTED CONFLICT RESOLUTION
    /// sections propose it (Fossil): the suggestion text inside every region,
    /// the common text elsewhere. All-or-nothing exactly like [`MarkerSides::base`]
    /// — a page where only SOME regions carried a suggestion would mix a
    /// proposal with unresolved regions, so the whole artifact is dropped.
    ///
    /// It is not a side: nothing here is a version anyone wrote. It is only ever
    /// offered as a `MergedSource::Artifact` proposal the user must confirm.
    pub suggested: Option<String>,
    /// How many conflict regions the file contains.
    pub regions: usize,
    /// Label from the first `<<<<<<<` line (e.g. `HEAD`), best effort.
    pub mine_label: String,
    /// Label from the first `>>>>>>>` line (e.g. the incoming branch).
    pub theirs_label: String,
}

/// Section of a conflict region a line belongs to while walking the file.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    /// Outside any region — the line belongs to every side.
    Outside,
    Ours,
    Base,
    Theirs,
    /// Fossil's precomputed suggestion. It is a DERIVATION of the two sides,
    /// not a side, so it contributes to none of the three reconstructed SIDES —
    /// it feeds only [`MarkerSides::suggested`].
    Suggested,
}

/// Reconstruct the sides of a marker-bearing file.
///
/// Returns `None` when the content is not conflicted by the shared scanner's
/// rules, or when the marker structure is malformed (unclosed region, `=======`
/// with no open region, …) — a malformed file is left strictly alone rather than
/// guessed at, so invariant 3 (never rewrite a marker file except as the direct
/// result of a resolution) can never be violated on a file we misread.
pub fn parse_vcs_marker_sides(content: &str) -> Option<MarkerSides> {
    if vcs_conflict_markers(content).is_empty() {
        return None;
    }
    let markers = scan_vcs_conflict_markers(content);
    let mut by_line = std::collections::HashMap::new();
    for (index, kind) in &markers {
        by_line.insert(*index, *kind);
    }
    let mut mine = String::new();
    let mut theirs = String::new();
    let mut base = String::new();
    let mut suggested = String::new();
    let mut section = Section::Outside;
    let mut regions = 0usize;
    // A base section is only trustworthy if every region supplies one.
    let mut regions_with_base = 0usize;
    let mut region_had_base = false;
    // Same all-or-nothing rule for the merge tool's suggestion.
    let mut regions_with_suggestion = 0usize;
    let mut region_had_suggestion = false;
    let mut mine_label = String::new();
    let mut theirs_label = String::new();
    for (index, line) in content.lines().enumerate() {
        match by_line.get(&index) {
            Some(ConflictMarkerKind::Ours) => {
                if section != Section::Outside {
                    return None; // nested / unclosed region
                }
                section = Section::Ours;
                regions += 1;
                region_had_base = false;
                region_had_suggestion = false;
                if mine_label.is_empty() {
                    mine_label = marker_label(line);
                }
                continue;
            }
            Some(ConflictMarkerKind::Suggested) => {
                if !matches!(section, Section::Ours) {
                    return None;
                }
                section = Section::Suggested;
                region_had_suggestion = true;
                continue;
            }
            Some(ConflictMarkerKind::Base) => {
                if !matches!(section, Section::Ours | Section::Suggested) {
                    return None;
                }
                section = Section::Base;
                region_had_base = true;
                continue;
            }
            Some(ConflictMarkerKind::Divider) => {
                if !matches!(section, Section::Ours | Section::Suggested | Section::Base) {
                    // A `=======` outside a region is a divider, not a marker —
                    // but the anchored file we are parsing puts us in "malformed"
                    // territory, so refuse rather than mis-split the page.
                    return None;
                }
                section = Section::Theirs;
                continue;
            }
            Some(ConflictMarkerKind::Theirs) => {
                if section != Section::Theirs {
                    return None;
                }
                section = Section::Outside;
                if region_had_base {
                    regions_with_base += 1;
                }
                if region_had_suggestion {
                    regions_with_suggestion += 1;
                }
                if theirs_label.is_empty() {
                    theirs_label = marker_label(line);
                }
                continue;
            }
            None => {}
        }
        match section {
            Section::Outside => {
                push_line(&mut mine, line);
                push_line(&mut theirs, line);
                push_line(&mut base, line);
                push_line(&mut suggested, line);
            }
            Section::Ours => push_line(&mut mine, line),
            Section::Theirs => push_line(&mut theirs, line),
            Section::Base => push_line(&mut base, line),
            // Feeds the artifact proposal ONLY — never any reconstructed side.
            Section::Suggested => push_line(&mut suggested, line),
        }
    }
    if section != Section::Outside || regions == 0 {
        return None;
    }
    Some(MarkerSides {
        mine,
        theirs,
        base: (regions_with_base == regions).then_some(base),
        suggested: (regions_with_suggestion == regions).then_some(suggested),
        regions,
        mine_label,
        theirs_label,
    })
}

fn push_line(out: &mut String, line: &str) {
    out.push_str(line);
    out.push('\n');
}

/// The label after a marker's leading run of `<`/`>`/`|`/`=` (git writes the ref
/// or "ours"/"theirs"; Fossil writes a sentence). Fossil repeats the marker run
/// at the end of the line, so trim that too.
fn marker_label(line: &str) -> String {
    let rest = line.trim_start_matches(['<', '>', '|', '=', '#']).trim();
    rest.trim_end_matches(['<', '>', '|', '=', '#'])
        .trim()
        .to_string()
}

/// Count the rows a user would have to decide on (recursively) — the "n
/// conflicts" figure shown per queue item and in the in-page counter.
pub fn decidable_row_count(rows: &[crate::sync_diff::DiffRow]) -> usize {
    rows.iter()
        .map(|row| {
            usize::from(row.kind != crate::sync_diff::RowKind::Unchanged)
                + decidable_row_count(&row.children)
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    // A git conflict, two-way style (`merge.conflictStyle = merge`).
    const GIT_2WAY: &str = "- shared top\n<<<<<<< HEAD\n- mine wins\n=======\n- theirs wins\n>>>>>>> feature\n- shared bottom\n";

    // The same conflict in git's diff3 style — the ancestor section is present.
    const GIT_DIFF3: &str = "- shared top\n<<<<<<< HEAD\n- mine wins\n||||||| merged common ancestors\n- original\n=======\n- theirs wins\n>>>>>>> feature\n- shared bottom\n";

    #[test]
    fn parses_a_two_way_git_conflict_into_two_full_pages() {
        let sides = parse_vcs_marker_sides(GIT_2WAY).expect("conflicted");
        assert_eq!(sides.mine, "- shared top\n- mine wins\n- shared bottom\n");
        assert_eq!(
            sides.theirs,
            "- shared top\n- theirs wins\n- shared bottom\n"
        );
        assert_eq!(sides.base, None);
        assert_eq!(sides.regions, 1);
        assert_eq!(sides.mine_label, "HEAD");
        assert_eq!(sides.theirs_label, "feature");
    }

    #[test]
    fn parses_a_diff3_conflict_and_recovers_the_ancestor() {
        let sides = parse_vcs_marker_sides(GIT_DIFF3).expect("conflicted");
        assert_eq!(sides.mine, "- shared top\n- mine wins\n- shared bottom\n");
        assert_eq!(
            sides.theirs,
            "- shared top\n- theirs wins\n- shared bottom\n"
        );
        assert_eq!(
            sides.base.as_deref(),
            Some("- shared top\n- original\n- shared bottom\n")
        );
    }

    #[test]
    fn parses_a_fossil_conflict_and_drops_its_suggestion_from_every_side() {
        // fossil src/merge3.c `mergeMarker` wording.
        let fossil = concat!(
            "- shared\n",
            "<<<<<<< BEGIN MERGE CONFLICT: local copy shown first <<<<<<<<<<<<<<<\n",
            "- mine wins\n",
            "####### SUGGESTED CONFLICT RESOLUTION follows ##################\n",
            "- a guess nobody asked for\n",
            "||||||| COMMON ANCESTOR content follows |||||||||||||||||||||||||\n",
            "- original\n",
            "======= MERGED IN content follows ==============================\n",
            "- theirs wins\n",
            ">>>>>>> END MERGE CONFLICT >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>\n",
        );
        let sides = parse_vcs_marker_sides(fossil).expect("conflicted");
        assert_eq!(sides.mine, "- shared\n- mine wins\n");
        assert_eq!(sides.theirs, "- shared\n- theirs wins\n");
        assert_eq!(sides.base.as_deref(), Some("- shared\n- original\n"));
        for text in [&sides.mine, &sides.theirs, sides.base.as_ref().unwrap()] {
            assert!(
                !text.contains("nobody asked"),
                "Fossil's suggestion is a derivation, not a side: {text:?}"
            );
        }
        // It is not dropped on the floor either — it reconstructs a fourth,
        // clearly-labelled document that only the merge PROPOSAL may use.
        assert_eq!(
            sides.suggested.as_deref(),
            Some("- shared\n- a guess nobody asked for\n")
        );
    }

    #[test]
    fn a_fossil_region_reconstructs_all_four_documents() {
        // Fossil writes the sections in this order (src/merge3.c): ours,
        // suggestion, common ancestor, merged-in. Verbose marker lines, several
        // lines per section, shared text on both sides of the region.
        let fossil = concat!(
            "- top shared\n",
            "- second shared\n",
            "<<<<<<< BEGIN MERGE CONFLICT: local copy shown first <<<<<<<<<<<<<<<\n",
            "- alpha mine\n",
            "- beta\n",
            "####### SUGGESTED CONFLICT RESOLUTION follows ##################\n",
            "- alpha mine\n",
            "- beta theirs\n",
            "||||||| COMMON ANCESTOR content follows |||||||||||||||||||||||||\n",
            "- alpha\n",
            "- beta\n",
            "======= MERGED IN content follows ==============================\n",
            "- alpha\n",
            "- beta theirs\n",
            ">>>>>>> END MERGE CONFLICT >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>\n",
            "- bottom shared\n",
        );
        let sides = parse_vcs_marker_sides(fossil).expect("conflicted");
        let shared = |body: &str| format!("- top shared\n- second shared\n{body}- bottom shared\n");
        assert_eq!(sides.mine, shared("- alpha mine\n- beta\n"));
        assert_eq!(sides.theirs, shared("- alpha\n- beta theirs\n"));
        assert_eq!(
            sides.base.as_deref(),
            Some(shared("- alpha\n- beta\n")).as_deref()
        );
        assert_eq!(
            sides.suggested.as_deref(),
            Some(shared("- alpha mine\n- beta theirs\n")).as_deref()
        );
        assert_eq!(sides.regions, 1);
    }

    #[test]
    fn a_suggestion_in_only_some_regions_is_dropped_for_the_whole_page() {
        // Same all-or-nothing rule as the ancestor: a partial artifact would
        // propose a resolution for one region and leave the other conflicted.
        let two = concat!(
            "<<<<<<< HEAD\n- a1\n",
            "####### SUGGESTED CONFLICT RESOLUTION follows ###\n- a merged\n",
            "||||||| base\n- a0\n=======\n- a2\n>>>>>>> x\n",
            "- middle\n",
            "<<<<<<< HEAD\n- b1\n||||||| base\n- b0\n=======\n- b2\n>>>>>>> x\n",
        );
        let sides = parse_vcs_marker_sides(two).expect("conflicted");
        assert_eq!(sides.regions, 2);
        assert_eq!(sides.suggested, None);
        // The sides and the ancestor are unaffected, and still carry no
        // suggestion text.
        assert_eq!(sides.mine, "- a1\n- middle\n- b1\n");
        assert_eq!(sides.theirs, "- a2\n- middle\n- b2\n");
        assert_eq!(sides.base.as_deref(), Some("- a0\n- middle\n- b0\n"));
    }

    #[test]
    fn every_region_carrying_a_suggestion_reconstructs_the_whole_page() {
        let two = concat!(
            "<<<<<<< HEAD\n- a1\n",
            "####### SUGGESTED CONFLICT RESOLUTION follows ###\n- a merged\n",
            "||||||| base\n- a0\n=======\n- a2\n>>>>>>> x\n",
            "- middle\n",
            "<<<<<<< HEAD\n- b1\n",
            "####### SUGGESTED CONFLICT RESOLUTION follows ###\n- b merged\n",
            "||||||| base\n- b0\n=======\n- b2\n>>>>>>> x\n",
        );
        let sides = parse_vcs_marker_sides(two).expect("conflicted");
        assert_eq!(
            sides.suggested.as_deref(),
            Some("- a merged\n- middle\n- b merged\n")
        );
    }

    #[test]
    fn a_git_conflict_offers_no_artifact() {
        // Only Fossil writes the section; git's two styles never do.
        assert_eq!(parse_vcs_marker_sides(GIT_2WAY).unwrap().suggested, None);
        assert_eq!(parse_vcs_marker_sides(GIT_DIFF3).unwrap().suggested, None);
    }

    #[test]
    fn several_regions_all_contribute_and_a_partial_ancestor_is_refused() {
        let two = concat!(
            "<<<<<<< HEAD\n- a1\n=======\n- a2\n>>>>>>> x\n",
            "- middle\n",
            "<<<<<<< HEAD\n- b1\n||||||| base\n- b0\n=======\n- b2\n>>>>>>> x\n",
        );
        let sides = parse_vcs_marker_sides(two).expect("conflicted");
        assert_eq!(sides.regions, 2);
        assert_eq!(sides.mine, "- a1\n- middle\n- b1\n");
        assert_eq!(sides.theirs, "- a2\n- middle\n- b2\n");
        // Region 1 has no ancestor → the reconstructed base would claim both
        // sides ADDED `- a1`/`- a2`, so no base is offered at all.
        assert_eq!(sides.base, None);
    }

    #[test]
    fn a_page_merely_documenting_markers_is_not_parsed() {
        let fenced = "- how git marks conflicts:\n```\n<<<<<<< HEAD\nmine\n=======\ntheirs\n>>>>>>> other\n```\n";
        assert!(vcs_conflict_markers(fenced).is_empty());
        assert_eq!(parse_vcs_marker_sides(fenced), None);
        assert_eq!(parse_vcs_marker_sides("- plain page\n"), None);
    }

    #[test]
    fn malformed_marker_structure_is_refused_rather_than_guessed() {
        // Unclosed region.
        assert_eq!(
            parse_vcs_marker_sides("<<<<<<< HEAD\n- mine\n=======\n- theirs\n"),
            None
        );
        // Nested opener.
        assert_eq!(
            parse_vcs_marker_sides("<<<<<<< HEAD\n<<<<<<< HEAD\n- x\n=======\n- y\n>>>>>>> b\n"),
            None
        );
        // Closer with no region open.
        assert_eq!(parse_vcs_marker_sides("- x\n>>>>>>> b\n"), None);
    }

    #[test]
    fn a_diff3_regions_ancestor_drives_per_block_suggestions() {
        // A git region spans whole HUNKS, so it routinely contains blocks only
        // ONE side touched. The recovered ancestor turns those into confident
        // per-block suggestions — the whole point of parsing markers rather than
        // just showing two columns.
        let content = concat!(
            "- shared top\n",
            "<<<<<<< HEAD\n- alpha edited by me\n- beta\n",
            "||||||| merged common ancestors\n- alpha\n- beta\n",
            "=======\n- alpha\n- beta edited by them\n",
            ">>>>>>> feature\n",
            "- shared bottom\n",
        );
        let sides = parse_vcs_marker_sides(content).expect("conflicted");
        let diff = crate::sync_diff::diff3_docs(
            &crate::doc::parse(sides.base.as_deref().expect("ancestor recovered")),
            &crate::doc::parse(&sides.mine),
            &crate::doc::parse(&sides.theirs),
        );
        assert!(diff.three_way);
        let suggestions: Vec<_> = diff
            .rows
            .iter()
            .filter(|r| r.kind != crate::sync_diff::RowKind::Unchanged)
            .map(|r| r.suggestion.clone())
            .collect();
        // EVERY decidable row is decided by the ancestor — nothing is left as a
        // true both-changed conflict, so the gesture is glance-and-confirm.
        assert_eq!(suggestions.len(), decidable_row_count(&diff.rows));
        assert!(suggestions.iter().all(Option::is_some), "{suggestions:?}");
        assert!(suggestions.iter().any(|s| s.as_deref() == Some("mine")));
        assert!(suggestions.iter().any(|s| s.as_deref() == Some("theirs")));
    }

    #[test]
    fn a_two_way_marker_file_still_yields_a_reviewable_diff() {
        let sides = parse_vcs_marker_sides(GIT_2WAY).expect("conflicted");
        assert!(sides.base.is_none());
        let diff = crate::sync_diff::diff_docs(
            &crate::doc::parse(&sides.mine),
            &crate::doc::parse(&sides.theirs),
        );
        assert!(!diff.three_way);
        assert!(!diff.blocks_identical);
        assert!(decidable_row_count(&diff.rows) > 0);
    }

    #[test]
    fn vcs_marker_detection_matches_real_markers_only() {
        // git (merge and diff3 styles).
        assert_eq!(
                vcs_conflict_markers(
                    "<<<<<<< HEAD\n- mine\n||||||| merged common ancestors\n- old\n=======\n- theirs\n>>>>>>> feature\n"
                ),
                vec!["<<<<<<<", "|||||||", "=======", ">>>>>>>"]
            );
        // Fossil's verbose variants (mergeMarker table in fossil src/merge3.c).
        assert_eq!(
            vcs_conflict_markers(concat!(
                "<<<<<<< BEGIN MERGE CONFLICT: local copy shown first <<<<<<<<<<<<\n",
                "- mine\n",
                "####### SUGGESTED CONFLICT RESOLUTION follows ###################\n",
                "- suggestion\n",
                "||||||| COMMON ANCESTOR content follows |||||||||||||||||||||||||\n",
                "- old\n",
                "======= MERGED IN content follows ===============================\n",
                "- theirs\n",
                ">>>>>>> END MERGE CONFLICT >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>> (line 3)\n"
            )),
            vec!["<<<<<<<", "#######", "|||||||", "=======", ">>>>>>>"]
        );
        // Markers quoted inside a column-0 fenced code block (someone
        // DOCUMENTING git) must not flag the page.
        assert!(vcs_conflict_markers(
            "```\n<<<<<<< HEAD\n=======\n>>>>>>> feature\n```\n- notes about git\n"
        )
        .is_empty());
        assert!(vcs_conflict_markers("~~~text\n<<<<<<< HEAD\n>>>>>>> feature\n~~~\n").is_empty());
        // Markers quoted in an indented fence inside a bullet are not at
        // column 0 at all.
        assert!(vcs_conflict_markers(
            "- how git conflicts look:\n  ```\n  <<<<<<< HEAD\n  =======\n  >>>>>>> theirs\n  ```\n"
        )
        .is_empty());
        // A lone `=======` (setext-style divider) never quarantines a page —
        // an anchor marker must be present.
        assert!(vcs_conflict_markers("Heading\n=======\n- content\n").is_empty());
        // Markers must start at column 0 with their trailing space/shape.
        assert!(vcs_conflict_markers("- <<<<<<< HEAD\n- >>>>>>> x\n").is_empty());
        // A real conflict below a closed fence is still detected.
        assert_eq!(
            vcs_conflict_markers(
                "```\nexample\n```\n<<<<<<< HEAD\n- mine\n=======\n- theirs\n>>>>>>> feature\n"
            ),
            vec!["<<<<<<<", "=======", ">>>>>>>"]
        );
    }
}
