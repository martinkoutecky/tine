//! CM step 2, merge-writer slice: the merged file is what the confirmed model's
//! merge rule says (`tine-agents/specs/notes/cm-cut-model-r6.md`, O6, with P2 to
//! P4). Every case runs the user operation (the file merge, which shares the
//! writer with the rename-selected merge) on a fresh graph, then checks what the
//! user gets: the survivor's bytes or parsed content, or the refusal with both
//! files unchanged. Layout the model does not promise (blank lines, bullets,
//! indentation, endings) is asserted as the writer's tested convention.
use std::{fs, sync::Arc};
use tine_core::model::PageDto;
use tine_graph_features::pages;
use tine_store::Store;

/// Merge `src` into `dst` and return the survivor's bytes and page. A merge
/// that is done must also be stable (P3): two unchanged saves keep the bytes
/// and the survivor's name, and a reopen reads the same page. A refusal
/// returns its reason and proves both files are unchanged.
fn merged(src: &str, dst: &str, org: bool) -> Result<(String, PageDto), String> {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    let ext = if org { "org" } else { "md" };
    let source = format!("pages/src.{ext}");
    let target = format!("pages/dst.{ext}");
    fs::write(dir.path().join(&source), src).unwrap();
    fs::write(dir.path().join(&target), dst).unwrap();
    let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    let host = crate::test_host(&store);
    if let Err(error) = pages::merge_pages(&store, &host, &source, &target) {
        assert_eq!(fs::read_to_string(dir.path().join(&source)).unwrap(), src);
        assert_eq!(fs::read_to_string(dir.path().join(&target)).unwrap(), dst);
        return Err(error.to_string());
    }
    assert!(!dir.path().join(&source).exists());
    let bytes = fs::read_to_string(dir.path().join(&target)).unwrap();
    let id = tine_core::model::PageId::from(target.as_str());
    for _ in 0..2 {
        let loaded = store.page(&id).unwrap();
        assert_eq!(loaded.doc.name, "dst", "P3: the survivor keeps its name");
        crate::test_host_writer::save_on(
            &host,
            &store,
            tine_store::EditKind::SaveBlock,
            &id,
            tine_store::SaveBase::Existing(loaded.rev),
            &loaded.doc,
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join(&target)).unwrap(),
            bytes,
            "P3: an unchanged save keeps the merged bytes"
        );
    }
    let doc = store.page(&id).unwrap().doc;
    drop(host);
    drop(store);
    let reopened = Store::open(dir.path(), Default::default()).unwrap().0;
    assert_eq!(reopened.page(&id).unwrap().doc.name, "dst");
    Ok((bytes, doc))
}

/// The rename-selected merge: rename `old` to `new`, confirming the merge
/// into the existing `new` page; `files` are (stem, text) pairs written under
/// `pages/`. The model maps the name (O1/O7) in both files before O6 merges
/// them. Returns the survivor's bytes, or the refusal with every file
/// unchanged.
fn rename_merged(
    files: &[(&str, &str)],
    old: &str,
    new: &str,
    org: bool,
) -> Result<String, String> {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::create_dir(dir.path().join("logseq")).unwrap();
    fs::write(
        dir.path().join("logseq/config.edn"),
        "{:file/name-format :triple-lowbar}\n",
    )
    .unwrap();
    let ext = if org { "org" } else { "md" };
    let path = |stem: &str| format!("pages/{}.{ext}", stem.replace('/', "___"));
    for (stem, text) in files {
        fs::write(dir.path().join(path(stem)), text).unwrap();
    }
    let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    let host = crate::test_host(&store);
    let survivor = path(new);
    let done = pages::rename_or_merge_page(&store, &host, old, new, None, Some(&survivor), &[]);
    if let Err(error) = done {
        for (stem, text) in files {
            assert_eq!(
                fs::read_to_string(dir.path().join(path(stem))).unwrap(),
                *text
            );
        }
        return Err(error.to_string());
    }
    assert!(!dir.path().join(path(old)).exists());
    let bytes = fs::read_to_string(dir.path().join(&survivor)).unwrap();
    let id = tine_core::model::PageId::from(survivor.as_str());
    assert!(
        refs_same(&store.page(&id).unwrap().doc.name, new),
        "P3: the survivor keeps its name"
    );
    Ok(bytes)
}

fn refs_same(a: &str, b: &str) -> bool {
    tine_core::refs::page_key(a) == tine_core::refs::page_key(b)
}

/// The survivor's alias members as Tine reads them from its header.
fn aliases(doc: &PageDto, org: bool) -> Vec<String> {
    tine_core::page_header::page_header_text(
        doc.pre_block.as_deref(),
        doc.blocks.first().map(|b| b.raw.as_str()),
    )
    .into_iter()
    .flat_map(|header| tine_core::page_header::page_property_lines(header, org))
    .filter(|(k, _)| tine_core::page_header::is_alias_key(k))
    .flat_map(|(_, value)| tine_core::property_refs::linkable_names(&value, org))
    .collect()
}

fn bytes(result: Result<(String, PageDto), String>, what: &str) -> String {
    result.unwrap_or_else(|e| panic!("{what}: refused: {e}")).0
}

/// O6: "alias declarations that have members, of both files, become one
/// declaration holding every member once, survivor's first; a member that
/// reads as the same name as one already kept is dropped." Reading is the
/// page-name identity (case and NFC), and a day reads as its day.
#[test]
fn o6_alias_members_unite_by_how_a_name_reads() {
    let _guard = crate::integration_lock();
    for (org, key, b) in [(false, "alias:: ", "-"), (true, "#+alias: ", "*")] {
        // Case only: one name.
        let out = bytes(
            merged(
                &format!("{key}ml\n{b} source\n"),
                &format!("{key}ML\n{b} target\n"),
                org,
            ),
            "case",
        );
        assert_eq!(out, format!("{key}ML\n{b} target\n{b} source\n"));
        // Two spellings of one day read as that day.
        let out = bytes(
            merged(
                &format!("{key}2026-11-02\n{b} source\n"),
                &format!("{key}[[Nov 2nd, 2026]]\n{b} target\n"),
                org,
            ),
            "day",
        );
        assert_eq!(
            out,
            format!("{key}[[Nov 2nd, 2026]]\n{b} target\n{b} source\n")
        );
        // A tag-shaped and a linked spelling of a kept member are dropped whole.
        let out = bytes(
            merged(
                &format!("{key}#a, [[A]], b\n{b} source\n"),
                &format!("{key}a\n{b} target\n"),
                org,
            ),
            "wrappers",
        );
        assert_eq!(out, format!("{key}a, b\n{b} target\n{b} source\n"));
    }
    // `C\#` reads as `C#` in Markdown: one member, the survivor's spelling,
    // across files and inside one declaration (witness `alias_spellings`).
    for (src, dst, want) in [
        (
            "alias:: C#\n- source\n",
            "alias:: C\\#\n- target\n",
            "alias:: C\\#\n- target\n- source\n",
        ),
        (
            "- source\n",
            "alias:: C\\#, C#\n- target\n",
            "alias:: C\\#\n- target\n- source\n",
        ),
    ] {
        let (out, doc) = merged(src, dst, false).unwrap();
        assert_eq!(out, want);
        assert_eq!(aliases(&doc, false), ["C#"]);
    }
}

/// O6 + P4: "the only things dropped are an alias member that reads as the
/// same name as one kept, and the syntax of alias declarations that were
/// combined." A combined declaration leaves no key, delimiter or spacing
/// behind, whatever its key spelling.
#[test]
fn o6_combined_alias_declarations_leave_no_syntax_behind() {
    let _guard = crate::integration_lock();
    for (src, dst, org, want) in [
        (
            "#+ALIAS: Former\n* source\n",
            "#+ALIAS: Kept\n* target\n",
            true,
            "#+ALIAS: Kept, Former\n* target\n* source\n",
        ),
        (
            "alias::  a\n- source\n",
            "alias:: kept\n- target\n",
            false,
            "alias:: kept, a\n- target\n- source\n",
        ),
        (
            "Alias:: Former\n- source\n",
            "alias:: Kept\n- target\n",
            false,
            "alias:: Kept, Former\n- target\n- source\n",
        ),
    ] {
        assert_eq!(
            bytes(merged(src, dst, org), src),
            want,
            "{src:?} into {dst:?}"
        );
    }
}

/// O6: "The survivor keeps its name, path, header and blocks." A memberless
/// survivor declaration is the survivor's header text and stays; the united
/// declaration is the only one with members.
#[test]
fn o6_survivor_keeps_its_memberless_alias_declaration() {
    let _guard = crate::integration_lock();
    for (org, key, b) in [(false, "alias::", "-"), (true, "#+alias:", "*")] {
        let (out, doc) = merged(
            &format!("{b} source\n"),
            &format!("{key}\n{key} a\n{b} target\n"),
            org,
        )
        .unwrap();
        assert_eq!(out, format!("{key}\n{key} a\n{b} target\n{b} source\n"));
        assert_eq!(aliases(&doc, org), ["a"]);
        let (out, doc) = merged(
            &format!("{key} X\n{b} source\n"),
            &format!("{key}\n{b} target\n"),
            org,
        )
        .unwrap();
        assert_eq!(out, format!("{key}\n{key} X\n{b} target\n{b} source\n"));
        assert_eq!(aliases(&doc, org), ["X"]);
    }
}

/// O6 + brief item 2: a childless first-root survivor header carrying
/// `title::` keeps its entries; it cannot also keep the survivor's name and
/// be stable under a save, so the merge is refused, not patched. Its other
/// entries stay and its bullet is layout (P4: bullets are not promised).
#[test]
fn o6_survivor_first_root_header_keeps_its_entries_or_refuses() {
    let _guard = crate::integration_lock();
    let refused = merged("- source\n", "- title:: other\n", false).unwrap_err();
    assert!(refused.contains("SurvivorIdentity"), "{refused}");
    let (out, doc) = merged("- source\n", "- alias:: Kept\n", false).unwrap();
    assert_eq!(out, "alias:: Kept\n- source\n");
    assert_eq!(aliases(&doc, false), ["Kept"]);
}

/// O6: "If there are leftover lines, one leftover block follows ... A blank
/// line and a bare bullet are not leftover lines." Blank header lines, a
/// structural prefix left by a combined alias line and the frame of a fully
/// joined Org drawer make no leftover block.
#[test]
fn o6_no_leftover_block_without_leftover_lines() {
    let _guard = crate::integration_lock();
    for (src, dst, org, want) in [
        (
            "alias:: Former, Shared\n\n- moved\n",
            "alias:: Shared, Kept\n\n- kept\n",
            false,
            "alias:: Shared, Kept, Former\n\n- kept\n- moved\n",
        ),
        (
            "\n\n- source\n",
            "- target\n",
            false,
            "- target\n- source\n",
        ),
        (
            "- alias:: Former\n- source\n",
            "- target\n",
            false,
            "alias:: Former\n- target\n- source\n",
        ),
        (
            ":PROPERTIES:\n:TYPE: x\n:END:\n* source\n",
            "* target\n",
            true,
            ":PROPERTIES:\n:TYPE: x\n:END:\n* target\n* source\n",
        ),
        (
            ":PROPERTIES:\n:END:\n* source\n",
            "* target\n",
            true,
            "* target\n* source\n",
        ),
        (
            "#+alias: Former\n\n* source\n",
            "#+alias: Kept\n* target\n",
            true,
            "#+alias: Kept, Former\n* target\n* source\n",
        ),
    ] {
        assert_eq!(
            bytes(merged(src, dst, org), src),
            want,
            "{src:?} into {dst:?}"
        );
    }
}

/// O6: the leftover's content is fixed: "an empty first line (in Org, a
/// headline with an empty title), then in Markdown the leftover lines, in Org
/// a line `#+BEGIN_QUOTE`, the leftover lines, and a line `#+END_QUOTE`. A
/// leftover line is the entry's own line without its structural prefix,
/// whatever its native form (a drawer entry too)."
#[test]
fn o6_leftover_block_has_the_fixed_content() {
    let _guard = crate::integration_lock();
    for (src, dst, org, want) in [
        (
            "title:: src\n- source\n",
            "- target\n",
            false,
            "- target\n-\n  title:: src\n- source\n",
        ),
        (
            // A first-root source header: its bullet is not a leftover line.
            "- title:: src\n  tags:: t\n- source\n",
            "tags:: u\n- target\n",
            false,
            "tags:: u\n- target\n-\n  title:: src\n  tags:: t\n- source\n",
        ),
        (
            "#+title: src\n* source\n",
            "* target\n",
            true,
            "* target\n*\n#+BEGIN_QUOTE\n#+title: src\n#+END_QUOTE\n* source\n",
        ),
        (
            // A clashing drawer entry is a leftover line inside the quote; the
            // drawer's frame and its joined entry are not.
            ":PROPERTIES:\n:TITLE: src\n:TAGS: t\n:END:\n* source\n",
            ":PROPERTIES:\n:TYPE: y\n:END:\n* target\n",
            true,
            ":PROPERTIES:\n:TYPE: y\n:TAGS: t\n:END:\n* target\n*\n#+BEGIN_QUOTE\n:TITLE: src\n#+END_QUOTE\n* source\n",
        ),
    ] {
        let (out, doc) = merged(src, dst, org).unwrap_or_else(|e| panic!("{src:?}: {e}"));
        assert_eq!(out, want, "{src:?} into {dst:?}");
        let leftover = doc
            .blocks
            .iter()
            .find(|b| b.raw.contains("src"))
            .expect("a leftover block");
        assert!(
            leftover.raw.starts_with('\n') || leftover.raw.is_empty() || org,
            "an empty first line: {:?}",
            leftover.raw
        );
    }
}

/// O6: "Blocks that hung under the source's header hang under the leftover
/// block; if there are such blocks and no leftover lines, the merge is
/// refused" (witness `header_children_no_leftover`).
#[test]
fn o6_header_children_need_leftover_lines() {
    let _guard = crate::integration_lock();
    let refused = merged("- alias:: X\n  - kid\n- source\n", "- target\n", false).unwrap_err();
    assert!(refused.contains("HeaderChildren"), "{refused}");
    let (out, doc) = merged("- title:: A\n  - kid\n- source\n", "- target\n", false).unwrap();
    assert_eq!(out, "- target\n-\n  title:: A\n  - kid\n- source\n");
    assert_eq!(doc.blocks.len(), 3);
    assert_eq!(doc.blocks[1].children.len(), 1);
    assert_eq!(doc.blocks[1].children[0].raw, "kid");
}

/// O6: "a property whose key neither the survivor nor an earlier joined entry
/// has joins ... Keys are compared as the parser reads them" (the reader's
/// one property-key identity, `property_key_norm`: `foo_bar` and `foo-bar` are
/// one key). A repeated source key is a leftover line (witness `repeated_key`).
#[test]
fn o6_joined_property_keys_compare_as_the_parser_reads_them() {
    let _guard = crate::integration_lock();
    for (src, dst, org, want) in [
        (
            "foo_bar:: x\n- source\n",
            "foo-bar:: y\n- target\n",
            false,
            "foo-bar:: y\n- target\n-\n  foo_bar:: x\n- source\n",
        ),
        (
            "#+foo_bar: x\n* source\n",
            "#+foo-bar: y\n* target\n",
            true,
            "#+foo-bar: y\n* target\n*\n#+BEGIN_QUOTE\n#+foo_bar: x\n#+END_QUOTE\n* source\n",
        ),
        (
            "x:: one\nx:: two\n- source\n",
            "- target\n",
            false,
            "x:: one\n- target\n-\n  x:: two\n- source\n",
        ),
    ] {
        assert_eq!(
            bytes(merged(src, dst, org), src),
            want,
            "{src:?} into {dst:?}"
        );
    }
}

/// O6: "In the leftover the lines are no longer page properties; the
/// references inside them stay references, and a merge whose leftover could
/// not keep one (in Org, a property line inside the quote) is refused"
/// (witnesses `leftover_reference_md`, `org_leftover_reference`). The model
/// decides this with its observations, which are Tine's reader (manager
/// ruling 2026-11-16; the witness texts came from a toy reader): the merge is
/// refused exactly when Tine does not read, in the leftover, the references
/// the lines had where they came from. Tine reads an Org `#+key:` line in the
/// quote as before, so it merges; a drawer entry loses its drawer there.
#[test]
fn o6_leftover_keeps_its_references_or_refuses() {
    let _guard = crate::integration_lock();
    assert_eq!(
        bytes(
            merged("key:: [[X]]\n- source\n", "key:: target\n- target\n", false),
            "md reference"
        ),
        "key:: target\n- target\n-\n  key:: [[X]]\n- source\n"
    );
    assert_eq!(
        bytes(
            merged(
                "#+key: [[X]]\n* source\n",
                "#+key: target\n* target\n",
                true
            ),
            "org reference"
        ),
        "#+key: target\n* target\n*\n#+BEGIN_QUOTE\n#+key: [[X]]\n#+END_QUOTE\n* source\n"
    );
    let refused = merged(
        ":PROPERTIES:\n:K: source\n:END:\n* source\n",
        ":PROPERTIES:\n:K: target\n:END:\n* target\n",
        true,
    )
    .unwrap_err();
    assert!(
        refused.contains("HiddenOccurrence"),
        "drawer entry: {refused}"
    );
}

/// The remaining merge witnesses of `witness-fixtures-r6.json`, layout aside:
/// a join placed with the survivor's header (`joined_before_survivor`), a
/// title twin (`file_twins`), an Org title leftover (`org_title`), and a
/// header permutation keeping every entry (`header_permutation`). Each
/// expected file is the writer's tested layout for the witness's content.
#[test]
fn o6_merge_witness_rows() {
    let _guard = crate::integration_lock();
    for (src, dst, org, want) in [
        (
            "clash:: source\njoin:: value\n- s\n",
            "clash:: target\n- t\n",
            false,
            "clash:: target\njoin:: value\n- t\n-\n  clash:: source\n- s\n",
        ),
        (
            "join:: value\n- s\n",
            "clash:: target\n- t\n",
            false,
            "clash:: target\njoin:: value\n- t\n- s\n",
        ),
        (
            "title:: dst\n- source\n",
            "title:: dst\n- target\n",
            false,
            "title:: dst\n- target\n-\n  title:: dst\n- source\n",
        ),
        (
            "#+title: src\n* source\n",
            "#+title: dst\n* target\n",
            true,
            "#+title: dst\n* target\n*\n#+BEGIN_QUOTE\n#+title: src\n#+END_QUOTE\n* source\n",
        ),
        (
            "j:: J\nk:: K\n- source\n",
            "x:: X\ny:: Y\n- target\n",
            false,
            "x:: X\ny:: Y\nj:: J\nk:: K\n- target\n- source\n",
        ),
        // `bare_bullet_header`, read by Tine (manager ruling 2026-11-16): the
        // bare first bullet is an ordinary empty block, kept once (P4).
        (
            "- \n- source\n",
            "- target\n",
            false,
            "- target\n- \n- source\n",
        ),
        // `header_children_leftover`, read by Tine: with a pre-block the first
        // bullet is an ordinary block (its `alias::` a block property) and
        // moves with its child; only the title is a leftover line.
        (
            "title:: src\n- alias:: X\n  - kid\n- source\n",
            "- target\n",
            false,
            "- target\n-\n  title:: src\n- alias:: X\n  - kid\n- source\n",
        ),
    ] {
        assert_eq!(
            bytes(merged(src, dst, org), src),
            want,
            "{src:?} into {dst:?}"
        );
    }
}

/// O6 (r5): "Text before the first block of a file (after its header lines)
/// is read as that file's first block, at the top level, so a merge keeps it
/// like any other block, in its place before that file's other blocks."
/// Checked against Tine's reader on both inputs: each file's pre-block prose
/// (the reader's `preamble_block`) is the content of the corresponding block.
#[test]
fn o6_text_before_the_first_block_is_that_files_first_block() {
    let _guard = crate::integration_lock();
    for (org, src, dst) in [
        (
            false,
            "source prose\n\n- source\n",
            "alias:: Kept\ntarget prose\n- target\n",
        ),
        (
            true,
            "#+title: src\nsource prose\n* source\n",
            "#+alias: Kept\ntarget prose\n* target\n",
        ),
    ] {
        // The reader's own pre-block prose for one input file.
        let prose = |text: &str| {
            let pre: String = text
                .split_inclusive('\n')
                .take_while(|l| !l.starts_with(if org { "* " } else { "- " }))
                .collect();
            let range = tine_core::page_properties::preamble_content_range(&pre, org)
                .expect("the input has pre-block prose");
            pre[range].trim_end().to_owned()
        };
        let (_, doc) = merged(src, dst, org).unwrap();
        let rows: Vec<_> = doc
            .blocks
            .iter()
            .map(|b| b.raw.trim_end().to_owned())
            .collect();
        let at = |s: &str| {
            rows.iter()
                .position(|r| r == s)
                .unwrap_or_else(|| panic!("{s:?} in {rows:?}"))
        };
        let (tp, sp) = (prose(dst), prose(src));
        assert!(at(&tp) < at("target"), "{rows:?}");
        assert!(at("target") < at(&sp) && at(&sp) < at("source"), "{rows:?}");
    }
}

/// O6 compares keys "after the O1/O7 transformations" (`CutObserve.lean`
/// `mapHeader`, `expectedContent`): in a rename-selected merge a property key
/// that names the renamed page is the destination's key before allocation,
/// in both files, as the parser reads the mapped key.
#[test]
fn o6_rename_merge_allocates_by_the_mapped_keys() {
    let _guard = crate::integration_lock();
    for (org, files, want) in [
        // The reviewer's input: the source key becomes the survivor's.
        (
            false,
            [
                ("Old", "old:: x\n- source\n"),
                ("New", "new:: y\n- target\n"),
            ],
            "new:: y\n- target\n-\n  new:: x\n- source\n",
        ),
        (
            true,
            [
                ("Old", "#+old: x\n* source\n"),
                ("New", "#+new: y\n* target\n"),
            ],
            "#+new: y\n* target\n*\n#+BEGIN_QUOTE\n#+new: x\n#+END_QUOTE\n* source\n",
        ),
        // The survivor's own key is a renamed site.
        (
            false,
            [
                ("Old", "new:: x\n- source\n"),
                ("New", "old:: y\n- target\n"),
            ],
            "new:: y\n- target\n-\n  new:: x\n- source\n",
        ),
        // Two source keys that coincide only after the map: the first joins.
        (
            false,
            [
                ("Old", "old:: a\nnew:: b\n- source\n"),
                ("New", "k:: v\n- target\n"),
            ],
            "k:: v\nnew:: a\n- target\n-\n  new:: b\n- source\n",
        ),
    ] {
        assert_eq!(
            rename_merged(&files, "Old", "New", org).unwrap(),
            want,
            "{files:?}"
        );
    }
}

/// A namespace key: O1 maps `old/k` to `new/k` (`siteResult` maps every site
/// kind through `nameMap`), so it clashes with the survivor's `new/k`. Red:
/// the reference rewriter maps a property key only by its exact name
/// (`refs_rename.rs` `Renames::keys`), not by namespace prefix, in every
/// rename. That file is outside this slice; the writer allocates by the key
/// the rewriter writes.
#[test]
#[ignore = "O1 gap in the reference rewriter (namespace property keys), outside the merge writer"]
fn o6_rename_merge_maps_a_namespace_key() {
    let _guard = crate::integration_lock();
    let files = [
        ("Old", "old/k:: x\n- source\n"),
        ("New", "new/k:: y\n- target\n"),
    ];
    assert_eq!(
        rename_merged(&files, "Old", "New", false).unwrap(),
        "new/k:: y\n- target\n-\n  new/k:: x\n- source\n"
    );
}

/// O1 includes the file's title, and O6 demotes the transformed title
/// (`mapAtom`/`titleParts`, `leftoverPayload`): the source's title, now a
/// leftover line, names the destination. A title that only starts with the
/// old name keeps its namespace tail; one that does not name a renamed page
/// is unchanged.
#[test]
fn o6_rename_merge_demotes_the_mapped_title() {
    let _guard = crate::integration_lock();
    for (org, files, old, new, want) in [
        (
            false,
            vec![("Old", "title:: Old\n- source\n"), ("New", "- target\n")],
            "Old",
            "New",
            "- target\n-\n  title:: New\n- source\n",
        ),
        (
            true,
            vec![("Old", "#+TITLE: Old\n* source\n"), ("New", "* target\n")],
            "Old",
            "New",
            "* target\n*\n#+BEGIN_QUOTE\n#+TITLE: New\n#+END_QUOTE\n* source\n",
        ),
        // Case only: the title reads as the page, so it names the destination.
        (
            false,
            vec![("Old", "title:: old\n- source\n"), ("New", "- target\n")],
            "Old",
            "New",
            "- target\n-\n  title:: New\n- source\n",
        ),
        // A namespace source moved by its parent's rename: only the old
        // prefix is replaced (`titlePrefix`), the tail is kept.
        (
            false,
            vec![
                ("Old", "- parent\n"),
                ("Old/child", "title:: Old/child\n- source\n"),
                ("New/child", "- target\n"),
            ],
            "Old/child",
            "New/child",
            "- target\n-\n  title:: New/child\n- source\n",
        ),
    ] {
        assert_eq!(
            rename_merged(&files, old, new, org).unwrap(),
            want,
            "{files:?}"
        );
    }
}

/// O6's alias union in a rename-selected merge reads the members after the
/// map: a member naming the old page names the destination, so it reads as
/// a member equal to the survivor's name, and one spelled as that name is the
/// same member.
#[test]
fn o6_rename_merge_unites_the_mapped_alias_members() {
    let _guard = crate::integration_lock();
    for (org, files, want) in [
        (
            false,
            [
                ("Old", "alias:: New, X\n- source\n"),
                ("New", "alias:: Old\n- target\n"),
            ],
            "alias:: New, X\n- target\n- source\n",
        ),
        (
            false,
            [("Old", "alias:: Old\n- source\n"), ("New", "- target\n")],
            "alias:: New\n- target\n- source\n",
        ),
        (
            true,
            [
                ("Old", "#+alias: New, X\n* source\n"),
                ("New", "#+alias: Old\n* target\n"),
            ],
            "#+alias: New, X\n* target\n* source\n",
        ),
    ] {
        assert_eq!(
            rename_merged(&files, "Old", "New", org),
            Ok(want.to_owned()),
            "{files:?}"
        );
    }
}

/// P4: only bytes the parser does not report as content are layout. Content
/// that looks like layout (a blank line inside a block, a code block or a
/// quote, a literal drawer frame in a block body, Markdown `*` and `+` lines,
/// Org prose before a later directive) survives the merge byte for byte,
/// while a blank line in the source header is dropped (not a leftover line).
#[test]
fn o6_layout_shaped_content_survives_the_merge() {
    let _guard = crate::integration_lock();
    let md_body = "- a\n\n  b\n\
                   - ```\n  :PROPERTIES:\n\n  -\n  :END:\n  ```\n\
                   - > q\n  >\n  > r\n\
                   - body\n  :PROPERTIES:\n  :END:\n\
                   - c\n  *\n  +\n  * star\n  + plus\n";
    let org_body = "* a\n\n  b\n\
                    * code\n#+BEGIN_SRC\n:PROPERTIES:\n\n:END:\n#+END_SRC\n\
                    * quote\n#+BEGIN_QUOTE\n\n:END:\n#+END_QUOTE\n";
    for (org, src, dst, want) in [
        (
            false,
            format!("k:: s\n\n{md_body}"),
            "k:: t\n- target\n".to_owned(),
            format!("k:: t\n- target\n-\n  k:: s\n{md_body}"),
        ),
        (
            true,
            format!("#+k: s\n\n{org_body}"),
            "#+k: t\n* target\n".to_owned(),
            format!("#+k: t\n* target\n*\n#+BEGIN_QUOTE\n#+k: s\n#+END_QUOTE\n{org_body}"),
        ),
        // Org prose before a later directive is header text: each line is a
        // leftover line, and the blank line between them is not one (O6).
        (
            true,
            "free text\n\n#+k: s\n* source\n".to_owned(),
            "#+k: t\n* target\n".to_owned(),
            "#+k: t\n* target\n*\n#+BEGIN_QUOTE\nfree text\n#+k: s\n#+END_QUOTE\n* source\n"
                .to_owned(),
        ),
    ] {
        assert_eq!(bytes(merged(&src, &dst, org), &src), want, "{src:?}");
    }
}
