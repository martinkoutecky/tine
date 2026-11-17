use super::*;

#[test]
fn parent_success_header_bytes_are_preserved() {
    // Exported from ebffef508's exact writer; each row independently passed
    // O6/P2 there. Keep these bytes pinned rather than deriving them here.
    let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../tests/fixtures/cm-merge-header-parent.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 16);
    let root = tempfile::tempdir().unwrap();
    let store = tine_store::Store::open(root.path(), Default::default())
        .unwrap()
        .0;
    for row in rows {
        let source = row["source"].as_str().unwrap();
        let target = row["target"].as_str().unwrap();
        let src = store
            .page_of(&tine_store::PageId::from("pages/src.md"), source.as_bytes())
            .unwrap()
            .doc;
        let dst = store
            .page_of(&tine_store::PageId::from("pages/dst.md"), target.as_bytes())
            .unwrap()
            .doc;
        let proposal = propose(
            &dst,
            target,
            &src,
            source,
            &Default::default(),
            &Default::default(),
            false,
            store.config().file_name_format,
        )
        .unwrap();
        assert_eq!(proposal.text, row["expected"].as_str().unwrap(), "{row}");
    }
}

#[test]
fn overlapping_header_observations_require_both_fields_and_the_prose() {
    use tine_core::cm_check::{self, Atom};
    let root = tempfile::tempdir().unwrap();
    let store = tine_store::Store::open(root.path(), Default::default())
        .unwrap()
        .0;
    let names = store.config().file_name_format;
    let text = |atoms: &[Atom]| {
        atoms
            .iter()
            .map(|a| match a {
                Atom::Text(s) => s.as_str(),
                Atom::Site(s) | Atom::Title(s, _) => s.native.as_str(),
            })
            .collect::<String>()
    };
    let source = cm_check::observe("a:: x\n \nb:: y\n", Format::Md, names);
    let target = cm_check::observe("- target\n", Format::Md, names);
    let expected = cm_check::expected(
        &cm_check::Rename::none(None, names),
        Format::Md,
        cm_check::Expect::Survivor {
            source: &source,
            target: &target,
        },
    );
    assert_eq!(
        source
            .header
            .iter()
            .map(|h| h.key.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(
        expected
            .header
            .iter()
            .map(|h| h.key.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(source.blocks.len(), 1);
    assert_eq!(expected.blocks.len(), 2);
    assert_eq!(text(&source.blocks[0].body), text(&expected.blocks[1].body));
    eprintln!("overlap source={source:#?}\nO6 expected={expected:#?}");
    assert_eq!(text(&source.blocks[0].body), " \nb:: y");
}

#[test]
fn prose_uses_the_readers_content_end_and_excludes_only_layout() {
    let root = tempfile::tempdir().unwrap();
    let store = tine_store::Store::open(root.path(), Default::default())
        .unwrap()
        .0;
    for (org, target) in [(false, "prose\n \n- target\n"), (true, "#+K: v\n \n")] {
        let source = if org { "* source\n" } else { "- source\n" };
        let ext = if org { "org" } else { "md" };
        let page = |name, text: &str| {
            store
                .page_of(&tine_store::PageId::from(name), text.as_bytes())
                .unwrap()
                .doc
        };
        let dst = page(format!("pages/target.{ext}"), target);
        let src = page(format!("pages/source.{ext}"), source);
        let header = Header::read(&dst, target, org);
        assert_eq!(
            &target[header.prose.clone().unwrap()],
            if org { " " } else { "prose" },
            "pre={:?}",
            dst.pre_block
        );
        let proposal = propose(
            &dst,
            target,
            &src,
            source,
            &Default::default(),
            &Default::default(),
            org,
            store.config().file_name_format,
        )
        .unwrap();
        assert!(proposal
            .trace
            .consumed
            .iter()
            .all(|&id| proposal.trace.pieces[id].role == Role::Layout));
        let fmt = if org { Format::Org } else { Format::Md };
        let names = store.config().file_name_format;
        let rename = tine_core::cm_check::Rename::new("", "", None, names);
        let expected = tine_core::cm_check::expected(
            &rename,
            fmt,
            tine_core::cm_check::Expect::Survivor {
                source: &tine_core::cm_check::observe(source, fmt, names),
                target: &tine_core::cm_check::observe(target, fmt, names),
            },
        );
        assert!(
            tine_core::cm_check::matches(
                &expected,
                &tine_core::cm_check::observe(&proposal.text, fmt, names),
                true
            ),
            "{:?}",
            proposal.text
        );
    }
}

#[test]
fn prose_content_comparison_visits_each_observed_byte_once() {
    let root = tempfile::tempdir().unwrap();
    let store = tine_store::Store::open(root.path(), Default::default())
        .unwrap()
        .0;
    for lines in [64, 1024] {
        for org in [false, true] {
            let target = format!(
                "{}\n \n{}\n",
                "prose\n".repeat(lines).trim_end_matches('\n'),
                if org { "* target" } else { "- target" }
            );
            let source = if org { "* source\n" } else { "- source\n" };
            let ext = if org { "org" } else { "md" };
            let page = |name, text: &str| {
                store
                    .page_of(&tine_store::PageId::from(name), text.as_bytes())
                    .unwrap()
                    .doc
            };
            let dst = page(format!("pages/target.{ext}"), &target);
            let src = page(format!("pages/source.{ext}"), source);
            let prose =
                tine_core::page_properties::preamble_block(dst.pre_block.as_deref().unwrap(), org)
                    .unwrap();
            BODY_BYTES.with(|count| count.set(0));
            propose(
                &dst,
                &target,
                &src,
                source,
                &Default::default(),
                &Default::default(),
                org,
                store.config().file_name_format,
            )
            .unwrap();
            let expected = prose.raw().len() + "target".len() + "source".len();
            assert_eq!(BODY_BYTES.with(|count| count.get()), expected);
            eprintln!("{lines} prose lines, org={org}: {expected} content bytes compared once");
        }
    }
}

#[test]
fn org_final_breaks_are_only_consumed_join_layout() {
    let root = tempfile::tempdir().unwrap();
    let store = tine_store::Store::open(root.path(), Default::default())
        .unwrap()
        .0;
    for (eol, source_eol) in [("\n", "\n"), ("\r\n", "\r\n"), ("\r", "\r"), ("\n", "\r")] {
        let target = format!("* target{eol}{eol}{eol}");
        let source = format!("* source{source_eol}{source_eol}");
        let page = |name, text: &str| {
            store
                .page_of(&tine_store::PageId::from(name), text.as_bytes())
                .unwrap()
                .doc
        };
        let proposal = propose(
            &page("pages/target.org", &target),
            &target,
            &page("pages/source.org", &source),
            &source,
            &Default::default(),
            &Default::default(),
            true,
            store.config().file_name_format,
        )
        .unwrap();
        assert_eq!(proposal.text, format!("* target{eol}{source}"));
        assert!(proposal
            .trace
            .output
            .iter()
            .all(|p| matches!(p, Placement::Original(_))));
        let consumed: String = proposal
            .trace
            .consumed
            .iter()
            .map(|&id| {
                assert_eq!(proposal.trace.pieces[id].role, Role::Layout);
                proposal.trace.original(id)
            })
            .collect();
        assert_eq!(consumed, eol.repeat(2));
        proposal
            .trace
            .check(&proposal.text, &proposal.trace.substitution)
            .unwrap();
    }
}

#[test]
fn source_indentation_work_is_one_visit_per_line_and_bounded_by_merge_work() {
    let root = tempfile::tempdir().unwrap();
    let store = tine_store::Store::open(root.path(), Default::default())
        .unwrap()
        .0;
    let page = |name, text: &str| {
        store
            .page_of(&tine_store::PageId::from(name), text.as_bytes())
            .unwrap()
            .doc
    };
    let target = "- target\n";
    let dst = page("pages/target.md", target);
    for blocks in [64, 1024] {
        let aligned = "- source\n  continuation\n  - child\n".repeat(blocks);
        let indented = "  - source\n    continuation\n    - child\n".repeat(blocks);
        let normal = page("pages/source.md", &aligned);
        let source = page("pages/source.md", &indented);
        let merge = |doc: &PageDto, text: &str| {
            propose(
                &dst,
                target,
                doc,
                text,
                &Default::default(),
                &Default::default(),
                false,
                store.config().file_name_format,
            )
            .unwrap()
        };
        BODY_LINES.with(|count| count.set(0));
        BODY_BYTES.with(|count| count.set(0));
        merge(&normal, &aligned);
        assert_eq!(
            BODY_LINES.with(|count| count.get()),
            0,
            "aligned sources need no normalization pass"
        );
        let expected_bytes =
            ("source".len() + "continuation".len() + "child".len()) * blocks + "target".len();
        assert_eq!(
            BODY_BYTES.with(|count| count.get()),
            expected_bytes,
            "each native content byte is compared once, using the existing output read"
        );
        BODY_BYTES.with(|count| count.set(0));
        merge(&source, &indented);
        assert_eq!(
            BODY_LINES.with(|count| count.get()),
            3 * blocks,
            "normalization must visit each physical source line once"
        );
        assert_eq!(
            BODY_BYTES.with(|count| count.get()),
            expected_bytes,
            "indent removal adds no second content check"
        );
        let best = |doc: &PageDto, text: &str| {
            (0..3)
                .map(|_| {
                    let start = std::time::Instant::now();
                    std::hint::black_box(merge(doc, text));
                    start.elapsed()
                })
                .min()
                .unwrap()
        };
        let baseline = best(&normal, &aligned);
        let normalized = best(&source, &indented);
        eprintln!("{blocks} roots: aligned {baseline:?}, indented {normalized:?}");
        assert!(
            normalized <= baseline * 4 + std::time::Duration::from_millis(50),
            "normalizing indentation must stay a small multiple of existing merge work"
        );
    }
}

#[test]
fn source_and_survivor_trace_context_survive_the_io_boundary() {
    use super::super::MergeRefused;
    let root = tempfile::tempdir().unwrap();
    let store = tine_store::Store::open(root.path(), Default::default())
        .unwrap()
        .0;
    let links = HashMap::from([("a".into(), "B".into())]);
    let src = tine_store::PageId::from("pages/source.md");
    let dst = tine_store::PageId::from("pages/survivor.md");
    for source_fails in [false, true] {
        let text = "title:: A/[[A]]\n\n- body\n";
        let (source, target) = if source_fails {
            (text, "- target\n")
        } else {
            ("- source\n", text)
        };
        let failure = propose(
            &store.page_of(&dst, target.as_bytes()).unwrap().doc,
            target,
            &store.page_of(&src, source.as_bytes()).unwrap().doc,
            source,
            &links,
            &Default::default(),
            false,
            store.config().file_name_format,
        )
        .unwrap_err();
        let error = MergeRefused::error(failure, &src, &dst);
        let refused = MergeRefused::of(&error).unwrap();
        assert_eq!(
            refused.reason,
            RealizationError::Trace(TraceError::TitleIdentity)
        );
        assert_eq!(
            refused.file,
            if source_fails {
                src.as_str()
            } else {
                dst.as_str()
            }
        );
    }
}

#[test]
fn ordinary_line_endings_never_require_an_internal_trace_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let store = tine_store::Store::open(dir.path(), Default::default())
        .unwrap()
        .0;
    let endings = ["\n", "\r\n", "\r", ""];
    for org in [false, true] {
        for source_eol in endings {
            for target_eol in endings {
                for middle in ["\n", "\r\n", "\r"] {
                    let (key, bullet, ext) = if org {
                        ("#+k:", "*", "org")
                    } else {
                        ("k::", "-", "md")
                    };
                    let source = format!("{key} source{middle}{bullet} source{source_eol}");
                    let target = format!("{key} target{middle}{bullet} target{target_eol}");
                    let page = |name, text: &str| {
                        store
                            .page_of(
                                &tine_store::PageId::from(format!("pages/{name}.{ext}")),
                                text.as_bytes(),
                            )
                            .unwrap()
                            .doc
                    };
                    if let Err(error) = propose(
                        &page("target", &target),
                        &target,
                        &page("source", &source),
                        &source,
                        &Default::default(),
                        &Default::default(),
                        org,
                        store.config().file_name_format,
                    ) {
                        assert!(!matches!(error.reason, RealizationError::Trace(_) | RealizationError::FusedLineEnding),
                            "ordinary ending refused: source={source:?}, target={target:?}, error={error:?}");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod drawer_cost_tests {
    use super::{propose, Header, DRAWER_STEPS};

    /// A survivor head of `drawers` three-line Org property drawers.
    fn survivor(drawers: usize) -> String {
        let mut text = String::new();
        for k in 0..drawers {
            text.push_str(&format!(":PROPERTIES:\n:K{k}: v\n:END:\n"));
        }
        text + "* target\n"
    }

    fn merge(target: &str) -> usize {
        // The store reads the given bytes; this crate's sources touch no path.
        let dir = tempfile::tempdir().unwrap();
        let store = tine_store::Store::open(dir.path(), Default::default())
            .unwrap()
            .0;
        let source = ":PROPERTIES:\n:S: w\n:END:\n* source\n";
        let page = |p: &str, bytes: &str| {
            let id = tine_core::model::PageId::from(p);
            store.page_of(&id, bytes.as_bytes()).unwrap().doc
        };
        let (dst, src) = (page("pages/dst.org", target), page("pages/src.org", source));
        DRAWER_STEPS.with(|n| n.set(0));
        let _ = propose(
            &dst,
            target,
            &src,
            source,
            &Default::default(),
            &Default::default(),
            true,
            store.config().file_name_format,
        );
        DRAWER_STEPS.with(|n| n.get())
    }

    /// Lower bound: every header line's drawer is decided once, from a cursor
    /// that only moves forward, so the comparisons are O(lines + drawers);
    /// every delimiter test is O(1).
    #[test]
    fn drawer_classification_is_linear_in_lines_and_drawers() {
        for drawers in [1, 64, 1024, 8192] {
            let target = survivor(drawers);
            let lines = 3 * drawers + 1 + 4;
            let steps = merge(&target);
            eprintln!("{drawers} drawers, {lines} lines: {steps} comparisons");
            assert!(
                steps <= 2 * (lines + drawers),
                "{drawers} drawers, {lines} lines: {steps} comparisons"
            );
        }
    }

    /// Timing: classifying the head costs a small multiple of parsing it once
    /// (the parser's answer it maps), with 8,192 drawers in a 261 KB head.
    #[test]
    fn drawer_classification_stays_within_a_parse_of_the_head() {
        let target = survivor(8192);
        let page = tine_core::model::PageDto {
            name: "dst".into(),
            title: "dst".into(),
            kind: tine_core::model::PageKind::Page,
            pre_block: Some(target[..target.len() - "* target\n".len()].into()),
            blocks: Vec::new(),
            rev: None,
            format: tine_core::model::Format::Org,
            read_only: false,
            guide: false,
        };
        let best = |f: &mut dyn FnMut()| {
            (0..3)
                .map(|_| {
                    let t = std::time::Instant::now();
                    f();
                    t.elapsed()
                })
                .min()
                .unwrap()
        };
        let parse = best(&mut || {
            std::hint::black_box(tine_core::block_regions::parse_document(&target, true));
        });
        let classify = best(&mut || {
            let header = Header::read(&page, &target, true);
            let delimiters = (0..header.lines.len())
                .filter(|&n| header.delimiter(n))
                .count();
            assert_eq!(delimiters, 2 * 8192);
        });
        eprintln!("classification {classify:?} vs one head parse {parse:?}");
        assert!(
            classify <= parse * 6 + std::time::Duration::from_millis(50),
            "classification {classify:?} vs one head parse {parse:?}"
        );
    }
}

#[cfg(test)]
mod header_property_tests {
    use super::{Format, Header, PageDto};

    fn lines(text: &str, org: bool) -> Vec<Option<(String, String)>> {
        let page = PageDto {
            name: "Header".into(),
            title: "Header".into(),
            kind: tine_core::model::PageKind::Page,
            pre_block: Some(text.into()),
            blocks: Vec::new(),
            rev: None,
            format: if org { Format::Org } else { Format::Md },
            read_only: false,
            guide: false,
        };
        Header::read(&page, text, org)
            .lines
            .into_iter()
            .map(|line| line.property)
            .collect()
    }

    fn property(text: &str, org: bool) -> Option<(String, String)> {
        lines(text, org).into_iter().next().flatten()
    }

    #[test]
    fn merge_recognizes_unicode_page_headers_in_both_formats() {
        assert_eq!(
            property("klíč:: hodnota", false),
            Some(("klíč".into(), "hodnota".into()))
        );
        assert_eq!(
            property("#+klíč: hodnota", true),
            Some(("klíč".into(), "hodnota".into()))
        );
        assert_eq!(
            property("#+a.b/c: value", true),
            Some(("a.b/c".into(), "value".into()))
        );
    }

    #[test]
    fn a_fenced_example_is_never_a_header_property() {
        let prose = "```\nnote:: keep me\n```\nkey:: v\n";
        assert!(lines(prose, false).is_empty());
        assert_eq!(
            lines(&format!("kept:: yes\n{prose}"), false),
            vec![Some(("kept".into(), "yes".into()))],
            "only the leading page property belongs to the header; the fence and following property stay prose"
        );
    }
}
