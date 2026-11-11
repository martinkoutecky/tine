//! Complete-text merge outcomes, through the production host and disk reader.
use std::{fs, sync::Arc};
use tine_graph_features::pages;
use tine_store::Store;

// Exercise the user operation first, then inspect its provenance and ordinary
// save closure independently against the same original inputs.
fn valid_merge(
    src: &str,
    dst: &str,
    org: bool,
    check: impl Fn(&tine_core::model::PageDto, &pages::merge_payload::MergeProposal),
) -> Result<(), String> {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    let ext = if org { "org" } else { "md" };
    let source = format!("pages/src.{ext}");
    let target = format!("pages/dst.{ext}");
    fs::write(dir.path().join(&source), src).unwrap();
    fs::write(dir.path().join(&target), dst).unwrap();
    let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    let host = crate::test_host(&store);
    let src_id = tine_core::model::PageId::from(source.as_str());
    let dst_id = tine_core::model::PageId::from(target.as_str());
    let before_src = store.page(&src_id).unwrap().doc;
    let before_dst = store.page(&dst_id).unwrap().doc;
    if let Err(error) = pages::merge_pages(&store, &host, &source, &target) {
        assert_eq!(fs::read_to_string(dir.path().join(&source)).unwrap(), src);
        assert_eq!(fs::read_to_string(dir.path().join(&target)).unwrap(), dst);
        return Err(format!(
            "org={org}, source={src:?}, target={dst:?}: {error}"
        ));
    }
    assert!(!dir.path().join(&source).exists());
    let bytes = fs::read_to_string(dir.path().join(&target)).unwrap();
    let proof = pages::merge_payload::propose(
        &before_dst,
        dst,
        &before_src,
        src,
        &Default::default(),
        &Default::default(),
        org,
        store.config().file_name_format,
    )
    .unwrap();
    assert_eq!(bytes, proof.text);
    proof
        .trace
        .check(&bytes, &proof.trace.substitution)
        .unwrap();
    for _ in 0..2 {
        let loaded = store.page(&dst_id).unwrap();
        check(&loaded.doc, &proof);
        crate::test_host_writer::save_on(
            &host,
            &store,
            tine_store::EditKind::SaveBlock,
            &dst_id,
            tine_store::SaveBase::Existing(loaded.rev),
            &loaded.doc,
        )
        .unwrap();
        assert_eq!(fs::read_to_string(dir.path().join(&target)).unwrap(), bytes);
    }
    if let Ok(output) = std::env::var("CM_MERGE_ORACLE") {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        (src, dst, org).hash(&mut hash);
        let doc = store.page(&dst_id).unwrap().doc;
        fs::write(std::path::Path::new(&output).join(format!("valid-{}.json", hash.finish())),
            serde_json::to_vec(&serde_json::json!({"source":src,"target":dst,"text":bytes,"org":org,"document":doc})).unwrap()).unwrap();
    }
    drop(host);
    drop(store);
    let reopened = Store::open(dir.path(), Default::default()).unwrap().0;
    check(&reopened.page(&dst_id).unwrap().doc, &proof);
    Ok(())
}

#[test]
fn valid_merge_deduplicates_retained_alias_members() {
    let _guard = crate::integration_lock();
    let mut failures = Vec::new();
    for org in [false, true] {
        let (key, bullet) = if org {
            ("#+alias: ", "*")
        } else {
            ("alias:: ", "-")
        };
        for (members, expected) in [
            ("a, a", vec!["a"]),
            ("a,  a", vec!["a"]),
            ("[[a]], [[a]]", vec!["a"]),
            ("#a, #a", vec!["a"]),
            ("#[[a]], #[[a]]", vec!["a"]),
            ("[[a]], a, #a, [[b]], [[a]]", vec!["a", "b"]),
            ("[[a]], [[A]], [[a]]", vec!["a", "A"]),
        ] {
            for retained_target in [false, true] {
                for ending in ["\n", "\r\n"] {
                    let alias = format!("{key}{members}\n");
                    let src = format!(
                        "{}{bullet} source\n",
                        if retained_target { "" } else { &alias }
                    )
                    .replace('\n', ending);
                    let dst = format!(
                        "{}{bullet} target\n",
                        if retained_target { &alias } else { "" }
                    )
                    .replace('\n', ending);
                    if let Err(error) = valid_merge(&src, &dst, org, |doc, _| {
                        let header = tine_core::page_header::page_header_text(
                            doc.pre_block.as_deref(),
                            doc.blocks.first().map(|b| b.raw.as_str()),
                        )
                        .unwrap();
                        let names: Vec<_> =
                            tine_core::page_header::page_property_lines(header, org)
                                .into_iter()
                                .filter(|(k, _)| tine_core::page_header::is_alias_key(k))
                                .flat_map(|(_, value)| {
                                    tine_core::property_refs::linkable_names(&value, org)
                                })
                                .collect();
                        assert_eq!(names, expected);
                    }) {
                        failures.push(error);
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} valid merges refused:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn valid_merge_keeps_source_header_children_under_leftover() {
    let _guard = crate::integration_lock();
    let mut failures = Vec::new();
    for org in [false, true] {
        for nested in [false, true] {
            for ending in ["\n", "\r\n"] {
                let (src, dst, children) = if org {
                    // Org page properties live in the preamble; their first
                    // heading and its children are ordinary source body.
                    (
                        format!(
                            "#+alias: Former\n* child\n{}* sibling\n* source\n",
                            if nested { "** deep\n" } else { "" }
                        ),
                        "#+alias: Kept\n* target\n",
                        "",
                    )
                } else {
                    (
                        format!(
                            "- alias:: Former\n  - child\n{}  - sibling\n- source\n",
                            if nested { "    - deep\n" } else { "" }
                        ),
                        "alias:: Kept\n- target\n",
                        "  - child\n",
                    )
                };
                let src = src.replace('\n', ending);
                let dst = dst.replace('\n', ending);
                if let Err(error) = valid_merge(&src, &dst, org, |doc, proof| {
                    use pages::merge_payload::Origin;
                    let trim = |s: &str| s.trim_end_matches('\r').to_owned();
                    assert_eq!(trim(&doc.blocks[0].raw), "target");
                    let at = if org { 1 } else { 2 };
                    let rows = if org {
                        &doc.blocks[at..at + 2]
                    } else {
                        &doc.blocks[1].children[..]
                    };
                    assert_eq!(rows.len(), 2);
                    assert_eq!(trim(&rows[0].raw), "child");
                    assert_eq!(trim(&rows[1].raw), "sibling");
                    assert_eq!(rows[0].children.len(), usize::from(nested));
                    if nested {
                        assert_eq!(trim(&rows[0].children[0].raw), "deep");
                    }
                    assert_eq!(trim(&doc.blocks.last().unwrap().raw), "source");
                    if !org {
                        assert!(proof.text.contains(&children.replace('\n', ending)));
                        let first = proof
                            .origins
                            .iter()
                            .position(|o| o.contains(&Origin::Leftover))
                            .unwrap();
                        for j in 0..2 + usize::from(nested) {
                            assert_eq!(proof.origins[first + 1 + j], vec![Origin::Source(1 + j)]);
                        }
                        assert_eq!(
                            proof.origins.last().unwrap(),
                            &vec![Origin::Source(3 + usize::from(nested))]
                        );
                    }
                }) {
                    failures.push(error);
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} valid merges refused:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn preamble_prose_keeps_its_body_row_through_merge_and_saves() {
    let _guard = crate::integration_lock();
    for (org, ending) in [(false, "\n"), (false, "\r\n"), (true, "\n"), (true, "\r\n")] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        let ext = if org { "org" } else { "md" };
        let source = format!("pages/src.{ext}");
        let target = format!("pages/dst.{ext}");
        let (source_text, target_text) = if org {
            (
                "#+title: src\n{{query [[A]]}}\n* source\n",
                "#+alias: Kept\ntarget prose\n* target\n",
            )
        } else {
            (
                "title:: src\n{{query [[A]]}}\n- source\n",
                "alias:: Kept\ntarget prose\n- target\n",
            )
        };
        fs::write(dir.path().join(&source), source_text.replace('\n', ending)).unwrap();
        fs::write(dir.path().join(&target), target_text.replace('\n', ending)).unwrap();
        let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
        let host = crate::test_host(&store);
        pages::merge_pages(&store, &host, &source, &target).unwrap();
        let id = tine_core::model::PageId::from(target.as_str());
        let bytes = fs::read_to_string(dir.path().join(&target)).unwrap();
        let raw = |s: &str| format!("{s}{}", if org && ending == "\r\n" { "\r" } else { "" });
        for _ in 0..2 {
            let loaded = store.page(&id).unwrap();
            assert_eq!(loaded.doc.blocks[0].raw, raw("target prose"));
            assert_eq!(loaded.doc.blocks[1].raw, raw("target"));
            assert_eq!(loaded.doc.blocks[3].raw, raw("{{query [[A]]}}"));
            assert_eq!(loaded.doc.blocks[4].raw, raw("source"));
            assert_eq!(loaded.doc.blocks.len(), 5);
            crate::test_host_writer::save_on(
                &host,
                &store,
                tine_store::EditKind::SaveBlock,
                &id,
                tine_store::SaveBase::Existing(loaded.rev),
                &loaded.doc,
            )
            .unwrap();
            assert_eq!(fs::read_to_string(dir.path().join(&target)).unwrap(), bytes);
        }
        drop(host);
        drop(store);
        let reopened = Store::open(dir.path(), Default::default()).unwrap().0;
        assert_eq!(
            reopened.page(&id).unwrap().doc.blocks[3].raw,
            raw("{{query [[A]]}}")
        );
    }
}

#[test]
fn inert_leftover_survives_two_guarded_saves_and_reopen() {
    let _guard = crate::integration_lock();
    for (org, ending) in [(false, "\n"), (false, "\r\n"), (true, "\n"), (true, "\r\n")] {
        for headerless in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir(dir.path().join("pages")).unwrap();
            let ext = if org { "org" } else { "md" };
            let source = format!("pages/src.{ext}");
            let target = format!("pages/dst.{ext}");
            fs::write(
                dir.path().join(&source),
                (if org {
                    "#+title: src\n"
                } else {
                    "title:: src\n"
                })
                .replace('\n', ending),
            )
            .unwrap();
            fs::write(
                dir.path().join(&target),
                (if headerless {
                    if org {
                        "* kept\n"
                    } else {
                        "- kept\n"
                    }
                } else {
                    ""
                })
                .replace('\n', ending),
            )
            .unwrap();
            let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
            let host = crate::test_host(&store);
            pages::merge_pages(&store, &host, &source, &target).unwrap();
            let id = store
                .as_page(
                    &store
                        .file_id(tine_store::Area::Pages, &format!("dst.{ext}"))
                        .unwrap(),
                )
                .unwrap();
            if headerless {
                let mut loaded = store.page(&id).unwrap();
                loaded.doc.blocks.remove(0);
                crate::test_host_writer::save_on(
                    &host,
                    &store,
                    tine_store::EditKind::DeleteBlocks,
                    &id,
                    tine_store::SaveBase::Existing(loaded.rev),
                    &loaded.doc,
                )
                .unwrap();
            }
            let bytes = fs::read_to_string(dir.path().join(&target)).unwrap();
            for _ in 0..2 {
                let loaded = store.page(&id).unwrap();
                crate::test_host_writer::save_on(
                    &host,
                    &store,
                    tine_store::EditKind::SaveBlock,
                    &id,
                    tine_store::SaveBase::Existing(loaded.rev),
                    &loaded.doc,
                )
                .unwrap();
                assert_eq!(fs::read_to_string(dir.path().join(&target)).unwrap(), bytes);
                assert_eq!(store.page(&id).unwrap().doc.name, "dst");
            }
            drop(host);
            drop(store);
            let reopened = Store::open(dir.path(), Default::default()).unwrap().0;
            assert_eq!(reopened.page(&id).unwrap().doc.name, "dst");
        }
    }
}

#[test]
fn tag_alias_name_cuts_keep_hash_and_existing_wrappers() {
    let _guard = crate::integration_lock();
    for org in [false, true] {
        for token in ["#src", "#[[src]]"] {
            for destination in ["dst", "two words"] {
                for target_alias in [false, true] {
                    let (key, bullet) = if org {
                        ("#+alias: ", "*")
                    } else {
                        ("alias:: ", "-")
                    };
                    let alias = format!("{key}{token}\n");
                    let source = format!(
                        "{}{bullet} source\n",
                        if target_alias { "" } else { &alias }
                    );
                    let target = format!(
                        "{}{bullet} target\n",
                        if target_alias { &alias } else { "" }
                    );
                    let dir = tempfile::tempdir().unwrap();
                    fs::create_dir(dir.path().join("pages")).unwrap();
                    let ext = if org { "org" } else { "md" };
                    let source_path = format!("pages/src.{ext}");
                    let target_path = format!("pages/{destination}.{ext}");
                    fs::write(dir.path().join(&source_path), &source).unwrap();
                    fs::write(dir.path().join(&target_path), &target).unwrap();
                    let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
                    let source_id = tine_core::model::PageId::from(source_path.as_str());
                    let target_id = tine_core::model::PageId::from(target_path.as_str());
                    let map = std::collections::HashMap::from([("src".into(), destination.into())]);
                    let proof = pages::merge_payload::propose(
                        &store.page(&target_id).unwrap().doc,
                        &target,
                        &store.page(&source_id).unwrap().doc,
                        &source,
                        &map,
                        &map,
                        org,
                        tine_core::config::FileNameFormat::Legacy,
                    )
                    .unwrap();
                    let expected = if token.contains("[[") || destination.contains(' ') {
                        format!("#[[{destination}]]")
                    } else {
                        format!("#{destination}")
                    };
                    assert!(proof.text.contains(&expected), "{}", proof.text);
                    assert!(proof
                        .trace
                        .pieces
                        .iter()
                        .enumerate()
                        .any(|(id, p)| !p.editable && proof.trace.original(id).contains('#')));
                    proof
                        .trace
                        .check(&proof.text, &proof.trace.substitution)
                        .unwrap();
                    let host = crate::test_host(&store);
                    pages::rename_page_as(
                        &store,
                        &host,
                        "src",
                        destination,
                        None,
                        pages::RenameChoice::MergeInto(&target_path),
                        &[],
                    )
                    .unwrap();
                    assert_eq!(
                        fs::read_to_string(dir.path().join(&target_path)).unwrap(),
                        proof.text
                    );
                    assert!(!dir.path().join(&source_path).exists());
                    if let Ok(output) = std::env::var("CM_MERGE_ORACLE") {
                        let label = format!(
                            "tag-{org}-{target_alias}-{}-{}.json",
                            token.contains("[["),
                            destination.contains(' ')
                        );
                        fs::write(std::path::Path::new(&output).join(label), serde_json::to_vec(&serde_json::json!({"text": proof.text, "org": org, "name": destination})).unwrap()).unwrap();
                    }
                }
            }
        }
    }
}

#[test]
fn survivor_first_root_headers_keep_native_save_closure() {
    let _guard = crate::integration_lock();
    for target in [
        "- alias:: Kept\n",
        "- title:: other\n",
        "- alias:: Kept\n  - child\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::write(dir.path().join("pages/src.md"), "- source\n").unwrap();
        fs::write(dir.path().join("pages/dst.md"), target).unwrap();
        let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
        let host = crate::test_host(&store);
        pages::merge_pages(&store, &host, "pages/src.md", "pages/dst.md").unwrap();
        let id = tine_core::model::PageId::from("pages/dst.md");
        let bytes = fs::read_to_string(dir.path().join("pages/dst.md")).unwrap();
        for _ in 0..2 {
            let loaded = store.page(&id).unwrap();
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
                fs::read_to_string(dir.path().join("pages/dst.md")).unwrap(),
                bytes
            );
            assert_eq!(store.page(&id).unwrap().doc.name, "dst");
        }
        drop(host);
        drop(store);
        assert_eq!(
            Store::open(dir.path(), Default::default())
                .unwrap()
                .0
                .page(&id)
                .unwrap()
                .doc
                .name,
            "dst"
        );
    }
}

fn merge(
    src: &str,
    dst: &str,
    org: bool,
    check: impl FnOnce(&str, &Store, &pages::merge_payload::MergeProposal),
) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    let ext = if org { "org" } else { "md" };
    let source = format!("pages/src.{ext}");
    let target = format!("pages/dst.{ext}");
    fs::write(dir.path().join(&source), src).unwrap();
    fs::write(dir.path().join(&target), dst).unwrap();
    let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    let host = crate::test_host(&store);
    let src_id = store
        .as_page(
            &store
                .file_id(tine_store::Area::Pages, &format!("src.{ext}"))
                .unwrap(),
        )
        .unwrap();
    let dst_id = store
        .as_page(
            &store
                .file_id(tine_store::Area::Pages, &format!("dst.{ext}"))
                .unwrap(),
        )
        .unwrap();
    let proposal = pages::merge_payload::propose(
        &store.page(&dst_id).unwrap().doc,
        dst,
        &store.page(&src_id).unwrap().doc,
        src,
        &Default::default(),
        &Default::default(),
        org,
        store.config().file_name_format,
    )
    .unwrap();
    proposal
        .trace
        .check(&proposal.text, &proposal.trace.substitution)
        .unwrap();
    pages::merge_pages(&store, &host, &source, &target).unwrap();
    assert!(!dir.path().join(source).exists());
    let text = fs::read_to_string(dir.path().join(target)).unwrap();
    assert_eq!(
        text, proposal.text,
        "the host writes the certified proposal"
    );
    if let Ok(output) = std::env::var("CM_MERGE_ORACLE") {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        (src, dst, org).hash(&mut hash);
        let next_save = if !org && dst.is_empty() && src.starts_with("title::") {
            let loaded = store.page(&dst_id).unwrap();
            let mut tx = store.page_plan(Some(tine_store::EditKind::SaveBlock));
            tx.save_page(
                &[tine_store::EditKind::SaveBlock],
                &dst_id,
                tine_store::SaveBase::Existing(loaded.rev),
                &loaded.doc,
            );
            let operation = tx.plan().unwrap();
            operation
                .pages
                .iter()
                .find_map(|(id, effect, _)| match effect {
                    tine_store::HostEffect::Write(bytes) if *id == dst_id => {
                        Some(String::from_utf8(bytes.to_vec()).unwrap())
                    }
                    _ => None,
                })
        } else {
            None
        };
        fs::write(
            std::path::Path::new(&output).join(format!("merge-{}.json", hash.finish())),
            serde_json::to_vec(
                &serde_json::json!({"source":src,"target":dst,"text":text,"org":org,"nextSave":next_save}),
            )
            .unwrap(),
        )
        .unwrap();
    }
    check(&text, &store, &proposal);
}

#[test]
fn merge_preserves_equal_differing_title_unknown_and_blank_header_lines() {
    let _guard = crate::integration_lock();
    for (src, dst, protected) in [
        (
            "same:: equal\n- source\n",
            "same:: equal\n- target\n",
            "same:: equal",
        ),
        (
            "same:: two\n- source\n",
            "same:: one\n- target\n",
            "same:: two",
        ),
        (
            "title:: src\n- source\n",
            "title:: dst\n- target\n",
            "title:: src",
        ),
        (
            "unknown prose\nmalformed:::\n\n- source\n",
            "- target\n",
            "unknown prose",
        ),
        ("\n\n- source\n", "- target\n", "\n  \n"),
    ] {
        merge(src, dst, false, |text, _, _| {
            assert!(text.starts_with(dst), "survivor bytes: {text:?}");
            let after = &text[dst.len()..];
            assert!(after.contains(protected), "lost {protected:?}: {text:?}");
            assert!(after.ends_with("- source\n"));
        });
    }
}

#[test]
fn merge_keeps_bodies_and_mixed_line_endings_verbatim() {
    let _guard = crate::integration_lock();
    for org in [false, true] {
        for (a, b) in [("\r\n", "\n"), ("\n", "\r\n")] {
            let (src, dst) = if org {
                (
                    format!("#+fresh: yes{a}* source{a}** child{a}"),
                    format!("#+title: dst{b}* target{b}** kept{b}"),
                )
            } else {
                (
                    format!("fresh:: yes{a}- source{a}\t- child{a}"),
                    format!("title:: dst{b}- target{b}    - kept{b}"),
                )
            };
            merge(&src, &dst, org, |text, store, _| {
                for bytes in [&src, &dst] {
                    for line in bytes.split_inclusive('\n') {
                        assert!(text.contains(line), "lost {line:?}: {text:?}");
                    }
                }
                let graph = store.whole_graph().unwrap();
                let tine_store::Resolved::Existing { id, .. } = graph.resolve("dst", false) else {
                    panic!("missing survivor");
                };
                let page = store.page(&id).unwrap();
                assert_eq!(page.doc.blocks.len(), 2);
                assert_eq!(page.doc.blocks[0].children.len(), 1);
                assert_eq!(page.doc.blocks[1].children.len(), 1);
            });
        }
    }
}

#[test]
fn merge_empty_survivor_keeps_its_name_and_org_directives_are_nested() {
    let _guard = crate::integration_lock();
    for (src, dst, org) in [
        ("title:: src\n", "", false),
        (
            "#+TITLE: src\n#+CATEGORY: source\n* body\n",
            "#+TITLE: dst\n* kept\n",
            true,
        ),
    ] {
        merge(src, dst, org, |text, store, _| {
            assert_eq!(
                tine_core::model::page_title_from_preamble(
                    text,
                    if org {
                        tine_core::model::Format::Org
                    } else {
                        tine_core::model::Format::Md
                    }
                )
                .as_deref()
                .unwrap_or("dst"),
                "dst"
            );
            let graph = store.whole_graph().unwrap();
            assert!(matches!(
                graph.resolve("dst", false),
                tine_store::Resolved::Existing { .. }
            ));
            if org {
                assert!(
                    text.contains("#+BEGIN_QUOTE\n#+TITLE: src\n#+END_QUOTE"),
                    "{text:?}"
                );
                assert!(text.contains("#+CATEGORY: source\n"));
            }
        });
    }
}

#[test]
fn unrealizable_body_or_escaping_org_directives_refuse_without_partial_writes() {
    let _guard = crate::integration_lock();
    for (src, dst, org, reason) in [
        (
            "  - indented source\n",
            "- target\n",
            false,
            pages::merge_payload::RealizationError::BodyLaw,
        ),
        (
            "#+END_QUOTE\n#+title: src\n* source\n",
            "#+title: dst\n* target\n",
            true,
            pages::merge_payload::RealizationError::DirectiveEscape,
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        let ext = if org { "org" } else { "md" };
        let a = format!("pages/src.{ext}");
        let b = format!("pages/dst.{ext}");
        fs::write(dir.path().join(&a), src).unwrap();
        fs::write(dir.path().join(&b), dst).unwrap();
        let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
        let host = crate::test_host(&store);
        let error = pages::merge_pages(&store, &host, &a, &b).unwrap_err();
        assert_eq!(
            error
                .get_ref()
                .unwrap()
                .downcast_ref::<pages::merge_payload::RealizationError>(),
            Some(&reason)
        );
        assert_eq!(fs::read_to_string(dir.path().join(a)).unwrap(), src);
        assert_eq!(fs::read_to_string(dir.path().join(b)).unwrap(), dst);
    }
}

#[test]
fn alias_union_uses_exact_spelling_and_protects_non_native_remainders() {
    let _guard = crate::integration_lock();
    for org in [false, true] {
        let (key, bullet) = if org {
            ("#+alias: ", "*")
        } else {
            ("alias:: ", "-")
        };
        for (source, target, active, leftover) in [
            ("b, c", "a, b", "a, b, c", None),
            ("x", "X", "X, x", None),
            ("b, b, c", "a, a, b", "a, b, c", None),
            ("[[a, b]]", "kept", "kept, [[a, b]]", None),
            ("[[a]]", "kept", "kept, [[a]]", None),
            (" a", "kept", "kept, a", Some(" ")),
        ] {
            let src = format!("{key}{source}\n{bullet} source\n");
            let dst = format!("{key}{target}\n{bullet} target\n");
            merge(&src, &dst, org, |text, store, proof| {
                if source == "b, c" && target == "a, b" {
                    let consumed: Vec<_> = proof
                        .trace
                        .consumed
                        .iter()
                        .map(|id| proof.trace.original(*id))
                        .collect();
                    assert_eq!(
                        consumed,
                        vec![key, "b", ", ", "\n"],
                        "only the approved source syntax and duplicate member are consumed"
                    );
                }
                assert!(text.starts_with(&format!("{key}{active}\n")), "{text:?}");
                if let Some(leftover) = leftover {
                    let expected = if org {
                        format!("*\n#+BEGIN_QUOTE\n{leftover}\n#+END_QUOTE\n")
                    } else {
                        format!("- {leftover}\n")
                    };
                    assert!(text.contains(&expected), "{text:?}");
                }
                assert!(
                    proof.trace.output.iter().all(|p| match p {
                        tine_core::text_trace::Placement::Original(_) => true,
                        tine_core::text_trace::Placement::Insert(s) => s
                            .replace("#+BEGIN_QUOTE", "")
                            .replace("#+END_QUOTE", "")
                            .chars()
                            .all(|c| !c.is_alphanumeric()),
                    }),
                    "member names must be moved originals"
                );
                let graph = store.whole_graph().unwrap();
                let tine_store::Resolved::Existing { id, .. } = graph.resolve("dst", false) else {
                    panic!()
                };
                let doc = store.page(&id).unwrap().doc;
                assert_eq!(doc.blocks.len(), 2 + usize::from(leftover.is_some()));
                let names = tine_core::page_header::page_alias_spellings(
                    doc.pre_block.as_deref(),
                    doc.blocks.first().map(|b| b.raw.as_str()),
                    org,
                );
                for name in tine_core::property_refs::linkable_names(source, org) {
                    assert!(
                        names.iter().any(|n| tine_core::refs::same_page(n, &name)),
                        "{name}: {names:?}"
                    );
                }
            });
        }
    }
}

#[test]
fn org_quote_preserves_clashing_property_reference_occurrences() {
    let _guard = crate::integration_lock();
    merge(
        "#+tags: [[x]]\n#+CATEGORY: source\n* source\n",
        "#+tags: [[y]]\n#+CATEGORY: target\n* target\n",
        true,
        |text, store, _| {
            assert!(text.contains("#+BEGIN_QUOTE\n#+tags: [[x]]\n#+CATEGORY: source\n#+END_QUOTE"));
            let graph = store.whole_graph().unwrap();
            let tine_store::Resolved::Existing { id, .. } = graph.resolve("dst", false) else {
                panic!()
            };
            let doc = store.page(&id).unwrap().doc;
            let block = tine_core::page_properties::property_projection(&doc.blocks[1].raw, true);
            for name in ["x", "category"] {
                assert!(
                    block
                        .projection()
                        .reference_source()
                        .explicit
                        .iter()
                        .any(|r| r.name == name),
                    "{name} stays a covered site"
                );
            }
        },
    );
}

#[test]
fn invalid_merge_certificate_refuses_before_the_survivor_or_source_changes() {
    let _guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    let source = "- source\n";
    let target = "- target\n";
    fs::write(dir.path().join("pages/src.md"), source).unwrap();
    fs::write(dir.path().join("pages/dst.md"), target).unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let src = store
        .as_page(&store.file_id(tine_store::Area::Pages, "src.md").unwrap())
        .unwrap();
    let dst = store
        .as_page(&store.file_id(tine_store::Area::Pages, "dst.md").unwrap())
        .unwrap();
    let (_, src_rev) = store.read(&src.file(), None).unwrap();
    let (_, dst_rev) = store.read(&dst.file(), None).unwrap();
    let proof = pages::merge_payload::propose(
        &store.page(&dst).unwrap().doc,
        target,
        &store.page(&src).unwrap().doc,
        source,
        &Default::default(),
        &Default::default(),
        false,
        store.config().file_name_format,
    )
    .unwrap();
    for wrong_old in [false, true] {
        let mut trace = proof.trace.clone();
        if !wrong_old {
            trace.consumed.push(0);
        }
        let mut tx = store.page_plan(Some(tine_store::EditKind::InsertBlocks));
        tx.rewrite_refs_prepared(
            &[
                tine_store::EditKind::InsertBlocks,
                tine_store::EditKind::DeletePage,
            ],
            &dst,
            dst_rev.clone(),
            (
                if wrong_old {
                    "different".into()
                } else {
                    target.into()
                },
                proof.text.clone(),
            ),
            None,
            store.config().file_name_format,
            trace,
        );
        tx.trash(&src.file(), src_rev.clone(), tine_store::TrashIf::Any);
        assert!(matches!(
            tx.plan(),
            Err(tine_store::Why::Refused(
                tine_store::Refusal::TextRealization(_)
            ))
        ));
        assert_eq!(
            fs::read_to_string(dir.path().join("pages/src.md")).unwrap(),
            source
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("pages/dst.md")).unwrap(),
            target
        );
    }
}

#[test]
fn first_root_headers_and_source_heading_style_are_realized_from_original_bytes() {
    let _guard = crate::integration_lock();
    for (source, target) in [
        ("- alias:: Former\n- source\n", "- alias:: Kept\n- target\n"),
        ("alias:: Former\n- source\n", "- alias:: Kept\n- target\n"),
        (
            "alias:: Former\n- source\n",
            "- alias:: Kept\n  - child\n- target\n",
        ),
        ("- alias:: Former\n- source\n", "- target\n"),
    ] {
        merge(source, target, false, |text, store, proof| {
            assert!(text.contains("- source\n"));
            if target.starts_with("- alias") {
                let expected = if target.contains("child") {
                    "- alias:: Kept, Former\n"
                } else {
                    "alias:: Kept, Former\n"
                };
                assert!(text.starts_with(expected), "{text:?}");
            }
            let graph = store.whole_graph().unwrap();
            for alias in ["Former", "Kept"]
                .into_iter()
                .filter(|a| *a != "Kept" || target.contains("Kept"))
            {
                assert!(matches!(
                    graph.resolve(alias, false),
                    tine_store::Resolved::Alias { .. }
                ));
            }
            assert!(proof.origins.iter().all(|o| !o.is_empty()));
        });
    }
    merge("# source\n", "- target\n", false, |text, _, _| {
        assert_eq!(text, "- target\n# source\n")
    });
}

#[test]
fn alias_looking_examples_are_protected_in_both_formats() {
    let _guard = crate::integration_lock();
    for (source, target, org) in [
        (
            "```\nalias:: example\n```\n- source\n",
            "alias:: kept\n- target\n",
            false,
        ),
        (
            "#+BEGIN_EXAMPLE\n#+alias: example\n#+END_EXAMPLE\n* source\n",
            "#+alias: kept\n* target\n",
            true,
        ),
    ] {
        for ending in ["\n", "\r\n"] {
            let source = source.replace('\n', ending);
            let target = target.replace('\n', ending);
            merge(&source, &target, org, |text, _, proof| {
                assert!(text.contains("example"));
                assert!(proof.trace.consumed.is_empty());
                assert!(text.starts_with(target.split_inclusive('\n').next().unwrap()));
            });
        }
    }
}

#[test]
fn org_title_after_a_clashing_directive_stays_inside_the_leftover_quote() {
    let _guard = crate::integration_lock();
    merge(
        "#+CATEGORY: source\n#+TITLE: src\n* source\n",
        "#+TITLE: dst\n#+CATEGORY: target\n* target\n",
        true,
        |text, _, _| {
            assert_eq!(text,"#+TITLE: dst\n#+CATEGORY: target\n* target\n*\n#+BEGIN_QUOTE\n#+CATEGORY: source\n#+TITLE: src\n#+END_QUOTE\n* source\n");
        },
    );
}
