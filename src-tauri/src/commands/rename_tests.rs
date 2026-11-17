use super::*;

#[test]
fn rename_question_preserves_duplicate_stale_and_unindexed_refusal_data() {
    for (files, old, new, selected, kind) in [
        (
            vec![
                ("pages/one.md", b"title:: A\n\n- a\n".as_slice()),
                ("pages/two.md", b"title:: A\n\n- a\n".as_slice()),
            ],
            "A",
            "B",
            None,
            "duplicateFiles",
        ),
        (
            vec![("pages/A.md", b"- a\n".as_slice())],
            "A",
            "B",
            Some("pages/missing.md"),
            "staleSource",
        ),
        (
            vec![
                ("pages/A.md", b"- a\n".as_slice()),
                ("pages/bad.md", b"- \xff [[A]]\n".as_slice()),
            ],
            "A",
            "B",
            None,
            "unindexed",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("pages")).unwrap();
        for (path, bytes) in files {
            std::fs::write(root.path().join(path), bytes).unwrap();
        }
        let store = tine_store::Store::open(root.path(), Default::default())
            .unwrap()
            .0;
        let error = window_offer(&store, old, new, selected).unwrap_err();
        let wire = serde_json::to_value(error).unwrap();
        assert_eq!(wire["refused"]["kind"], kind, "{wire}");
        if kind == "duplicateFiles" {
            assert_eq!(
                wire["refused"]["files"],
                serde_json::json!(["pages/one.md", "pages/two.md"])
            );
        }
    }
}

#[test]
fn ordinary_merge_limitations_name_the_failing_input_at_the_window_boundary() {
    use tine_graph_features::pages;
    for (source, target, ext, kind, file) in [
        (
            "#+END_QUOTE\n#+title: source\n* source\n",
            "#+title: target\n* target\n",
            "org",
            "directiveEscape",
            "source",
        ),
        (
            // The Markdown tag `#a\_b` declares `a_b`, which no contiguous
            // native run spells.
            "alias:: #a\\_b\n- source\n",
            "- target\n",
            "md",
            "aliasSpelling",
            "source",
        ),
        (
            "- source\n",
            "alias:: #a\\_b\n- target\n",
            "md",
            "aliasSpelling",
            "target",
        ),
        (
            "a:: x\n \nb:: y\n",
            "- target\n",
            "md",
            "headerProseOverlap",
            "source",
        ),
        (
            "- source\n",
            "a:: x\n \nb:: y\n",
            "md",
            "headerProseOverlap",
            "target",
        ),
        ("\na::", "- target\n", "md", "headerProseOverlap", "source"),
    ] {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("pages")).unwrap();
        let src = format!("pages/source.{ext}");
        let dst = format!("pages/target.{ext}");
        std::fs::write(root.path().join(&src), source).unwrap();
        std::fs::write(root.path().join(&dst), target).unwrap();
        let store = std::sync::Arc::new(
            tine_store::Store::open(root.path(), Default::default())
                .unwrap()
                .0,
        );
        let data = tempfile::tempdir().unwrap();
        let host = tine_store::PageHost::start_for_tests(&store, data.path()).unwrap();
        for named in [false, true] {
            let error = if named {
                pages::rename_page_as(
                    &store,
                    &host,
                    "source",
                    "target",
                    Some(&src),
                    pages::RenameChoice::MergeInto(&dst),
                    &[],
                )
                .map(|_| ())
            } else {
                pages::merge_pages(&store, &host, &src, &dst)
            }
            .unwrap_err();
            let message = error.to_string();
            let wire = serde_json::to_value(RewriteError::of(error)).unwrap();
            assert_eq!(wire["refused"]["kind"], kind, "named={named}: {message}");
            if kind == "aliasSpelling" {
                assert_eq!(wire["refused"]["alias"], "alias:: #a\\_b");
                assert!(message.contains("alias:: #a\\_b"), "{message}");
            }
            if kind == "headerProseOverlap" {
                assert!(message.contains("both properties and prose"), "{message}");
                assert!(
                    message.contains("remove blank lines before or between the property lines"),
                    "{message}"
                );
                assert!(!message.contains("top-level blocks"), "{message}");
            }
            assert_eq!(wire["refused"]["file"], format!("pages/{file}.{ext}"));
            assert!(!message.contains("bug in Tine"), "{message}");
            assert!(message.contains("nothing was merged"), "{message}");
            assert_eq!(
                std::fs::read_to_string(root.path().join(&src)).unwrap(),
                source
            );
            assert_eq!(
                std::fs::read_to_string(root.path().join(&dst)).unwrap(),
                target
            );
        }
        drop(host);
        drop(store);
        let repaired = |text: &str| {
            text.replace("#+END_QUOTE\n", "")
                .replace("#a\\_b", "a_b")
                .replace("\n \n", "\n")
                .trim_start_matches('\n')
                .to_owned()
        };
        std::fs::write(root.path().join(&src), repaired(source)).unwrap();
        std::fs::write(root.path().join(&dst), repaired(target)).unwrap();
        let store = std::sync::Arc::new(
            tine_store::Store::open(root.path(), Default::default())
                .unwrap()
                .0,
        );
        let host = tine_store::PageHost::start_for_tests(&store, data.path()).unwrap();
        pages::merge_pages(&store, &host, &src, &dst)
            .expect("the suggested edit permits the merge with P2 and P3 enforced");
        assert!(!root.path().join(&src).exists());
        assert!(std::fs::read_to_string(root.path().join(&dst))
            .unwrap()
            .contains(if kind == "headerProseOverlap" {
                "a::"
            } else {
                "source"
            }));
    }
}

#[test]
fn indented_sources_merge_through_both_command_entries_with_checks_enforced() {
    use tine_graph_features::pages;
    for named in [false, true] {
        for eol in ["\n", "\r\n"] {
            for source in [
                "  - indented source\n",
                "  - title:: source\n    - child\n  - source\n",
            ] {
                let root = tempfile::tempdir().unwrap();
                std::fs::create_dir_all(root.path().join("pages")).unwrap();
                std::fs::write(
                    root.path().join("pages/source.md"),
                    source.replace('\n', eol),
                )
                .unwrap();
                std::fs::write(
                    root.path().join("pages/target.md"),
                    format!("- target{eol}"),
                )
                .unwrap();
                let store = std::sync::Arc::new(
                    tine_store::Store::open(root.path(), Default::default())
                        .unwrap()
                        .0,
                );
                let data = tempfile::tempdir().unwrap();
                let host = tine_store::PageHost::start_for_tests(&store, data.path()).unwrap();
                let answer = if named {
                    window_answer(pages::rename_page_as(
                        &store,
                        &host,
                        "source",
                        "target",
                        Some("pages/source.md"),
                        pages::RenameChoice::MergeInto("pages/target.md"),
                        &[],
                    ))
                    .map(|_| ())
                } else {
                    pages::merge_pages(&store, &host, "pages/source.md", "pages/target.md")
                        .map_err(RewriteError::of)
                };
                answer.unwrap();
                assert!(!root.path().join("pages/source.md").exists());
                let merged = std::fs::read_to_string(root.path().join("pages/target.md")).unwrap();
                assert!(merged.starts_with(&format!("- target{eol}")));
                assert!(merged.contains("source"));
            }
        }
    }
}

#[test]
fn merge_input_trace_failures_keep_their_own_file() {
    use tine_graph_features::pages::merge_payload::{self, MergePart, RealizationError};
    let root = tempfile::tempdir().unwrap();
    let store = tine_store::Store::open(root.path(), Default::default())
        .unwrap()
        .0;
    let links = std::collections::HashMap::from([("a".into(), "B".into())]);
    for source_fails in [false, true] {
        let text = "title:: A/[[A]]\n\n- body\n";
        let (source, target) = if source_fails {
            (text, "- target\n")
        } else {
            ("- source\n", text)
        };
        let page = |path: &str, text: &str| {
            store
                .page_of(&tine_store::PageId::from(path), text.as_bytes())
                .unwrap()
                .doc
        };
        let error = merge_payload::propose(
            &page("pages/target.md", target),
            target,
            &page("pages/source.md", source),
            source,
            &links,
            &Default::default(),
            false,
            store.config().file_name_format,
        )
        .unwrap_err();
        assert_eq!(
            error.reason,
            RealizationError::Trace(tine_core::text_trace::TraceError::TitleIdentity)
        );
        assert_eq!(
            error.part,
            if source_fails {
                MergePart::Source
            } else {
                MergePart::Survivor
            }
        );
    }
}

#[test]
fn a_merge_refusal_after_flush_saves_pending_edits_but_merges_nothing() {
    use tine_graph_features::pages;
    for named in [false, true] {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("pages")).unwrap();
        let source = "alias:: #a\\_b\n- source\n";
        std::fs::write(root.path().join("pages/source.md"), source).unwrap();
        std::fs::write(root.path().join("pages/target.md"), "- target\n").unwrap();
        let store = std::sync::Arc::new(
            tine_store::Store::open(root.path(), Default::default())
                .unwrap()
                .0,
        );
        store.whole_graph().unwrap();
        let data = tempfile::tempdir().unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let host = tine_store::PageHost::start(
            &store,
            data.path(),
            "pending-refusal",
            |_, _| {},
            move |mail| {
                let _ = send.send(serde_json::to_value(mail).unwrap());
            },
        )
        .unwrap();
        let session = serde_json::to_value(host.window_reloaded()).unwrap()["session"]
            .as_u64()
            .unwrap();
        let target = tine_store::PageId::from("pages/target.md");
        let opened =
            serde_json::to_value(host.open(session, 1, &target, "target").unwrap()).unwrap();
        let next = |id| loop {
            let mail = receive
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("page host must answer the pending input request");
            if mail["answer"]["id"] == id {
                break mail;
            }
        };
        let version = next(1)["page"]["version"].as_u64().unwrap();
        let pause = host.pause_for_tests(std::slice::from_ref(&target));
        let edited = "- target edited\n";
        let dto = store.page_of(&target, edited.as_bytes()).unwrap().doc;
        host.submit(
            session,
            2,
            opened["key"].as_str().unwrap(),
            &dto,
            version,
            None,
            &[tine_store::EditKind::SaveBlock],
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join(target.as_str())).unwrap(),
            "- target\n"
        );
        assert!(
            !host
                .owed(session, Some(std::slice::from_ref(&target)))
                .unwrap()
                .is_empty(),
            "the submitted edit must still be pending publication"
        );
        let error = std::thread::scope(|scope| {
            let run = scope.spawn(|| {
                if named {
                    pages::rename_page_as(
                        &store,
                        &host,
                        "source",
                        "target",
                        Some("pages/source.md"),
                        pages::RenameChoice::MergeInto("pages/target.md"),
                        &[],
                    )
                    .map(|_| ())
                } else {
                    pages::merge_pages(&store, &host, "pages/source.md", "pages/target.md")
                }
            });
            drop(pause);
            run.join().unwrap().unwrap_err()
        });
        let message = error.to_string();
        assert_eq!(
            std::fs::read_to_string(root.path().join(target.as_str())).unwrap(),
            edited
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("pages/source.md")).unwrap(),
            source
        );
        assert_eq!(
            pages::MergeRefused::of(&error).unwrap().file,
            "pages/source.md"
        );
        assert!(message.contains("nothing was merged"), "{message}");
        assert!(!message.contains("nothing was changed"), "{message}");
    }
}

#[test]
fn unchanged_save_recovery_cost_is_one_page_and_no_graph_walk() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("pages")).unwrap();
    std::fs::write(root.path().join("pages/r.md"), "- tags:: [[Old]]\n").unwrap();
    let store = std::sync::Arc::new(
        tine_store::Store::open(root.path(), Default::default())
            .unwrap()
            .0,
    );
    store.whole_graph().unwrap();
    let data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, data.path()).unwrap();
    assert!(host.quiescent_for_tests());
    tine_store::cost_counters::reset();
    let started = std::time::Instant::now();
    tine_graph_features::pages::save_page_unchanged(&store, &host, "pages/r.md").unwrap();
    // Publication returns before draft cleanup. Count through the host's
    // semantic idle boundary; this fixture deliberately has no watcher.
    let deadline = started + std::time::Duration::from_secs(2);
    while !host.quiescent_for_tests() {
        assert!(
            std::time::Instant::now() < deadline,
            "host did not retire the saved operation"
        );
        std::thread::yield_now();
    }
    assert!(
        std::time::Instant::now() < deadline,
        "save exceeded the fixture's two-second cleanup bound"
    );
    let counts = tine_store::cost_counters::snapshot();
    eprintln!("unchanged save: {:?}, {counts:?}", started.elapsed());
    assert_eq!(
        counts.readdir, 0,
        "saving one named file must not enumerate the graph"
    );
    assert_eq!(
        counts.model_checks, 0,
        "the ordinary save does not run another rename check"
    );
    assert_eq!(
        (
            counts.full_reads,
            counts.preamble_reads,
            counts.files_written,
            counts.fsyncs
        ),
        (5, 2, 2, 5)
    );
}

#[test]
fn real_merge_refusals_name_the_source_or_survivor_and_preserve_both_files() {
    for (source, target, ext, kind, file) in [
        (
            "- alias:: X\n  - child\n- source\n",
            "- target\n",
            "md",
            "headerChildren",
            "source",
        ),
        (
            ":PROPERTIES:\n:K: source\n:END:\n* source\n",
            ":PROPERTIES:\n:K: target\n:END:\n* target\n",
            "org",
            "hiddenOccurrence",
            "source",
        ),
        (
            "- source\n",
            "- title:: other\n",
            "md",
            "survivorIdentity",
            "target",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("pages")).unwrap();
        let src = format!("pages/source.{ext}");
        let dst = format!("pages/target.{ext}");
        std::fs::write(root.path().join(&src), source).unwrap();
        std::fs::write(root.path().join(&dst), target).unwrap();
        let store = std::sync::Arc::new(
            tine_store::Store::open(root.path(), Default::default())
                .unwrap()
                .0,
        );
        let data = tempfile::tempdir().unwrap();
        let host = tine_store::PageHost::start_for_tests(&store, data.path()).unwrap();
        let error = tine_graph_features::pages::merge_pages(&store, &host, &src, &dst).unwrap_err();
        let wire = serde_json::to_value(RewriteError::of(error)).unwrap();
        assert_eq!(wire["refused"]["kind"], kind);
        assert_eq!(wire["refused"]["file"], format!("pages/{file}.{ext}"));
        assert_eq!(
            std::fs::read_to_string(root.path().join(src)).unwrap(),
            source
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join(dst)).unwrap(),
            target
        );
    }
}

#[test]
fn stopping_between_the_save_and_repeat_keeps_only_the_ordinary_save() {
    use std::io::{BufRead, Write};
    if let Some(root) = std::env::var_os("TINE_TEST_CM_SAVE_GAP_ROOT") {
        let data = std::env::var_os("TINE_TEST_CM_SAVE_GAP_DATA").unwrap();
        let store = std::sync::Arc::new(
            tine_store::Store::open(std::path::Path::new(&root), Default::default())
                .unwrap()
                .0,
        );
        let host =
            tine_store::PageHost::start_for_tests(&store, std::path::Path::new(&data)).unwrap();
        assert!(
            tine_graph_features::pages::save_page_unchanged(&store, &host, "pages/missing.md")
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(std::path::Path::new(&root).join("pages/r.md")).unwrap(),
            "- tags:: [[Old]]\n"
        );
        tine_graph_features::pages::save_page_unchanged(&store, &host, "pages/r.md").unwrap();
        println!("CM_SAVE_GAP_PUBLISHED");
        std::io::stdout().flush().unwrap();
        // The parent kills this process before the second operation starts.
        loop {
            std::thread::park();
        }
    }
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("pages")).unwrap();
    std::fs::write(root.path().join("pages/Old.md"), "- source\n").unwrap();
    std::fs::write(root.path().join("pages/r.md"), "- tags:: [[Old]]\n").unwrap();
    let data = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new(tine_store::test_binary::own_test_binary())
        .args(["--exact", "commands::rename::tests::stopping_between_the_save_and_repeat_keeps_only_the_ordinary_save", "--nocapture"])
        .env("TINE_TEST_CM_SAVE_GAP_ROOT", root.path())
        .env("TINE_TEST_CM_SAVE_GAP_DATA", data.path())
        .stdout(std::process::Stdio::piped()).spawn().unwrap();
    let mut output = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    loop {
        assert!(
            output.read_line(&mut line).unwrap() > 0,
            "child exited before saving: {line}"
        );
        if line.contains("CM_SAVE_GAP_PUBLISHED") {
            break;
        }
        line.clear();
    }
    child.kill().unwrap();
    assert!(!child.wait().unwrap().success());
    let store = std::sync::Arc::new(
        tine_store::Store::open(root.path(), Default::default())
            .unwrap()
            .0,
    );
    let _host = tine_store::PageHost::start_for_tests(&store, data.path()).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.path().join("pages/r.md")).unwrap(),
        "tags:: [[Old]]\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("pages/Old.md")).unwrap(),
        "- source\n"
    );
    assert!(!root.path().join("pages/New.md").exists());
}

#[test]
fn rewrite_boundary_preserves_both_check_failures_and_all_named_merge_refusals() {
    use tine_graph_features::pages::{
        merge_payload::RealizationError as M, CheckRefused, MergeRefused,
    };
    for guarantee in [tine_store::Guarantee::P2, tine_store::Guarantee::P3] {
        let error = std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            CheckRefused {
                guarantee,
                file: "pages/r.md".into(),
            },
        );
        let wire = serde_json::to_value(window_answer(Err(error)).unwrap_err()).unwrap();
        assert_eq!(wire["refused"]["kind"], "checkRefused");
        assert_eq!(wire["refused"]["guarantee"], format!("{guarantee:?}"));
        assert_eq!(wire["refused"]["file"], "pages/r.md");
    }
    for (reason, kind) in [
        (M::HeaderChildren, "headerChildren"),
        (M::HiddenOccurrence, "hiddenOccurrence"),
        (M::SurvivorIdentity, "survivorIdentity"),
        (M::BodyLaw, "bodyLayout"),
        (M::HeaderProseOverlap, "headerProseOverlap"),
        (M::IndentedBody, "indentedBody"),
        (M::DirectiveEscape, "directiveEscape"),
        (
            M::MissingNativeNameRange {
                declaration: "alias:: [[C\\#]]".into(),
            },
            "aliasSpelling",
        ),
        (
            M::Trace(tine_core::text_trace::TraceError::TitleIdentity),
            "titleIdentity",
        ),
    ] {
        let error = std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            MergeRefused {
                reason,
                file: "pages/Old.md".into(),
            },
        );
        let wire = serde_json::to_value(window_answer(Err(error)).unwrap_err()).unwrap();
        let mut expected = serde_json::json!({"refused":{"kind":kind,"file":"pages/Old.md"}});
        if kind == "aliasSpelling" {
            expected["refused"]["alias"] = serde_json::json!("alias:: [[C\\#]]");
        }
        assert_eq!(wire, expected);
    }
}

#[test]
fn rewrite_boundary_reports_internal_merge_writer_faults_as_tines_failed_check() {
    use tine_graph_features::pages::{merge_payload::RealizationError as M, MergeRefused};
    for reason in [
        M::Trace(tine_core::text_trace::TraceError::Partition),
        M::Trace(tine_core::text_trace::TraceError::Substitution),
        M::Trace(tine_core::text_trace::TraceError::Output),
        M::Trace(tine_core::text_trace::TraceError::Multiplicity),
        M::Trace(tine_core::text_trace::TraceError::Consumption),
        M::FusedLineEnding,
    ] {
        let error = std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            MergeRefused {
                reason,
                file: "pages/source.md".into(),
            },
        );
        let wire = serde_json::to_value(window_answer(Err(error)).unwrap_err()).unwrap();
        assert_eq!(
            wire,
            serde_json::json!({"refused": {
                "kind":"checkRefused", "guarantee":"P2", "file":"pages/source.md"
            }})
        );
    }
}

#[test]
fn explicit_unchanged_save_then_rename_or_merge_has_exact_ordinary_save_then_rename_bytes() {
    use tine_graph_features::pages;
    for merge in [false, true] {
        for eol in ["\n", "\r\n"] {
            let root = tempfile::tempdir().unwrap();
            let dir = root.path();
            std::fs::create_dir_all(dir.join("pages")).unwrap();
            let source = format!("- old{eol}");
            let referrer = format!("- tags:: [[Old]]{eol}");
            std::fs::write(dir.join("pages/Old.md"), &source).unwrap();
            std::fs::write(dir.join("pages/r.md"), &referrer).unwrap();
            if merge {
                std::fs::write(dir.join("pages/New.md"), format!("- new{eol}")).unwrap();
            }
            let store =
                std::sync::Arc::new(tine_store::Store::open(dir, Default::default()).unwrap().0);
            store.whole_graph().unwrap();
            let data = tempfile::tempdir().unwrap();
            let host = tine_store::PageHost::start_for_tests(&store, data.path()).unwrap();
            let choice = if merge {
                pages::RenameChoice::MergeInto("pages/New.md")
            } else {
                pages::RenameChoice::Rename
            };
            let rename = || {
                pages::rename_page_as(
                    &store,
                    &host,
                    "Old",
                    "New",
                    Some("pages/Old.md"),
                    choice,
                    &[],
                )
            };
            let refusal = window_answer(rename()).unwrap_err();
            let wire = serde_json::to_value(refusal).unwrap();
            assert_eq!(wire["refused"]["guarantee"], "P3");
            assert_eq!(
                std::fs::read_to_string(dir.join("pages/r.md")).unwrap(),
                referrer
            );
            assert_eq!(
                std::fs::read_to_string(dir.join("pages/Old.md")).unwrap(),
                source
            );
            pages::save_page_unchanged(&store, &host, "pages/r.md").unwrap();
            let saved = std::fs::read_to_string(dir.join("pages/r.md")).unwrap();
            assert_eq!(saved, format!("tags:: [[Old]]{eol}"));
            window_answer(rename()).unwrap();
            assert_eq!(
                std::fs::read_to_string(dir.join("pages/r.md")).unwrap(),
                format!("tags:: [[New]]{eol}")
            );
            assert!(!dir.join("pages/Old.md").exists());
            assert_eq!(
                std::fs::read_to_string(dir.join("pages/New.md")).unwrap(),
                if merge {
                    format!("- new{eol}- old{eol}")
                } else {
                    source
                }
            );
        }
    }
}

/// Card HFmqtY 2a at the command boundary: `rename_page` answers the window
/// through `window_answer`, over the same `rename_page_as` call. When the page
/// files moved and only the config.edn write failed, the window gets the
/// applied report (cause in `config_skipped`), not an error string it would
/// show as "Rename failed". Without the conversion this returns `Err` and fails.
#[cfg(unix)]
#[test]
fn a_failed_config_write_after_the_files_moved_reaches_the_window_as_the_rename() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    std::fs::create_dir_all(dir.join("pages")).unwrap();
    // The page operation's trash chain exists and stays writable; only
    // `logseq/` itself refuses the config transaction's new file.
    std::fs::create_dir_all(dir.join("logseq/.tine-trash/pages")).unwrap();
    let config = "{:file/name-format :triple-lowbar\n :default-home {:page \"Start\"}}\n";
    std::fs::write(dir.join("logseq/config.edn"), config).unwrap();
    std::fs::write(dir.join("pages/Start.md"), "- home body\n").unwrap();
    std::fs::write(dir.join("pages/Ref.md"), "- see [[Start]]\n").unwrap();
    let store = std::sync::Arc::new(tine_store::Store::open(dir, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
    let logseq = dir.join("logseq");
    std::fs::set_permissions(&logseq, std::fs::Permissions::from_mode(0o555)).unwrap();
    let writable = std::fs::write(logseq.join("probe"), "x").is_ok();
    let answer = window_answer(tine_graph_features::pages::rename_page_as(
        &store,
        &host,
        "Start",
        "Begin",
        None,
        tine_graph_features::pages::RenameChoice::Rename,
        &[],
    ));
    std::fs::set_permissions(&logseq, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!writable, "the injected failure needs a non-root user");
    let report = answer.expect("the window is told the rename happened");
    assert_eq!(
        report.outcome,
        tine_graph_features::pages::RenameOutcome::Renamed
    );
    assert!(
        report
            .config_skipped
            .as_deref()
            .is_some_and(|cause| !cause.is_empty()),
        "{report:?}"
    );
    assert_eq!(report.home_page, None);
    assert!(report
        .touched
        .iter()
        .any(|page| page.path == "pages/Start.md" && page.moved));
    assert_eq!(
        report
            .renamed
            .iter()
            .map(|page| (page.from.as_str(), page.to.as_str()))
            .collect::<Vec<_>>(),
        [("Start", "Begin")]
    );
    assert!(dir.join("pages/Begin.md").exists());
    assert_eq!(
        std::fs::read_to_string(dir.join("pages/Ref.md")).unwrap(),
        "- see [[Begin]]\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("logseq/config.edn")).unwrap(),
        config
    );
}

/// Card HVgII at the command boundary: a rename refused because config.edn is
/// not UTF-8 reaches the window as the wire family `config-unreadable`, which
/// the window words from its catalog, not as the bare read error ("invalid utf-8
/// sequence of 1 bytes from index 35"). Nothing is renamed.
#[test]
fn a_rename_refused_for_an_unreadable_config_reaches_the_window_as_its_wire_family() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    std::fs::create_dir_all(dir.join("pages")).unwrap();
    std::fs::create_dir_all(dir.join("logseq")).unwrap();
    std::fs::write(
        dir.join("logseq/config.edn"),
        b"{:default-home {:page \"Start\"}} ;; \xff\xfe\n",
    )
    .unwrap();
    std::fs::write(dir.join("pages/Start.md"), "- home body\n").unwrap();
    let store = std::sync::Arc::new(tine_store::Store::open(dir, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
    let refused = window_answer(tine_graph_features::pages::rename_page_as(
        &store,
        &host,
        "Start",
        "Begin",
        None,
        tine_graph_features::pages::RenameChoice::Rename,
        &[],
    ))
    .expect_err("a read-only graph refuses the rename");
    assert!(
        matches!(&refused, RewriteError::Message(message) if message.starts_with("config-unreadable: ")),
        "{refused}"
    );
    assert!(dir.join("pages/Start.md").exists());
    assert!(!dir.join("pages/Begin.md").exists());
}
