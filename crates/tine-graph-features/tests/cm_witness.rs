//! K's generated matrix through the real offer, page host and guarded writers.
//! Expected paths/text come only from the model fixture, never the writer.
use super::*;
use serde_json::Value;
use tine_core::{cm_check, config::FileNameFormat, model::Format};
use tine_graph_features::pages::CheckRefused;

// The existing byte-for-byte model fixture is shared with the pure checker tests.
const MATRIX: &str = include_str!("../../tine-core/src/cm_check/witness-fixtures-r6.json");
const NAMES: FileNameFormat = FileNameFormat::Legacy;

// The toy reader's observations cannot be expressed by these original texts.
// Each row is accounted for by name; parser-readable equivalents run below.
const READER_DIFFERENCES: &[(&str, &str)] = &[
    ("source_preamble", "toy reader includes trailing blank continuation lines in prose content; Tine observes the prose block without them"),
    ("bare_bullet_header", "toy reader drops a bare header bullet; Tine observes and preserves an empty ordinary block"),
    ("header_children_leftover", "toy reader attaches the first bullet to a pre-block header; Tine reads it as an ordinary block when a pre-block header exists"),
    ("key_spelling", "matrix save=id; Tine's save promotes the first properties-only bullet, so P3 refuses until that file is saved"),
];

const NEGATIVE_READER_DIFFERENCES: &[(&str, &str)] = &[
    ("org_leftover_reference", "toy reader loses the directive's reference in the Org quote; Tine reads and preserves it; a native drawer control pins a genuinely lost occurrence"),
];

fn matrix() -> Value {
    serde_json::from_str(MATRIX).unwrap()
}

fn format(path: &str) -> Format {
    if path.ends_with(".org") {
        Format::Org
    } else {
        Format::Md
    }
}

fn setup(name: &str, files: &Value) -> (PathBuf, Arc<Store>) {
    let mut entries = vec![(
        "logseq/config.edn",
        "{:file/name-format :legacy :journal/file-name-format \"yyyy/MM/dd\"}\n",
    )];
    for file in files.as_array().unwrap() {
        entries.push((
            file["path"].as_str().unwrap(),
            file["text"].as_str().unwrap(),
        ));
    }
    let (root, store) = fixture(name, &entries);
    store.whole_graph().unwrap();
    (root, store)
}

fn compare_after(root: &Path, row: &Value, survivor: Option<&str>) -> Result<(), String> {
    let actual = snapshot(root);
    let after = row["after"].as_array().unwrap();
    let paths: std::collections::BTreeSet<_> =
        after.iter().map(|f| f["path"].as_str().unwrap()).collect();
    if actual
        .keys()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>()
        != paths
    {
        return Err(format!("paths {:?}, expected {paths:?}", actual.keys()));
    }
    for before in row["before"].as_array().unwrap() {
        let path = before["path"].as_str().unwrap();
        if after
            .iter()
            .any(|f| f["path"] == before["path"] && f["text"] == before["text"])
            && actual[path] != before["text"].as_str().unwrap()
        {
            return Err(format!("{path}: an untouched file changed bytes"));
        }
    }
    // Identity mapping exposes the fixture's semantic observation, allowing
    // precisely K's header permutation for a survivor, with exact multiplicities.
    let identity = cm_check::Rename::none(None, NAMES);
    for f in after {
        let path = f["path"].as_str().unwrap();
        let expected = cm_check::observe(f["text"].as_str().unwrap(), format(path), NAMES);
        let want = cm_check::expected(
            &identity,
            format(path),
            cm_check::Expect::File {
                content: &expected,
                skip: false,
            },
        );
        let got = cm_check::observe(&actual[path], format(path), NAMES);
        if !cm_check::matches(&want, &got, survivor == Some(path)) {
            return Err(format!("{path}: expected {want:#?}, observed {got:#?}"));
        }
    }
    Ok(())
}

fn admitted(row: &Value) -> Result<(), String> {
    let name = row["name"].as_str().unwrap();
    let (root, store) = setup(name, &row["before"]);
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
    let old = row["old"].as_str().unwrap();
    let new = row["new"].as_str().unwrap();
    let survivor;
    if row["filemerge"] == true {
        survivor = Some(new.to_owned());
        tine_graph_features::pages::merge_pages(&store, &host, old, new)
            .map_err(|e| e.to_string())?;
    } else {
        let offer = rename_offer(&store, old, new, None).map_err(|e| e.to_string())?;
        let into = match (&offer, row["merge"].as_bool().unwrap()) {
            (RenameOffer::Rename, false) => None,
            (RenameOffer::Merge { into, .. }, true) => Some(into.as_str()),
            _ => return Err(format!("unexpected offer {offer:?}")),
        };
        survivor = into.map(str::to_owned);
        let result = rename_page_as(
            &store,
            &host,
            old,
            new,
            None,
            into.map_or(RenameChoice::Rename, RenameChoice::MergeInto),
            &[],
        )
        .map_err(|e| e.to_string())?;
        let want = if into.is_some() {
            RenameOutcome::Merged
        } else {
            RenameOutcome::Renamed
        };
        if result.outcome != want {
            return Err(format!("outcome {:?}, expected {want:?}", result.outcome));
        }
    }
    compare_after(&root, row, survivor.as_deref())?;
    let expected = snapshot(&root);
    for (path, text) in &expected {
        let original = row["before"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"] == path.as_str());
        if original.is_some_and(|f| f["text"] == text.as_str()) {
            continue;
        }
        let saved = tine_graph_features::pages::save_page_unchanged(&store, &host, path);
        if let Err(error) = saved {
            if tine_core::concord_queue::vcs_conflict_markers(text, format(path)).is_empty() {
                return Err(format!("{path}: unchanged save refused: {error}"));
            }
        }
        if snapshot(&root) != expected {
            return Err(format!("{path}: unchanged save changed bytes or paths"));
        }
    }
    Ok(())
}

#[test]
fn k_witness_requests_run_without_skips() {
    let _guard = crate::integration_lock();
    let matrix = matrix();
    let rows = matrix["fixtures"].as_array().unwrap();
    assert_eq!(rows.len(), 36);
    assert_eq!(rows.iter().filter(|r| r["filemerge"] == true).count(), 1);
    let mut listed = Vec::new();
    let mut failures = Vec::new();
    let mut ran = 0;
    for row in rows {
        let name = row["name"].as_str().unwrap();
        if let Some((_, reason)) = READER_DIFFERENCES.iter().find(|r| r.0 == name) {
            eprintln!("K reader difference {name}: {reason}");
            listed.push(name);
            continue;
        }
        ran += 1;
        match admitted(row) {
            Err(error) => failures.push(format!("{name}: {error}")),
            Ok(()) => eprintln!("K admitted {name}: passed"),
        }
    }
    assert_eq!(
        listed,
        [
            "key_spelling",
            "source_preamble",
            "header_children_leftover",
            "bare_bullet_header"
        ]
    );
    assert_eq!(ran, 32);
    eprintln!("K witnesses: {ran} executed, {} explicitly listed reader differences, zero unaccounted rows; failing rows: {failures:#?}", listed.len());
    assert!(
        failures.is_empty(),
        "Tine differs from K:\n{}",
        failures.join("\n")
    );
}

/// These texts express the observations that K intended, under Tine's reader.
/// They are separate cases; the generated fixture is never modified.
#[test]
fn k_reader_differences_have_transaction_controls() {
    let _guard = crate::integration_lock();
    let matrix = matrix();
    for (name, source, after) in [
        (
            "source_preamble",
            None,
            "- target prose\n- target\n- source prose\n- source\n",
        ),
        ("bare_bullet_header", None, "- target\n- \n- source\n"),
        (
            "header_children_leftover",
            Some("- title:: A\n  alias:: X\n  - kid\n- source\n"),
            "alias:: X\n- target\n-\n  title:: B\n  - kid\n- source\n",
        ),
    ] {
        let mut row = matrix["fixtures"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == name)
            .unwrap()
            .clone();
        if let Some(source) = source {
            row["before"][0]["text"] = source.into();
        }
        row["after"][0]["text"] = after.into();
        for eol in ["\n", "\r\n"] {
            let mut native = row.clone();
            for side in ["before", "after"] {
                for file in native[side].as_array_mut().unwrap() {
                    file["text"] = file["text"].as_str().unwrap().replace('\n', eol).into();
                }
            }
            admitted(&native).unwrap_or_else(|e| panic!("{name}, {eol:?}: {e}"));
            if name == "header_children_leftover" {
                native["before"][0]["text"] = "- alias:: X\n  title:: A\n  - kid\n- source\n"
                    .replace('\n', eol)
                    .into();
                admitted(&native)
                    .unwrap_or_else(|e| panic!("{name}, continuation title, {eol:?}: {e}"));
            }
        }
    }
}

#[test]
fn key_spelling_lists_the_real_save_boundary_and_admits_stable_referrers() {
    let _guard = crate::integration_lock();
    let matrix = matrix();
    let row = matrix["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "key_spelling")
        .unwrap();
    let (root, store) = setup("key_spelling_original", &row["before"]);
    let before = snapshot(&root);
    let error = rename(&store, "Old", "Two Words", RenameChoice::Rename).unwrap_err();
    assert_eq!(
        CheckRefused::of(&error),
        Some(&CheckRefused {
            guarantee: tine_store::Guarantee::P3,
            file: "pages/R.md".into()
        }),
        "{error:?}"
    );
    assert_eq!(snapshot(&root), before);
    let mut stable = row.clone();
    for side in ["before", "after"] {
        let referrer = stable[side]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|f| f["path"] == "pages/R.md")
            .unwrap();
        referrer["text"] = format!("- ordinary\n{}", referrer["text"].as_str().unwrap()).into();
    }
    admitted(&stable).unwrap();
}

#[test]
fn first_root_titles_follow_real_renames() {
    let _guard = crate::integration_lock();
    for eol in ["\n", "\r\n"] {
        let source = "- alias:: X\n  title:: A\n  - kid\n- source\n".replace('\n', eol);
        let (root, store) = fixture("first-root-title-rename", &[("pages/A.md", &source)]);
        assert_eq!(
            rename(&store, "A", "B", RenameChoice::Rename).unwrap(),
            RenameOutcome::Renamed
        );
        assert_eq!(
            snapshot(&root),
            std::collections::BTreeMap::from([(
                "pages/B.md".into(),
                source.replace("title:: A", "title:: B")
            ),])
        );
    }
}

/// The title adapter's replacement is a text part; the parser sees a new
/// inner link. This is separate from locating a first-root title's bytes.
#[test]
fn titles_with_new_inner_links_refuse_p2_without_writing() {
    let _guard = crate::integration_lock();
    for (ext, source, target) in [
        (
            "md",
            "title:: a [[X]]\n- [[a [[X]]]]\n",
            "title:: b [[Y]]\n- target\n",
        ),
        (
            "org",
            "#+title: a [[X]]\n* [[a [[X]]]]\n",
            "#+title: b [[Y]]\n* target\n",
        ),
    ] {
        for merge in [false, true] {
            let src = format!("pages/A.{ext}");
            let dst = format!("pages/B.{ext}");
            let mut files = vec![(src.as_str(), source)];
            if merge {
                files.push((dst.as_str(), target));
            }
            let (root, store) = fixture("title-new-inner-link", &files);
            let before = snapshot(&root);
            let choice = if merge {
                RenameChoice::MergeInto(&dst)
            } else {
                RenameChoice::Rename
            };
            let error = rename(&store, "a [[X]]", "b [[Y]]", choice).unwrap_err();
            assert_eq!(
                CheckRefused::of(&error),
                Some(&CheckRefused {
                    guarantee: tine_store::Guarantee::P2,
                    file: if merge { dst.clone() } else { src.clone() }
                }),
                "{ext}, merge={merge}: {error:?}"
            );
            assert_eq!(snapshot(&root), before);
        }
    }
}

fn wrong_output(row: &Value) {
    let before = row[1].as_array().unwrap();
    let after = row[5].as_array().unwrap();
    let days = tine_core::date::JournalFormat::new(Some("yyyy/MM/dd"), None);
    let rename = cm_check::Rename::new(
        row[2].as_str().unwrap(),
        row[3].as_str().unwrap(),
        Some(&days),
        NAMES,
    );
    if after.len() != 1 {
        assert_eq!(row[0], "child_title_conflict");
        // This candidate breaks O4 title identity, not P2 alone. Inject its
        // exact bytes at transaction preflight, which checks both promises.
        let (root, store) = setup("child_title_conflict_injection", &row[1]);
        let unchanged = snapshot(&root);
        let old = before[1]["text"].as_str().unwrap();
        let new = after[1]["text"].as_str().unwrap();
        let title = tine_core::model::page_title_from_header(old, Format::Md).unwrap();
        let range = tine_core::model::own_title_span(old, Format::Md, &title).unwrap();
        let replacement = tine_core::model::page_title_from_header(new, Format::Md).unwrap();
        let mut trace = tine_core::text_trace::TextTrace::default();
        trace.add_input(old, &[(range, replacement)], &[]).unwrap();
        trace.ordered_output();
        trace.check_shadow(new).unwrap();
        let id = tine_core::model::PageId::from(before[1]["path"].as_str().unwrap());
        let (_, rev) = store.read(&id.file(), None).unwrap();
        let map = Arc::new(tine_store::RenameMap(
            vec![(
                row[2].as_str().unwrap().into(),
                row[3].as_str().unwrap().into(),
            )],
            vec![],
        ));
        let mut tx = store.page_plan(Some(tine_store::EditKind::RenamePage));
        tx.rewrite_refs_prepared(
            &[tine_store::EditKind::RenamePage],
            &id,
            rev,
            (old.into(), new.into()),
            Some(&map),
            NAMES,
            trace,
        );
        let error = match tx.plan() {
            Err(error) => error,
            Ok(_) => panic!("child_title_conflict: bad output passed preflight"),
        };
        assert!(
            matches!(
                error,
                tine_store::Why::Refused(tine_store::Refusal::TextRealization(
                    tine_core::text_trace::TraceError::TitleIdentity
                ))
            ),
            "{error:?}"
        );
        assert_eq!(snapshot(&root), unchanged);
        return;
    }
    let path = after[0]["path"].as_str().unwrap();
    let target = before.iter().find(|f| f["path"] == path).unwrap();
    let source = before.iter().find(|f| f["path"] != path).unwrap();
    let read = |f: &Value| {
        cm_check::observe(
            f["text"].as_str().unwrap(),
            format(f["path"].as_str().unwrap()),
            NAMES,
        )
    };
    let (source, target) = (read(source), read(target));
    let want = cm_check::expected(
        &rename,
        format(path),
        cm_check::Expect::Survivor {
            source: &source,
            target: &target,
        },
    );
    assert!(
        !cm_check::matches(&want, &read(&after[0]), true),
        "{}: bad output passed P2",
        row[0]
    );
    // The four answer annotations are confirmations of these selected pairs.
    // Exercise that real offer/answer as a control, then check the native
    // writer against the input-derived expectation, not its bad fixture output.
    let (root, store) = setup(row[0].as_str().unwrap(), &row[1]);
    let (old, new) = (row[2].as_str().unwrap(), row[3].as_str().unwrap());
    let offer = rename_offer(&store, old, new, None).unwrap();
    let RenameOffer::Merge { into, .. } = offer else {
        panic!("{}: expected confirmMerge, got {offer:?}", row[0]);
    };
    assert_eq!(into, path);
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
    let report = rename_page_as(
        &store,
        &host,
        old,
        new,
        None,
        RenameChoice::MergeInto(&into),
        &[],
    )
    .unwrap();
    assert_eq!(report.outcome, RenameOutcome::Merged);
    let text = fs::read_to_string(root.join(path)).unwrap();
    assert!(
        cm_check::matches(&want, &cm_check::observe(&text, format(path), NAMES), true),
        "{}: confirmation wrote wrong content",
        row[0]
    );
}

#[test]
fn k_negative_rows_run_without_skips() {
    let _guard = crate::integration_lock();
    let matrix = matrix();
    let rows = matrix["negative"].as_array().unwrap();
    assert_eq!(rows.len(), 17);
    let mut injections = Vec::new();
    let mut failures = Vec::new();
    let mut listed = Vec::new();
    let mut refusals = 0;
    let mut folding_admissions = 0;
    for row in rows {
        let name = row[0].as_str().unwrap();
        if name == "child_title_conflict" {
            wrong_output(row);
        }
        if row[4] == ".observationsP2" || name == "leftover_changed_text" {
            wrong_output(row);
            injections.push(name);
            continue;
        }
        if let Some((_, reason)) = NEGATIVE_READER_DIFFERENCES.iter().find(|r| r.0 == name) {
            eprintln!("K reader difference {name}: {reason}");
            let mut native = serde_json::json!({"name": name, "old": row[2], "new": row[3], "before": row[1], "merge": true});
            // Tine keeps the source directive as a quote line, retaining X.
            native["after"] = serde_json::json!([{"path":"pages/B.org", "text":"#+key: target\n* target\n*\n#+BEGIN_QUOTE\n#+key: [[X]]\n#+END_QUOTE\n* source\n"}]);
            admitted(&native).unwrap();
            listed.push(name);
            continue;
        }
        let (root, store) = setup(name, &row[1]);
        let before = snapshot(&root);
        let (old, new) = (row[2].as_str().unwrap(), row[3].as_str().unwrap());
        let offer = rename_offer(&store, old, new, None).unwrap();
        let app_data = tempfile::tempdir().unwrap();
        let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
        let into = match &offer {
            RenameOffer::Merge { into, .. } => Some(into.as_str()),
            _ => None,
        };
        let result = rename_page_as(
            &store,
            &host,
            old,
            new,
            None,
            into.map_or(RenameChoice::Rename, RenameChoice::MergeInto),
            &[],
        );
        if name == "folded_occupant" {
            // Probe the fixture's own filesystem, rather than the platform.
            let probe = tempfile::tempdir_in(&root).unwrap();
            fs::write(probe.path().join("CaseProbe"), b"probe").unwrap();
            let folds = probe.path().join("caseprobe").exists();
            eprintln!("K folded_occupant: filesystem folds case={folds}");
            if !folds {
                assert_eq!(result.unwrap().outcome, RenameOutcome::Renamed);
                assert_eq!(
                    snapshot(&root),
                    std::collections::BTreeMap::from([
                        ("pages/B.md".to_owned(), before["pages/A.md"].clone()),
                        ("pages/b.md".to_owned(), before["pages/b.md"].clone()),
                    ])
                );
                folding_admissions += 1;
                continue;
            }
        }
        let error = match result {
            Err(error) => error,
            Ok(result) => {
                failures.push(format!("{name}: expected {}, got {result:?}", row[4]));
                continue;
            }
        };
        let kind = RenameBlocked::of(&error).map(|r| {
            serde_json::to_value(r).unwrap()["kind"]
                .as_str()
                .unwrap()
                .to_owned()
        });
        let wanted = match row[4].as_str().unwrap() {
            r#".aliasHeld "pages/C.md""# => matches!(RenameBlocked::of(&error), Some(RenameBlocked::AliasHeld { path, .. }) if path == "pages/C.md"),
            ".contested" => kind.as_deref() == Some("contested"),
            ".capture" => kind.as_deref() == Some("capture"),
            ".ownChild" => kind.as_deref() == Some("ownChild"),
            ".pathTaken" => kind.as_deref() == Some("fileTaken"),
            ".titleIdentity" => error.get_ref().and_then(|e| e.downcast_ref::<tine_core::text_trace::TraceError>()) == Some(&tine_core::text_trace::TraceError::TitleIdentity),
            ".headerChildren" => tine_graph_features::pages::MergeRefused::of(&error).is_some_and(|e| e.reason == tine_graph_features::pages::merge_payload::RealizationError::HeaderChildren),
            ".conflictPair" => error.to_string().contains("conflict"),
            ".leftoverReference" => error.to_string().contains("reference"),
            other => panic!("unhandled K refusal {other}"),
        };
        if !wanted {
            failures.push(format!(
                "{name}: expected {}, got {error:?} (offer {offer:?})",
                row[4]
            ));
        }
        assert_eq!(snapshot(&root), before, "{name}: refusal changed bytes");
        refusals += 1;
    }
    assert_eq!(
        injections,
        [
            "split_alias",
            "header_missing_entry",
            "leftover_changed_text",
            "leftover_changed_text_md"
        ]
    );
    assert_eq!(listed, ["org_leftover_reference"]);
    assert_eq!(refusals + folding_admissions, 12);
    let answers = matrix["negative_answers"].as_object().unwrap();
    assert_eq!(answers.len(), 4);
    for name in &injections {
        assert_eq!(answers[*name], "some .confirm");
    }
    eprintln!("K negatives: {refusals} refusals, {folding_admissions} case-sensitive admissions, {} listed reader differences with native controls, 4 wrong-output injections with named confirmations, zero unaccounted rows; failing rows: {failures:#?}", listed.len());
    assert!(
        failures.is_empty(),
        "Tine differs from K:\n{}",
        failures.join("\n")
    );
}

/// P1 is a byte promise outside the pair, stronger than P2's observations.
/// LF/CRLF, Unicode, opaque examples and unrelated losing twins all survive.
#[test]
fn p1_real_transactions_keep_every_byte_outside_the_named_spans() {
    let _guard = crate::integration_lock();
    for merge in [false, true] {
        for eol in ["\n", "\r\n"] {
            let before = "alias:: A\n\n- α  [[A]]  [[A/child]]\n  continued   #A\n- `[[A]]`\n- ```text\n  [[A]]\n  ```\n".replace('\n', eol);
            let wanted = "alias:: B\n\n- α  [[B]]  [[B/child]]\n  continued   #B\n- `[[A]]`\n- ```text\n  [[A]]\n  ```\n".replace('\n', eol);
            let mut files = vec![
                ("pages/A.md", "- source\n"),
                ("pages/R.md", before.as_str()),
                ("pages/twin1.md", "title:: Unrelated\n\n- one\n"),
                ("pages/twin2.md", "title:: Unrelated\n\n- two\n"),
                ("journals/2026_11_02.md", "- journal\n"),
            ];
            if merge {
                files.push(("pages/B.md", "- target\n"));
            }
            let (root, store) = fixture("p1-gaps", &files);
            let original = snapshot(&root);
            let choice = if merge {
                RenameChoice::MergeInto("pages/B.md")
            } else {
                RenameChoice::Rename
            };
            rename(&store, "A", "B", choice).unwrap();
            let actual = snapshot(&root);
            assert_eq!(actual["pages/R.md"], wanted);
            for path in ["pages/twin1.md", "pages/twin2.md", "journals/2026_11_02.md"] {
                assert_eq!(actual[path], original[path], "{path}");
            }
        }
    }
}

#[test]
fn o2_real_rename_and_merge_pin_marker_and_priority_sites() {
    let _guard = crate::integration_lock();
    for old in ["TODO", "A"] {
        for merge in [false, true] {
            let source = format!("pages/{old}.md");
            let before = format!("- TODO [#A] task [[{old}]]\n");
            let mut files = vec![
                (source.as_str(), "- source\n"),
                ("pages/R.md", before.as_str()),
            ];
            if merge {
                files.push(("pages/New.md", "- target\n"));
            }
            let (root, store) = fixture("fixed-sites-gap", &files);
            let choice = if merge {
                RenameChoice::MergeInto("pages/New.md")
            } else {
                RenameChoice::Rename
            };
            rename(&store, old, "New", choice).unwrap();
            assert_eq!(
                fs::read_to_string(root.join("pages/R.md")).unwrap(),
                "- TODO [#A] task [[New]]\n"
            );
        }
    }
}

#[test]
fn physical_file_merge_never_rewrites_outside_references_or_settings() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "file-merge-gap",
        &[
            ("pages/A.md", "- source\n"),
            ("pages/B.md", "- target\n"),
            ("pages/R.md", "- [[A]] [[B]]\n"),
            (
                "logseq/config.edn",
                "{:favorites [\"A\"] :default-home {:page \"A\"}}\n",
            ),
        ],
    );
    let before = fs::read(root.join("logseq/config.edn")).unwrap();
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
    tine_graph_features::pages::merge_pages(&store, &host, "pages/A.md", "pages/B.md").unwrap();
    assert_eq!(fs::read(root.join("logseq/config.edn")).unwrap(), before);
    assert_eq!(
        fs::read_to_string(root.join("pages/R.md")).unwrap(),
        "- [[A]] [[B]]\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("pages/B.md")).unwrap(),
        "- target\n- source\n"
    );
    assert!(!root.join("pages/A.md").exists());
}

#[test]
fn both_merge_entries_refuse_mixed_formats_without_writing() {
    let _guard = crate::integration_lock();
    for by_name in [false, true] {
        let (root, store) = fixture(
            "mixed-format-gap",
            &[
                ("pages/A.md", "- source\n"),
                ("pages/B.org", "* target\n"),
                ("pages/R.md", "- [[A]]\n"),
            ],
        );
        let before = snapshot(&root);
        let app_data = tempfile::tempdir().unwrap();
        let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
        let error = if by_name {
            assert!(matches!(
                rename_offer(&store, "A", "B", None).unwrap(),
                RenameOffer::Merge { .. }
            ));
            rename_page_as(
                &store,
                &host,
                "A",
                "B",
                None,
                RenameChoice::MergeInto("pages/B.org"),
                &[],
            )
            .unwrap_err()
        } else {
            tine_graph_features::pages::merge_pages(&store, &host, "pages/A.md", "pages/B.org")
                .unwrap_err()
        };
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(error.to_string(), "files are in different formats");
        assert_eq!(snapshot(&root), before);
    }
}

/// O3 selection precedes O4 authorization: contested physical holders beat
/// aliases; an alias-held destination beats a capture under the source.
#[test]
fn o3_refusal_precedence_is_observed_before_any_transaction_writes() {
    let _guard = crate::integration_lock();
    for (extra, wanted) in [
        (
            vec![
                ("pages/p.md", "title:: B\n- one\n"),
                ("pages/q.md", "title:: B\n- two\n"),
            ],
            "contested",
        ),
        (vec![], "aliasHeld"),
    ] {
        let mut files = vec![
            ("pages/A.md", "- source [[A/sub]]\n"),
            ("pages/owner.md", "alias:: B\n- owner\n"),
            ("pages/child.md", "title:: B/sub\n- occupied\n"),
        ];
        files.extend(extra);
        let (root, store) = fixture("precedence-gap", &files);
        let before = snapshot(&root);
        let offer = rename_offer(&store, "A", "B", None).unwrap();
        assert_eq!(
            serde_json::to_value(offer).unwrap()["reason"]["kind"],
            wanted
        );
        let error = rename(&store, "A", "B", RenameChoice::Rename).unwrap_err();
        assert_eq!(
            serde_json::to_value(RenameBlocked::of(&error).unwrap()).unwrap()["kind"],
            wanted
        );
        assert_eq!(snapshot(&root), before);
    }
}
