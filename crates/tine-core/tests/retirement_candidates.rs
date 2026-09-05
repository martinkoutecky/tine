//! The `// RETIREMENT-CANDIDATE:` census.
//!
//! Martin, 2026-09-03: *"also write it in the code, so that a future sweep
//! discovers 'there is a comment saying this should get deleted eventually; is
//! now the time?'"* The convention only works if the set of markers is pinned:
//! an unpinned marker can be added without anyone agreeing that the code is
//! temporary, and — much worse — can be silently deleted along with the code it
//! was warning about. Both are deliberate acts, so both fail here first.
//!
//! This is the first deliberate use of the convention, so the exemplar is the
//! `simple_query_candidate_paths` hatch in `direct_projection.rs`. Copy its
//! shape: WHAT may be deleted, the CONDITION that makes it deletable, and WHAT
//! CURRENTLY BLOCKS deletion — the last being the part a future sweep actually
//! needs, because a marker that only says "delete me eventually" tells the
//! sweep nothing about whether now is the time.
//!
//! Pinned by `(file, marker text)` and never by line number: a line-anchored
//! pin reddens on every unrelated packet that edits the file above it, which is
//! a tax with no signal.

#[path = "support/production_source.rs"]
mod production_source;

use production_source::{compiled_source, production_source_files, relative_path, repo_root};
use std::collections::BTreeSet;

const MARKER: &str = "// RETIREMENT-CANDIDATE:";

/// The exact `(file, first line of the marker)` set. Adding or removing a row
/// is the deliberate act this guard exists to force.
const PINNED: &[(&str, &str)] = &[
    (
        "crates/tine-core/src/direct_projection.rs",
        "// RETIREMENT-CANDIDATE: the candidate-count escape hatch below, together",
    ),
    (
        "crates/tine-core/src/query.rs",
        "// RETIREMENT-CANDIDATE: the pre-SQL candidate planner for the walk.",
    ),
    (
        "crates/tine-core/src/query.rs",
        "// RETIREMENT-CANDIDATE: the in-memory query walk.",
    ),
];

fn markers() -> BTreeSet<(String, String)> {
    let root = repo_root();
    let mut found = BTreeSet::new();
    for path in production_source_files() {
        let relative = relative_path(&root, &path);
        for line in compiled_source(&path).lines() {
            let trimmed = line.trim();
            if trimmed.starts_with(MARKER) {
                assert!(
                    found.insert((relative.clone(), trimmed.to_owned())),
                    "two {MARKER} markers in {relative} open with the same line; \
                     make the first line name the specific thing being retired"
                );
            }
        }
    }
    found
}

#[test]
fn retirement_candidate_markers_are_pinned() {
    let expected = PINNED
        .iter()
        .map(|(file, marker)| ((*file).to_owned(), (*marker).to_owned()))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        markers(),
        expected,
        "the `{MARKER}` set changed.\n\
         \n\
         The convention: a `{MARKER}` comment marks production code that is \
         known to be temporary, so a future sweep can ask \"is now the time to \
         delete this?\" and get an answer instead of a guess. Every marker \
         states three things — WHAT may be deleted, the CONDITION under which \
         it becomes deletable, and WHAT CURRENTLY BLOCKS deletion. The blocker \
         is the load-bearing part; without it the sweep cannot tell a ready \
         retirement from a premature one.\n\
         \n\
         Adding a marker and deleting one are both deliberate acts, so both \
         update this list. If you deleted the code, delete its row. If you \
         added a marker, add its row and copy the exemplar's shape: \
         `crates/tine-core/src/direct_projection.rs`, on the candidate-count \
         escape hatch in `simple_query_candidate_paths`.\n\
         \n\
         I-12: this census uses the one production-source scanner \
         (`tests/support/production_source.rs`); do not add a second walker."
    );
}

/// A marker that only announces itself is worthless to the sweep that finds it.
#[test]
fn every_retirement_candidate_states_condition_and_blocker() {
    let root = repo_root();
    for (file, marker) in PINNED {
        let source = compiled_source(&root.join(file));
        let start = source
            .find(marker)
            .unwrap_or_else(|| panic!("{file} lost the pinned marker {marker}"));
        // The note runs to the end of the contiguous comment block.
        let note = source[start..]
            .lines()
            .take_while(|line| line.trim().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for required in [
            "WHAT MAY BE DELETED",
            "CONDITION FOR DELETION",
            "WHAT CURRENTLY BLOCKS DELETION",
        ] {
            assert!(
                note.contains(required),
                "{file}: the {MARKER} note must state `{required}`. A future \
                 sweep reads this note to decide whether now is the time; a \
                 marker without all three parts cannot answer that."
            );
        }
        assert!(
            note.len() > 400,
            "{file}: the {MARKER} note is too short to carry its three parts"
        );
    }
}

/// The one blocker this packet's marker records: the parser walk is the
/// correctness ORACLE for the lowering that would replace it, so it outlives
/// that lowering by at least one release. A sweep that deletes the walk the
/// moment SQL works removes the only way to prove SQL right.
///
/// Martin, 2026-09-03, card `PVTI_lAHOAAbLVc4BhPsyzg5VyLk`. Asserted rather
/// than merely written, because this is exactly the reasoning a later reader
/// would otherwise have to reconstruct — and would get wrong.
#[test]
fn the_query_hatch_marker_records_the_walk_as_the_correctness_oracle() {
    let root = repo_root();
    let source = compiled_source(&root.join("crates/tine-core/src/direct_projection.rs"));
    let start = source.find(MARKER).expect("the exemplar marker");
    let note = source[start..]
        .lines()
        .take_while(|line| line.trim().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for required in [
        "CORRECTNESS ORACLE",
        "no external oracle exists",
        "DIFFERENTIAL AGAINST THE WALK",
        "outlives the lowering by at least one",
        "sparse_task_query_runner_matches_existing_page_evaluator",
        "PVTI_lAHOAAbLVc4BhPsyzg5VyLk",
    ] {
        assert!(
            note.contains(required),
            "the query hatch's {MARKER} note must record `{required}`: the \
             walk is not merely the fallback, it is the acceptance gate for \
             the lowering that replaces it, and retiring the hatch is NOT \
             permission to retire the walk."
        );
    }
    // The named oracle must actually exist, or the marker points at nothing.
    let query = std::fs::read_to_string(root.join("crates/tine-core/src/query.rs")).unwrap();
    assert!(
        query.contains("fn sparse_task_query_runner_matches_existing_page_evaluator("),
        "the marker's cited differential oracle test no longer exists; \
         relocate it by symbol and update the note"
    );
}
