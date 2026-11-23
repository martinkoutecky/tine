//! A2 (`RUNTIME-ID-FINDINGS.md`, R5): the browser's `submit_page` is an
//! untrusted entrance. A key submitted on two blocks, or one another page
//! holds, is kept by one block only: the page and the graph never hold one
//! key twice. Fails on 16cee4693 (both blocks published with it).
use super::memory_replace_tests::{open, Files};
use crate::PageId;
use std::collections::BTreeSet;
use tine_core::model::BlockDto;

fn ids(blocks: &[BlockDto], out: &mut Vec<String>) {
    for block in blocks {
        out.push(block.id.clone());
        ids(&block.children, out);
    }
}

/// Finding 4 of the ee4ea5b9a review: A is deleted, B is submitted with a
/// block under A's old first key (no page owns it: accepted), and A is
/// created again with its old bytes. The accepted key is registered to B,
/// so A's positional walk does not hand it out again. A key that another
/// page carries only as an authored `id::` is no runtime ownership: B's
/// block keeps it.
#[test]
fn r2_an_accepted_key_stays_with_the_page_that_took_it() {
    for (a, b, c, text, authored) in [
        (
            "pages/A.md",
            "pages/B.md",
            "pages/C.md",
            "- a\n- a2\n",
            "- c\n  id:: ",
        ),
        (
            "pages/A.org",
            "pages/B.org",
            "pages/C.org",
            "* a\n* a2\n",
            "* c\n  :PROPERTIES:\n  :id: ",
        ),
    ] {
        let uuid = "6f0c1a2b-0000-4000-8000-0000000000c1";
        let tail = if a.ends_with(".org") {
            "\n  :END:\n"
        } else {
            "\n"
        };
        let mut files = Files::new();
        files.insert("logseq/config.edn".into(), b"{}\n".to_vec());
        files.insert(a.into(), text.as_bytes().to_vec());
        files.insert(b.into(), text.as_bytes().to_vec());
        files.insert(c.into(), format!("{authored}{uuid}{tail}").into_bytes());
        let mut view = open(&files);
        let (a_id, b_id) = (PageId::from(a), PageId::from(b));
        let old = view.page(&a_id).unwrap().blocks[0].id.clone();
        view.replace_files(vec![(a.into(), None)]).unwrap();
        let mut dto = view.page(&b_id).unwrap();
        dto.blocks[0].id = old.clone();
        dto.blocks[1].id = uuid.into();
        view.submit_page(&b_id, &dto, true).unwrap();
        view.replace_files(vec![(a.into(), Some(text.as_bytes().to_vec()))])
            .unwrap();
        let mut on_a = Vec::new();
        ids(&view.page(&a_id).unwrap().blocks, &mut on_a);
        assert!(!on_a.contains(&old), "{a}: took B's key back: {on_a:?}");
        let mut on_b = Vec::new();
        ids(&view.page(&b_id).unwrap().blocks, &mut on_b);
        assert_eq!(on_b, [old, uuid.to_owned()], "{b}: keeps its keys");
    }
}

/// Blocker 1 of the cee055d33 review: page Q carries Z's runtime key K as
/// an authored `id::`. Submitting K on B, or on Q itself, is refused (K
/// becomes a hole), whichever of Q and Z comes first, Markdown and Org; so
/// once Q's `id::` goes, `((K))` still reads Z's block. Fails on cee055d33
/// when Q sorts first (intake looked at the first indexed page only), and on
/// Q itself in either order.
#[test]
fn r3_an_authored_id_hides_no_runtime_holder() {
    for (ext, authored) in [
        ("md", "- q\n  id:: {K}\n"),
        ("org", "* q\n  :PROPERTIES:\n  :id: {K}\n  :END:\n"),
    ] {
        for (q, z) in [("A", "Z"), ("Z", "A")] {
            let at = |name: &str| format!("pages/{name}.{ext}");
            let (q, z, b) = (at(q), at(z), at("B"));
            let item = if ext == "md" { "- " } else { "* " };
            let mut files = Files::new();
            files.insert("logseq/config.edn".into(), b"{}\n".to_vec());
            files.insert(z.clone(), format!("{item}z\n").into_bytes());
            files.insert(b.clone(), format!("{item}b\n").into_bytes());
            let (q_id, z_id, b_id) = (
                PageId::from(q.as_str()),
                PageId::from(z.as_str()),
                PageId::from(b.as_str()),
            );
            // K is Z's positional key; Q is in the first build, so the
            // index lists the two pages in name order.
            let key = open(&files).page(&z_id).unwrap().blocks[0].id.clone();
            files.insert(q.clone(), authored.replace("{K}", &key).into_bytes());
            for on in [&b_id, &q_id] {
                let mut view = open(&files);
                let mut dto = view.page(on).unwrap();
                dto.blocks[0].id = key.clone();
                view.submit_page(on, &dto, true).unwrap();
                let mut now = Vec::new();
                ids(&view.page(on).unwrap().blocks, &mut now);
                assert!(
                    !now.contains(&key),
                    "{on:?} ({q} first: {}): took Z's key",
                    q < z
                );
                let mut on_z = Vec::new();
                ids(&view.page(&z_id).unwrap().blocks, &mut on_z);
                assert_eq!(on_z, [key.clone()], "{z}: keeps its key");
                let plain = format!("{item}q\n").into_bytes();
                view.replace_files(vec![(q.clone(), Some(plain))]).unwrap();
                let preview = view.preview_block(&key, 1).unwrap().expect("K resolves");
                assert_eq!(
                    preview.group.blocks[0].raw.trim(),
                    "z",
                    "{on:?} ({q} first: {}): ((K)) reads Z's block",
                    q < z
                );
            }
        }
    }
}

#[test]
fn a2_a_submitted_key_names_one_block() {
    for (rel, other, text) in [
        ("pages/Note.md", "pages/Other.md", "- one\n- two\n- three\n"),
        (
            "pages/Note.org",
            "pages/Other.org",
            "* one\n* two\n* three\n",
        ),
    ] {
        let mut files = Files::new();
        files.insert("logseq/config.edn".into(), b"{}\n".to_vec());
        files.insert(rel.into(), text.as_bytes().to_vec());
        files.insert(other.into(), text.as_bytes().to_vec());
        let mut view = open(&files);
        let (id, other_id) = (PageId::from(rel), PageId::from(other));
        let theirs = view.page(&other_id).unwrap().blocks[0].id.clone();
        let mut dto = view.page(&id).unwrap();
        // The second block repeats the first's key; the third names a
        // block of the other page.
        dto.blocks[1].id = dto.blocks[0].id.clone();
        dto.blocks[2].id = theirs.clone();
        view.submit_page(&id, &dto, true).unwrap();
        let mut now = Vec::new();
        ids(&view.page(&id).unwrap().blocks, &mut now);
        let distinct: BTreeSet<_> = now.iter().collect();
        assert_eq!(distinct.len(), now.len(), "{rel}: one key twice: {now:?}");
        assert_eq!(now[0], dto.blocks[0].id, "{rel}: the first holder keeps it");
        let mut elsewhere = Vec::new();
        ids(&view.page(&other_id).unwrap().blocks, &mut elsewhere);
        assert!(
            elsewhere.contains(&theirs),
            "{rel}: the other page keeps its key"
        );
        assert!(!now.contains(&theirs), "{rel}: two pages hold {theirs}");
    }
}
