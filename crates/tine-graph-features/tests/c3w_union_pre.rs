//! C3W W5 (L03): resolving a conflict with the page's own properties set to
//! "both (merge)" (the default) must keep what only the conflict copy's
//! pre-block holds: its free text and its tag/alias members. A value both sides
//! set differently cannot be kept twice, so the resolve refuses and asks for an
//! explicit choice instead of silently dropping the copy's value.
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use tine_core::sync_diff::DiffRow;
use tine_graph_features::conflicts;
use tine_store::Store;

fn scratch() -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-c3w-w5-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    root
}

fn all_rows(rows: &[DiffRow], out: &mut HashMap<String, String>) {
    for row in rows {
        out.insert(row.id.clone(), "mine".into());
        all_rows(&row.children, out);
    }
}

/// Resolve with the pre-block choice "union"; returns the result and the
/// winner's bytes afterwards.
fn resolve_union(ext: &str, winner: &str, copy: &str) -> (std::io::Result<()>, String) {
    let root = scratch();
    let win = format!("pages/A.{ext}");
    let conf = format!("pages/A.sync-conflict-20260929-101010-ABCDEFG.{ext}");
    fs::write(root.join(&win), winner).unwrap();
    fs::write(root.join(&conf), copy).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let diff = conflicts::sync_conflict_diff(&store, &win, &conf, &[])
        .unwrap()
        .unwrap();
    assert!(diff.pre_differs);
    let mut decisions = HashMap::new();
    all_rows(&diff.rows, &mut decisions);
    let result = conflicts::resolve_sync_conflict(
        &store,
        None,
        &win,
        &conf,
        &decisions,
        &diff.base_rev,
        &diff.conflict_rev,
        None,
        &[],
        "union",
    );
    let out = fs::read_to_string(root.join(&win)).unwrap();
    assert_eq!(
        result.is_ok(),
        !root.join(&conf).exists(),
        "the copy is trashed exactly when the resolve succeeds"
    );
    let _ = fs::remove_dir_all(&root);
    (result, out)
}

#[test]
fn w5_union_keeps_the_copys_free_text_and_tag_members() {
    let (result, out) = resolve_union(
        "md",
        "tags:: x\nicon:: 🙂\n\n- block\n",
        "tags:: y, x\nicon:: 🙂\nstatus:: draft\na note written on the other device\n\n- block\n",
    );
    result.unwrap();
    assert!(out.contains("a note written on the other device"), "{out}");
    assert!(out.contains("status:: draft"), "{out}");
    let tags = out.lines().find(|l| l.starts_with("tags::")).unwrap();
    assert!(tags.contains('x') && tags.contains('y'), "{out}");
    assert_eq!(out.matches("tags::").count(), 1, "{out}");
    // Page properties stay the leading lines of the pre-block.
    let props: Vec<_> = out.lines().take(3).collect();
    assert!(props.iter().all(|l| l.contains(":: ")), "{out}");
}

#[test]
fn w5_union_never_drops_a_conflicting_value_silently() {
    let winner = "icon:: 🙂\n\n- block\n";
    let (result, out) = resolve_union("md", winner, "icon:: 🎉\n\n- block\n");
    match result {
        Err(error) => {
            assert!(error.to_string().contains("icon"), "{error}");
            assert_eq!(out, winner, "a refused resolve writes nothing");
        }
        Ok(()) => assert!(out.contains("🎉"), "{out}"),
    }
}

#[test]
fn w5_org_union_keeps_the_copys_pre_block_lines() {
    let (result, out) = resolve_union(
        "org",
        "#+title: A\n\n* block\n",
        "#+title: A\n#+filetags: :x:\nnote from there\n\n* block\n",
    );
    result.unwrap();
    assert!(out.contains("#+filetags: :x:"), "{out}");
    assert!(out.contains("note from there"), "{out}");
}

// OG-P12B (D06, I-12): the page preamble is read through the parser's regions,
// so a property-looking line inside a literal container is code, not a page
// property, and a container is one atomic unit.
#[test]
fn p12b_property_shaped_fence_content_is_not_a_clashing_property() {
    // `icon:: 🎉` sits inside the copy's fence. As a property it would clash
    // with mine's `icon:: 🙂` and refuse the merge; as code it is just kept.
    let (result, out) = resolve_union(
        "md",
        "icon:: 🙂\n\n- block\n",
        "icon:: 🙂\n```\nicon:: 🎉\n```\n\n- block\n",
    );
    result.unwrap();
    assert!(out.contains("```\nicon:: 🎉\n```"), "{out}");
    assert_eq!(out.matches("icon:: 🙂").count(), 1, "{out}");
}

#[test]
fn p12b_different_containers_are_kept_whole_mine_first_and_identical_ones_dedupe() {
    let (result, out) = resolve_union(
        "md",
        "tags:: x\n```\nmine line\nshared\n```\n\n- block\n",
        "tags:: x\n```\ntheirs line\nshared\n```\n```\nmine line\nshared\n```\n\n- block\n",
    );
    result.unwrap();
    let (mine_at, theirs_at) = (
        out.find("```\nmine line\nshared\n```").expect(&out),
        out.find("```\ntheirs line\nshared\n```").expect(&out),
    );
    assert!(mine_at < theirs_at, "mine's container first: {out}");
    // Never line-merged: the shared line is not de-duplicated across containers,
    // and mine's identical container is not repeated.
    assert_eq!(out.matches("shared").count(), 2, "{out}");
    assert_eq!(out.matches("mine line").count(), 1, "{out}");
}

#[test]
fn p12b_org_directive_inside_src_is_code_not_a_clashing_property() {
    let (result, out) = resolve_union(
        "org",
        "#+alias: Real\n\n* block\n",
        "#+alias: Real\n#+BEGIN_SRC text\n#+alias: Ghost\n#+END_SRC\n\n* block\n",
    );
    result.unwrap();
    assert!(
        out.contains("#+BEGIN_SRC text\n#+alias: Ghost\n#+END_SRC"),
        "{out}"
    );
    assert_eq!(out.matches("#+alias: Real").count(), 1, "{out}");
}

#[test]
fn p12b_a_real_clash_outside_a_container_still_refuses() {
    let winner = "icon:: 🙂\n```\nx\n```\n\n- block\n";
    let (result, out) = resolve_union("md", winner, "icon:: 🎉\n```\nx\n```\n\n- block\n");
    let error = result.expect_err("an un-keepable property value still refuses");
    assert!(error.to_string().contains("icon"), "{error}");
    assert_eq!(out, winner);
}
