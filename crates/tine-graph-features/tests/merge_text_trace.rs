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

fn header_aliases(doc: &tine_core::model::PageDto, org: bool) -> Vec<String> {
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

/// The terminator of every output line: original lines keep their own, a
/// rebuilt alias field takes the model's field ending, and a line the merge
/// inserts takes the survivor's Markdown convention. Org inserts LF: its model keeps a CRLF's `\r` in the
/// line text, and the guarded saves below prove the survivor stays writable.
#[test]
fn valid_merge_keeps_every_line_ending() {
    let _guard = crate::integration_lock();
    let mut failures = Vec::new();
    for org in [false, true] {
        let (key, bullet, title) = if org {
            ("#+alias: ", "*", "#+title: src")
        } else {
            ("alias:: ", "-", "title:: src")
        };
        let field = format!("{key}[[a]], [[a]]\n");
        let cases = [
            (
                format!("{bullet} source\n"),
                format!("{field}{bullet} target\n"),
            ),
            (
                format!("{field}{bullet} source\n"),
                format!("{bullet} target\n"),
            ),
            (
                format!("{title}\n{bullet} source\n"),
                format!("{bullet} target\n"),
            ),
            (
                format!("{bullet} source\n"),
                format!("{bullet} target\n{bullet} last"),
            ),
        ];
        for (src, dst) in cases {
            for (src_end, dst_end) in [
                ("\n", "\n"),
                ("\r\n", "\r\n"),
                ("\r\n", "\n"),
                ("\n", "\r\n"),
            ] {
                let src = src.replace('\n', src_end);
                let dst = dst.replace('\n', dst_end);
                // The model's active field ending (`mergeField`): the
                // survivor's first declaration's, else LF.
                let alias_end = if dst.starts_with(key) {
                    Some(dst_end)
                } else {
                    src.starts_with(key).then_some("\n")
                };
                // Collect every witness rather than stopping at the first.
                let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    valid_merge(&src, &dst, org, |doc, proof| {
                        let text = &proof.text;
                        if alias_end.is_some() {
                            assert_eq!(header_aliases(doc, org), ["a"]);
                        }
                        for line in src.split_inclusive('\n').chain(dst.split_inclusive('\n')) {
                            if line.ends_with('\n') && !line.starts_with(key) {
                                assert!(text.contains(line), "lost {line:?}: {text:?}");
                            }
                        }
                        if src_end == dst_end && !org {
                            // A uniform Markdown pair stays uniform outside the
                            // alias field: no LF-only line in a CRLF page.
                            let body: String = text
                                .split_inclusive('\n')
                                .filter(|l| !l.starts_with(key))
                                .collect();
                            let lone_lf = body.replace("\r\n", "").contains('\n');
                            let crlf = body.contains("\r\n");
                            assert!(
                                if dst_end == "\r\n" { !lone_lf } else { !crlf },
                                "mixed terminators: {text:?}"
                            );
                        }
                        if let Some(ending) = alias_end {
                            let line = text
                                .split_inclusive('\n')
                                .find(|l| l.starts_with(key))
                                .unwrap();
                            assert!(
                                line.ends_with(ending)
                                    && (ending == "\r\n" || !line.ends_with("\r\n")),
                                "alias line {line:?} lost its {ending:?}: {text:?}"
                            );
                        }
                    })
                }));
                match run {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => failures.push(error),
                    Err(panic) => failures.push(format!(
                        "org={org}, source={src:?}, target={dst:?}: {}",
                        panic
                            .downcast_ref::<String>()
                            .map_or("panic", String::as_str)
                    )),
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} valid merges failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Physical lines with their terminators: CRLF, LF or a lone CR ends a line,
/// as mldoc's `eol_chars` and Tine's reader read it. The oracle's own split,
/// independent of the writer's.
fn physical_lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let end = rest.find(['\r', '\n']).map_or(rest.len(), |at| {
            at + if rest[at..].starts_with("\r\n") { 2 } else { 1 }
        });
        out.push(&rest[..end]);
        rest = &rest[end..];
    }
    out
}

/// Lone-CR files are valid pages (REG-CM-MERGE-LONE-CR): they merge in both
/// formats, with headers on either side and endings mixed within one file.
/// Every original line keeps its exact terminator (a lone CR is never fused
/// into a CRLF), and in an all-CR pair every inserted line takes CR too.
#[test]
fn valid_merge_keeps_lone_cr_line_endings() {
    let _guard = crate::integration_lock();
    let mut failures = Vec::new();
    for org in [false, true] {
        let (key, b, title) = if org {
            ("#+alias: ", "*", "#+title: src")
        } else {
            ("alias:: ", "-", "title:: src")
        };
        // (source, target, aliases, exact survivor bytes when pinned)
        let cases: Vec<(String, String, Vec<&str>, Option<String>)> = vec![
            // The review's witness: a lone-CR source into an LF target.
            (
                format!("{b} source\r{b} second\r"),
                format!("{b} target\n"),
                vec![],
                Some(format!("{b} target\n{b} source\r{b} second\r")),
            ),
            (
                format!("{b} source\r"),
                format!("{b} target\r"),
                vec![],
                Some(format!("{b} target\r{b} source\r")),
            ),
            // An unterminated survivor gets the file's own CR.
            (
                format!("{b} source\r"),
                format!("{b} first\r{b} target"),
                vec![],
                Some(format!("{b} first\r{b} target\r{b} source\r")),
            ),
            // A source alias header becomes the survivor's field verbatim.
            (
                format!("{key}a\r{b} source\r"),
                format!("{b} target\r"),
                vec!["a"],
                Some(format!("{key}a\r{b} target\r{b} source\r")),
            ),
            // A target field rebuilt from duplicates, plus a source member.
            (
                format!("{key}b\r{b} source\r"),
                format!("{key}[[a]], [[a]]\r{b} target\r"),
                vec!["a", "b"],
                None,
            ),
            // A source title becomes leftover text.
            (
                format!("{title}\r{b} source\r"),
                format!("{b} target\r"),
                vec![],
                None,
            ),
            // Fields on both sides, the target's retained.
            (
                format!("{key}a, b\r{title}\r{b} source\r"),
                format!("{key}c\r{b} target\r"),
                vec!["c", "a", "b"],
                None,
            ),
            // Mixed endings within each file.
            (
                format!("{key}a\r\n{b} one\r{b} two\n{b} three\r\n"),
                format!("{key}b\n{b} target\r"),
                vec!["b", "a"],
                None,
            ),
            (
                format!("{title}\n{key}a\r{b} one\r\n"),
                format!("{key}[[b]], [[b]]\r\n{b} target\r{b} last"),
                vec!["b", "a"],
                None,
            ),
            // A survivor ending in a lone CR before a source that opens
            // with a blank LF line: the two must not fuse into one CRLF.
            (
                format!("\n{b} source\n"),
                format!("{b} target\r"),
                vec![],
                None,
            ),
            (
                format!("{title}\r\n{b} source\n"),
                format!("{key}a\r\n\r{b} target\r"),
                vec!["a"],
                None,
            ),
        ];
        for (src, dst, aliases, exact) in cases {
            let all_cr = !format!("{src}{dst}").contains('\n');
            let no_crlf = !format!("{src}{dst}").contains("\r\n");
            // Org's quote frame would end `#+BEGIN_QUOTE` with the survivor's
            // CR right before the source's blank LF line: one CRLF, two lines
            // joined. It refuses, files unchanged (RECEIPT-cm-b4-fix3 Q4).
            if org && src.starts_with('\n') && dst.ends_with('\r') {
                match valid_merge(&src, &dst, org, |_, _| {}) {
                    Err(e) if e.contains("FusedLineEnding") => {}
                    other => failures.push(format!("expected FusedLineEnding: {other:?}")),
                }
                continue;
            }
            let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                valid_merge(&src, &dst, org, |doc, proof| {
                    let text = &proof.text;
                    assert_eq!(header_aliases(doc, org), aliases, "{text:?}");
                    if let Some(exact) = &exact {
                        assert_eq!(text, exact);
                    }
                    let out = physical_lines(text);
                    for line in physical_lines(&src).into_iter().chain(physical_lines(&dst)) {
                        let terminated = line.ends_with(['\r', '\n']);
                        if terminated && !line.starts_with(key) {
                            assert!(
                                out.iter().any(|l| l.ends_with(line)),
                                "lost {line:?} with its own terminator: {text:?}"
                            );
                        }
                    }
                    if all_cr {
                        assert!(!text.contains('\n'), "an LF in an all-CR pair: {text:?}");
                    }
                    if no_crlf {
                        assert!(!text.contains("\r\n"), "a CR fused with an LF: {text:?}");
                    }
                })
            }));
            match run {
                Ok(Ok(())) => {}
                Ok(Err(error)) => failures.push(error),
                Err(panic) => failures.push(format!(
                    "org={org}, source={src:?}, target={dst:?}: {}",
                    panic
                        .downcast_ref::<String>()
                        .map_or("panic", String::as_str)
                )),
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} lone-CR merges failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// A declaration whose value has no members for a reason the reader allows
/// (wholly quoted, empty) is never the active field; its text stays protected.
#[test]
fn valid_merge_keeps_memberless_alias_declarations_as_text() {
    let _guard = crate::integration_lock();
    let mut failures = Vec::new();
    for org in [false, true] {
        let (key, bullet) = if org {
            ("#+alias: ", "*")
        } else {
            ("alias:: ", "-")
        };
        for empty in ["\"literal\"", ""] {
            let none = if empty.is_empty() {
                format!("{}\n", key.trim_end())
            } else {
                format!("{key}{empty}\n")
            };
            let some = format!("{key}a\n");
            for (src, dst) in [
                (
                    format!("{some}{bullet} source\n"),
                    format!("{none}{bullet} target\n"),
                ),
                (
                    format!("{none}{bullet} source\n"),
                    format!("{some}{bullet} target\n"),
                ),
                (
                    format!("{bullet} source\n"),
                    format!("{none}{some}{bullet} target\n"),
                ),
                (
                    format!("{none}{some}{bullet} source\n"),
                    format!("{bullet} target\n"),
                ),
            ] {
                for ending in ["\n", "\r\n"] {
                    let src = src.replace('\n', ending);
                    let dst = dst.replace('\n', ending);
                    if let Err(error) = valid_merge(&src, &dst, org, |doc, proof| {
                        assert_eq!(header_aliases(doc, org), ["a"], "{:?}", proof.text);
                        assert!(proof.text.contains(empty), "{:?}", proof.text);
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
        // Both models are LF-canonical (REG-ORG-CRLF-EMPTY-HEADLINE); the
        // file keeps its own endings.
        let raw = |s: &str| s.to_owned();
        assert!(
            bytes.contains(&format!("target prose{ending}")),
            "{bytes:?}"
        );
        assert!(
            bytes.contains(&format!("{{{{query [[A]]}}}}{ending}")),
            "{bytes:?}"
        );
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

/// Org pages whose page properties sit in a head `:PROPERTIES:` drawer
/// (REG-CM-MERGE-ORG-DRAWER), for each line ending: (source, target, header
/// aliases, header property keys, exact survivor bytes when pinned). Shared
/// with the Native15c admission test.
pub(crate) fn org_drawer_cases(
    e: &str,
) -> Vec<(
    String,
    String,
    Vec<&'static str>,
    Vec<&'static str>,
    Option<String>,
)> {
    let drawer = |entries: &[&str]| {
        let mut text = format!(":PROPERTIES:{e}");
        for entry in entries {
            text.push_str(&format!("{entry}{e}"));
        }
        text + &format!(":END:{e}")
    };
    let src = format!("* source{e}");
    let dst = format!("* target{e}");
    vec![
        // The review's F1 witness: a target drawer alias, a headerless source.
        (
            src.clone(),
            drawer(&[":ALIAS: b"]) + &dst,
            vec!["b"],
            vec!["alias"],
            Some(drawer(&[":ALIAS: b"]) + &dst + &src),
        ),
        // A target drawer alias takes a source directive's member.
        (
            format!("#+alias: a{e}{src}"),
            drawer(&[":ALIAS: b"]) + &dst,
            vec!["b", "a"],
            vec!["alias"],
            None,
        ),
        // The review's F2 witnesses: a source drawer joins the target header
        // as a whole container.
        (
            drawer(&[":TYPE: x"]) + &src,
            dst.clone(),
            vec![],
            vec!["type"],
            Some(drawer(&[":TYPE: x"]) + &dst + &src),
        ),
        (
            drawer(&[":ALIAS: a"]) + &src,
            dst.clone(),
            vec!["a"],
            vec!["alias"],
            Some(drawer(&[":ALIAS: a"]) + &dst + &src),
        ),
        // Both drawers, nonclashing: the source container follows the target's.
        (
            drawer(&[":TYPE: x", ":ALIAS: a"]) + &src,
            drawer(&[":ALIAS: b"]) + &dst,
            vec!["b", "a"],
            vec!["alias", "type"],
            None,
        ),
        // Both drawers, the source title clashing with nothing it may replace:
        // it is leftover text; its other entry joins.
        (
            drawer(&[":TITLE: src", ":TAGS: t"]) + &src,
            drawer(&[":TYPE: y"]) + &dst,
            vec![],
            vec!["type", "tags"],
            None,
        ),
        // A source drawer before target directives keeps every property
        // readable by Logseq too (the drawer comes first).
        (
            drawer(&[":TYPE: x"]) + &format!("#+alias: a{e}{src}"),
            format!("#+tags: t{e}{dst}"),
            vec!["a"],
            vec!["type", "tags", "alias"],
            None,
        ),
        // A target drawer and directive take a source directive.
        (
            format!("#+type: x{e}{src}"),
            drawer(&[":ALIAS: b"]) + &format!("#+tags: t{e}{dst}"),
            vec!["b"],
            vec!["alias", "tags", "type"],
            None,
        ),
        // Two source drawers, one empty.
        (
            drawer(&[":TYPE: x"]) + &drawer(&[]) + &src,
            dst.clone(),
            vec![],
            vec!["type"],
            None,
        ),
        // An empty drawer on either side.
        (src.clone(), drawer(&[]) + &dst, vec![], vec![], None),
        (drawer(&[]) + &src, dst.clone(), vec![], vec![], None),
    ]
}

fn header_keys(doc: &tine_core::model::PageDto) -> Vec<String> {
    let mut keys: Vec<_> = tine_core::page_header::page_header_text(
        doc.pre_block.as_deref(),
        doc.blocks.first().map(|b| b.raw.as_str()),
    )
    .into_iter()
    .flat_map(|header| tine_core::page_header::page_property_lines(header, true))
    .map(|(k, _)| k)
    .collect();
    keys.sort();
    keys
}

/// A valid Org page with a head drawer merges (REG-CM-MERGE-ORG-DRAWER): the
/// merger reads, places and checks page properties through the parser's one
/// view, so drawer entries count as page properties on both sides.
#[test]
fn valid_merge_reads_org_head_drawers_as_page_properties() {
    let _guard = crate::integration_lock();
    let mut failures = Vec::new();
    for e in ["\n", "\r\n", "\r"] {
        for (src, dst, aliases, keys, exact) in org_drawer_cases(e) {
            let mut keys: Vec<String> = keys.into_iter().map(String::from).collect();
            keys.sort();
            let result = valid_merge(&src, &dst, true, |doc, proof| {
                assert_eq!(header_aliases(doc, true), aliases, "{src:?} into {dst:?}");
                assert_eq!(header_keys(doc), keys, "{src:?} into {dst:?}");
                if let Some(exact) = &exact {
                    assert_eq!(&proof.text, exact);
                }
            });
            if let Err(error) = result {
                failures.push(error);
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Where the model has no answer for a head drawer yet, the merge refuses and
/// both pages stay as they were (RECEIPT-cm-b4-fix4 Q5-Q7): a clashing or
/// equal source drawer entry, or a drawer alias's protected duplicate member,
/// would lose its property occurrence inside the leftover quote; leftover
/// drawer delimiters would turn a following title into a tag; and a rebuilt
/// field's `#+alias: ` key cannot sit in a drawer.
#[test]
fn org_drawer_merges_the_model_does_not_settle_refuse_unchanged() {
    let _guard = crate::integration_lock();
    let drawer = |entry: &str| format!(":PROPERTIES:\n{entry}\n:END:\n");
    for (src, dst, error) in [
        (
            drawer(":TYPE: x") + "* source\n",
            drawer(":TYPE: y") + "* target\n",
            "HiddenOccurrence",
        ),
        (
            drawer(":TYPE: y") + "* source\n",
            drawer(":TYPE: y") + "* target\n",
            "HiddenOccurrence",
        ),
        (
            drawer(":TYPE: x") + "* source\n",
            "#+type: y\n* target\n".to_string(),
            "HiddenOccurrence",
        ),
        (
            drawer(":ALIAS: #[[a]], b") + "* source\n",
            "#+alias: a\n* target\n".to_string(),
            "HiddenOccurrence",
        ),
        (
            "* source\n".to_string(),
            drawer(":ALIAS: a, a") + "* target\n",
            "AliasField",
        ),
        // Leftover drawer delimiters make the reader take the source title
        // that follows them for a `#+title` tag (Q7).
        (
            drawer(":ALIAS: c d") + "#+title: src\n* source\n",
            drawer(":TAGS: t") + "* target\n",
            "AddedOccurrence",
        ),
        (
            ":PROPERTIES:\n:END:\n#+title: src\n* source\n".to_string(),
            "#+alias: x\n* target\n".to_string(),
            "AddedOccurrence",
        ),
    ] {
        let error_text = valid_merge(&src, &dst, true, |_, _| {}).unwrap_err();
        assert!(error_text.ends_with(error), "{error_text}");
    }
}

/// Graph with `files` in `pages/`, a host, and a snapshot check that every
/// file is byte-for-byte unchanged and still opens.
fn unchanged_after(
    files: &[(&str, String)],
    operation: impl FnOnce(&Arc<Store>, &tine_store::PageHost) -> std::io::Result<()>,
) -> std::io::Error {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    for (name, text) in files {
        fs::write(dir.path().join("pages").join(name), text).unwrap();
    }
    let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    let host = crate::test_host(&store);
    let ids: Vec<_> = files
        .iter()
        .map(|(name, _)| tine_core::model::PageId::from(format!("pages/{name}").as_str()))
        .collect();
    for id in &ids {
        store.page(id).expect("every input page opens");
    }
    let error = operation(&store, &host).expect_err("an output the store cannot open is refused");
    // A host operation that could not confirm in time keeps running: wait
    // for it to publish or give up, then restart and look.
    let changed = || {
        files.iter().any(|(name, text)| {
            fs::read(dir.path().join("pages").join(name))
                .ok()
                .as_deref()
                != Some(text.as_bytes())
        })
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
    while error.kind() == std::io::ErrorKind::TimedOut
        && !changed()
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    drop(host);
    drop(store);
    let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    let host = crate::test_host(&store);
    for ((name, text), id) in files.iter().zip(&ids) {
        let now = fs::read(dir.path().join("pages").join(name)).unwrap_or_default();
        assert!(
            now == text.as_bytes(),
            "{name} changed ({} bytes): {error}",
            now.len()
        );
        store.page(id).expect("every page still opens");
    }
    drop(host);
    error
}

#[test]
fn a_merge_never_publishes_a_survivor_the_store_cannot_open() {
    let _guard = crate::integration_lock();
    // Each page opens; together they exceed the page size limit.
    let half = tine_store::PARSE_INPUT_MAX_BYTES as usize / 2 + 1024;
    let page = |name: &str| format!("- {name} {}\n", "x".repeat(half));
    let files = [("src.md", page("source")), ("dst.md", page("target"))];
    let error = unchanged_after(&files, |store, host| {
        pages::merge_pages(store, host, "pages/src.md", "pages/dst.md")
    });
    assert!(error.to_string().contains("byte parse limit"), "{error}");
    let error = unchanged_after(&files, |store, host| {
        pages::rename_page_as(
            store,
            host,
            "src",
            "dst",
            None,
            pages::RenameChoice::MergeInto("pages/dst.md"),
            &[],
        )
        .map(drop)
    });
    assert!(error.to_string().contains("byte parse limit"), "{error}");
}

#[test]
fn a_rename_never_publishes_a_page_the_store_cannot_open() {
    let _guard = crate::integration_lock();
    // A rename's growth carries a page across the size limit: a referrer
    // (rewritten in place) and the renamed page itself (moved and rewritten).
    let long = "n".repeat(200);
    let refs = "[[a]] ".repeat(64);
    let filler = |used: usize| "x".repeat(tine_store::PARSE_INPUT_MAX_BYTES as usize - used - 16);
    let body = format!("- {refs}\n- ");
    let referrer = format!("{body}{}\n", filler(body.len()));
    for files in [
        vec![("a.md", "- a\n".to_string()), ("r.md", referrer.clone())],
        vec![("a.md", referrer)],
    ] {
        let error = unchanged_after(&files, |store, host| {
            pages::rename_page_expected(store, host, "a", &long, None)
        });
        assert!(error.to_string().contains("byte parse limit"), "{error}");
    }
}

/// Markdown decodes a backslash before ASCII punctuation in a plain alias
/// member (`C\#` is the alias `C#`); Org keeps the backslash. A member moves
/// with its own bytes, escapes included, deduplicates by those bytes (SPEC
/// §5.1) and reads back as the parser's decoded name.
#[test]
fn escaped_alias_members_move_with_their_bytes() {
    let _guard = crate::integration_lock();
    let punctuation: String = (b'!'..=b'~')
        .filter(u8::is_ascii_punctuation)
        .map(char::from)
        .collect();
    let mut cases: Vec<(String, String, String, Vec<String>)> = vec![
        (
            "C\\#".into(),
            "kept".into(),
            "kept, C\\#".into(),
            vec!["kept".into(), "C#".into()],
        ),
        (
            "foo\\*bar".into(),
            "kept".into(),
            "kept, foo\\*bar".into(),
            vec!["kept".into(), "foo*bar".into()],
        ),
        (
            "\\#x, a\\[b".into(),
            "C\\#".into(),
            "C\\#, \\#x, a\\[b".into(),
            vec!["C#".into(), "#x".into(), "a[b".into()],
        ),
        (
            "C\\#, b".into(),
            "C\\#".into(),
            "C\\#, b".into(),
            vec!["C#".into(), "b".into()],
        ),
        (
            "b".into(),
            "x\\\\, C\\#, C\\#".into(),
            "x\\\\, C\\#, b".into(),
            vec!["x\\".into(), "C#".into(), "b".into()],
        ),
        // Different bytes are different spellings, even when they read the same.
        (
            "C\\#".into(),
            "C#".into(),
            "C#, C\\#".into(),
            vec!["C#".into(), "C#".into()],
        ),
    ];
    let mut other = Vec::new();
    for c in punctuation.chars() {
        // Only where the parser reads one decoded plain member.
        let member = format!("p\\{c}q");
        if tine_core::property_refs::linkable_names(&member, false) != [format!("p{c}q")] {
            other.push(member);
            continue;
        }
        cases.push((
            format!("p\\{c}q"),
            "kept".into(),
            format!("kept, p\\{c}q"),
            vec!["kept".into(), format!("p{c}q")],
        ));
    }
    let mut failures = Vec::new();
    for (source, target, field, aliases) in &cases {
        let src = format!("alias:: {source}\n- source\n");
        let dst = format!("alias:: {target}\n- target\n");
        let result = valid_merge(&src, &dst, false, |doc, proof| {
            assert!(
                proof.text.starts_with(&format!("alias:: {field}\n")),
                "{source:?} into {target:?}: {:?}",
                proof.text
            );
            assert_eq!(
                &header_aliases(doc, false),
                aliases,
                "{source:?} into {target:?}"
            );
        });
        if let Err(error) = result {
            failures.push(error);
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    // A decoded separator splits the member (OG splits the decoded text).
    assert_eq!(
        other,
        ["p\\,q"],
        "escapes the parser does not read as one member"
    );
    // Org keeps the backslash: the same bytes are the name.
    valid_merge(
        "#+alias: C\\#\n* source\n",
        "#+alias: kept\n* target\n",
        true,
        |doc, proof| {
            assert!(
                proof.text.starts_with("#+alias: kept, C\\#\n"),
                "{:?}",
                proof.text
            );
            assert_eq!(header_aliases(doc, true), ["kept", "C\\#"]);
        },
    )
    .unwrap();
}

/// Review r6 F1: a case-only namespace rename (`Old` to `old`) reaches a child
/// whose title holds a reference already spelled the new way. The rename
/// succeeds and rewrites the child's title prefix, in Markdown and Org.
#[test]
fn a_case_only_namespace_rename_keeps_a_self_spelled_reference_in_a_child_title() {
    let _guard = crate::integration_lock();
    for (child, text, want) in [
        (
            "Old___c.md",
            "title:: Old/[[old]]\n- body\n",
            "title:: old/[[old]]\n",
        ),
        (
            "Old___c.org",
            "#+TITLE: Old/[[old]]\n* body\n",
            "#+TITLE: old/[[old]]\n",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let pages_dir = dir.path().join("pages");
        fs::create_dir(&pages_dir).unwrap();
        fs::write(pages_dir.join("Old.md"), "- a\n").unwrap();
        fs::write(pages_dir.join(child), text).unwrap();
        let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
        let host = crate::test_host(&store);
        pages::rename_page_expected(&store, &host, "Old", "old", None)
            .unwrap_or_else(|e| panic!("{child}: {e}"));
        let written: Vec<String> = fs::read_dir(&pages_dir)
            .unwrap()
            .map(|e| fs::read_to_string(e.unwrap().path()).unwrap())
            .collect();
        assert!(
            written.iter().any(|t| t.starts_with(want)),
            "{child}: {written:?}"
        );
    }
}
