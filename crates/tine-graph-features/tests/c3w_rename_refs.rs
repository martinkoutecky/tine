//! C3W W3 (L03): rename rewrites every reference to the old name that OG's
//! rename rewrites. A nested `[[a [[Target]] c]]` used to lose its inner
//! reference, and a `tags::` value split with the full-width `，` (which the
//! reference evidence counts) was never rewritten.
//!
//! Conceptual model step 2 (batch 6) corrected what the test first claimed
//! about an unmatched `[[`: the parser, not a text scan, says what is a
//! reference, and mldoc 1.5.9 (checked through the lsdoc harness) reads
//! `use [[ to link, then [[Target]] here` as ONE page link named
//! ` to link, then [[Target`, `#[[ opener then #[[Target]]` as ONE tag named
//! ` opener then #[[Target`, and the Org `[[ x and [[file:./Target.org][Target]]`
//! as one labelled link whose target is ` x and [[file:./Target.org`. OG's
//! `rename-update-refs!` (`frontend/handler/page.cljs:393` at `6e7afa8eb`)
//! rewrites only blocks in the renamed page's `:block/_refs`; that link does
//! not reference Target, so OG leaves those spans alone, and Tine renders
//! them as that one link or tag too.
use std::fs;
use std::path::PathBuf;
use tine_graph_features::pages;
use tine_store::Store;

#[test]
fn rn_c_rename_keeps_org_inline_literals_and_updates_file_links() {
    let _integration_guard = crate::integration_lock();
    for (ext, held, namespace) in [
        ("org", false, false),
        ("ORG", false, false),
        ("Org", false, false),
        ("ORG", true, false),
        ("Org", true, false),
        ("ORG", false, true),
    ] {
        let rel = format!("pages/Notes.{ext}");
        let (root, store) = fixture(&[
            ("pages/Target.md", "- target\n"),
            (&rel, "* [[Target]] and =[[Target]]= and ~[[Target]]~\n* [[file:./Target.org][target file]]\n"),
        ]);
        if namespace {
            fs::write(root.join("pages/Target___child.md"), "- descendant\n").unwrap();
            store.refresh(tine_store::Depth::Bytes).unwrap();
        }
        hosted(&store, |host| {
            if held {
                let window = serde_json::to_value(host.window_reloaded()).unwrap();
                host.open(
                    window["session"].as_u64().unwrap(),
                    window["nextId"].as_u64().unwrap(),
                    &tine_store::PageId::from(rel.as_str()),
                    "Notes",
                )
                .unwrap();
            }
            pages::rename_or_merge_page(&store, host, "Target", "Renamed", None, None, &[])
        })
        .unwrap();
        if namespace {
            assert!(root.join("pages/Renamed___child.md").exists());
        }
        assert_eq!(fs::read_to_string(root.join(&rel)).unwrap(),
            "* [[Renamed]] and =[[Target]]= and ~[[Target]]~\n* [[file:./Renamed.org][target file]]\n", "{ext}");
    }
}

/// Renames are the page host's (STEP3 §7): each runs through a host started
/// for it under its own app data, as the app's graph binding runs one.
fn hosted<T>(
    store: &std::sync::Arc<tine_store::Store>,
    run: impl FnOnce(&tine_store::PageHost) -> T,
) -> T {
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
    run(&host)
}

fn fixture(files: &[(&str, &str)]) -> (PathBuf, std::sync::Arc<Store>) {
    let root = crate::test_support::fresh_dir("tine-c3w-w3-");
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(
        root.join("logseq/config.edn"),
        "{:file/name-format :triple-lowbar}\n",
    )
    .unwrap();
    for (rel, body) in files {
        fs::write(root.join(rel), body).unwrap();
    }
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    (root, store)
}

#[test]
fn w3_rename_rewrites_references_after_an_unmatched_opener_and_fullwidth_tags() {
    let _integration_guard = crate::integration_lock();
    let (root, store) = fixture(&[
        ("pages/Target.md", "- the target\n"),
        (
            "pages/Referrer.md",
            "- use [[ to link, then [[Target]] here\n\
             - a tag #[[ opener then #[[Target]] too\n\
             - nested [[a [[Target]] c]]\n\
             - tagged\n  tags:: Other，Target\n\
             - plain [[Target]]\n",
        ),
        (
            "pages/Org.org",
            "* see [[ x and [[file:./Target.org][Target]] and [[Target]]\n",
        ),
    ]);
    hosted(&store, |host| {
        pages::rename_or_merge_page(&store, host, "Target", "Renamed", None, None, &[])
    })
    .unwrap();
    assert!(root.join("pages/Renamed.md").exists());
    let referrer = fs::read_to_string(root.join("pages/Referrer.md")).unwrap();
    assert_eq!(
        referrer,
        // The first two lines hold no reference to Target (see the module
        // comment): left as written.
        "- use [[ to link, then [[Target]] here\n\
         - a tag #[[ opener then #[[Target]] too\n\
         - nested [[a [[Renamed]] c]]\n\
         - tagged\n  tags:: Other，Renamed\n\
         - plain [[Renamed]]\n"
    );
    let org = fs::read_to_string(root.join("pages/Org.org")).unwrap();
    // The labelled link's target is ` x and [[file:./Target.org` (see the
    // module comment): it is not a reference to Target, so it stays; the
    // plain `[[Target]]` after it is one and is rewritten.
    assert_eq!(
        org, "* see [[ x and [[file:./Target.org][Target]] and [[Renamed]]\n",
        "{org}"
    );
    let _ = fs::remove_dir_all(&root);
}

/// CM step 2 batch 8 (`REG-CM-PREAMBLE-PROSE-REFERENCES`; flips batch 6's
/// declared limit): prose before the first bullet is the page's first block,
/// as in OG (graph-parser `with-pre-block-if-exists`; `rename-update-refs!`
/// walks `:block/_refs`, which holds the pre-block), so a rename rewrites a
/// link there on every page: one holding only prose, one whose blocks also
/// link the page, one held open in a window, one with page properties above
/// the prose (they are rewritten as properties), and an Org page's text
/// before its first heading. A link in a fenced code block there stays.
#[test]
fn a_link_in_prose_before_the_first_bullet_is_rewritten_on_every_page() {
    let _integration_guard = crate::integration_lock();
    let (root, store) = fixture(&[
        ("pages/Target.md", "- target\n"),
        ("pages/Prose.md", "[[Target]] in prose\n"),
        (
            "pages/Mixed.md",
            "[[Target]] in prose\n\n- [[Target]] in a block\n",
        ),
        ("pages/Held.md", "[[Target]] held\n\n- [[Target]] block\n"),
        (
            "pages/Props.md",
            "tags:: [[Target]]\n\nsee #Target and [[Target]]\n\n```\n[[Target]]\n```\n\n- body\n",
        ),
        (
            "pages/OrgProse.org",
            "#+title: OrgProse\n\nsee [[Target]] here\n* heading\n",
        ),
    ]);
    hosted(&store, |host| {
        let window = serde_json::to_value(host.window_reloaded()).unwrap();
        host.open(
            window["session"].as_u64().unwrap(),
            window["nextId"].as_u64().unwrap(),
            &tine_store::PageId::from("pages/Held.md"),
            "Held",
        )
        .unwrap();
        pages::rename_or_merge_page(&store, host, "Target", "Renamed", None, None, &[])
    })
    .unwrap();
    let read = |rel: &str| fs::read_to_string(root.join(rel)).unwrap();
    assert_eq!(read("pages/Prose.md"), "[[Renamed]] in prose\n");
    assert_eq!(
        read("pages/Mixed.md"),
        "[[Renamed]] in prose\n\n- [[Renamed]] in a block\n"
    );
    assert_eq!(
        read("pages/Held.md"),
        "[[Renamed]] held\n\n- [[Renamed]] block\n"
    );
    assert_eq!(
        read("pages/Props.md"),
        "tags:: [[Renamed]]\n\nsee #Renamed and [[Renamed]]\n\n```\n[[Target]]\n```\n\n- body\n"
    );
    assert_eq!(
        read("pages/OrgProse.org"),
        "#+title: OrgProse\n\nsee [[Renamed]] here\n* heading\n"
    );
    let _ = fs::remove_dir_all(&root);
}
