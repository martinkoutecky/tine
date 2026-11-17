//! Deterministic native-body placements through both checked merge entries.
use super::*;
use tine_core::{cm_check, model::Format};

struct Case {
    source: String,
    target: String,
    org: bool,
    rename: bool,
    refusal: Option<(&'static str, bool)>,
}

fn run(cases: Vec<Case>) {
    // Each transaction needs only its pair. Bound the fixture graph so the
    // matrix measures the writer rather than repeatedly indexing other cases.
    for batch in cases.chunks(64) {
        run_batch(batch);
    }
}

fn run_batch(cases: &[Case]) {
    let _guard = crate::integration_lock();
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("pages")).unwrap();
    let files: Vec<_> = cases
        .iter()
        .enumerate()
        .map(|(n, case)| {
            let source = format!("src{n}");
            let target = format!("dst{n}");
            let ext = if case.org { "org" } else { "md" };
            let render = |text: &str| text.replace("SOURCE", &source).replace("TARGET", &target);
            let a = format!("pages/{source}.{ext}");
            let b = format!("pages/{target}.{ext}");
            let src = render(&case.source);
            let dst = render(&case.target);
            fs::write(root.path().join(&a), &src).unwrap();
            fs::write(root.path().join(&b), &dst).unwrap();
            (source, target, a, b, src, dst)
        })
        .collect();
    let store = Arc::new(Store::open(root.path(), Default::default()).unwrap().0);
    let host = crate::test_host(&store);
    let mut failures = Vec::new();
    let (mut merged, mut refused) = (0, std::collections::BTreeMap::new());
    let started = std::time::Instant::now();
    for (n, (case, (source, target, a, b, src, dst))) in cases.iter().zip(&files).enumerate() {
        let result = if case.rename {
            pages::rename_or_merge_page(&store, &host, source, target, None, Some(b), &[])
                .map(|_| ())
        } else {
            pages::merge_pages(&store, &host, a, b).map(|_| ())
        };
        let label = || {
            format!(
                "case {n}, org={}, rename={}, source={src:?}, target={dst:?}",
                case.org, case.rename
            )
        };
        if let Err(error) = result {
            assert_eq!(fs::read_to_string(root.path().join(a)).unwrap(), *src);
            assert_eq!(fs::read_to_string(root.path().join(b)).unwrap(), *dst);
            match pages::MergeRefused::of(&error) {
                Some(reason) if reason.check_refused().is_none() && case.refusal.is_some() => {
                    let (kind, source) = case.refusal.unwrap();
                    assert_eq!(format!("{:?}", reason.reason), kind, "{}", label());
                    assert_eq!(reason.file, *if source { a } else { b }, "{}", label());
                    *refused.entry(format!("{:?}", reason.reason)).or_insert(0) += 1;
                }
                _ => failures.push(format!("{}: {error}", label())),
            }
            continue;
        }
        assert!(case.refusal.is_none(), "unexpected merge: {}", label());
        merged += 1;
        assert!(!root.path().join(a).exists(), "{}", label());
        let after = fs::read_to_string(root.path().join(b)).unwrap();
        let fmt = if case.org { Format::Org } else { Format::Md };
        let names = store.config().file_name_format;
        let rename = cm_check::Rename::new(
            if case.rename { source } else { "" },
            if case.rename { target } else { "" },
            None,
            names,
        );
        let expected = cm_check::expected(
            &rename,
            fmt,
            cm_check::Expect::Survivor {
                source: &cm_check::observe(src, fmt, names),
                target: &cm_check::observe(dst, fmt, names),
            },
        );
        assert!(
            cm_check::matches(&expected, &cm_check::observe(&after, fmt, names), true),
            "{}",
            label()
        );
        let id = tine_core::model::PageId::from(b.as_str());
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
            fs::read_to_string(root.path().join(b)).unwrap(),
            after,
            "unchanged save: {}",
            label()
        );
    }
    eprintln!(
        "native body matrix: {} cases, {merged} merged, {refused:?} refused, {} unexpected, {:?}",
        cases.len(),
        failures.len(),
        started.elapsed()
    );
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(merged > 0);
}

#[test]
fn review_sources_merge_with_code_intact_in_both_entries() {
    let mut cases = Vec::new();
    for source in [
        "  - first\n- second\n    code\n",
        "  - first\n- second\n  ```\n    code\n  ```\n",
    ] {
        for rename in [false, true] {
            for eol in ["\n", "\r\n"] {
                cases.push(Case {
                    source: source.replace('\n', eol),
                    target: format!("- target{eol}"),
                    org: false,
                    rename,
                    refusal: None,
                });
            }
        }
    }
    assert_eq!(cases.len(), 8);
    run(cases);
}

#[test]
fn review_prose_separators_merge_in_both_entries() {
    let mut cases = Vec::new();
    for rename in [false, true] {
        for ending in 0..4 {
            for survivor in [false, true] {
                for (separator, following) in [
                    (" ", "- target\n"),
                    ("\t", "- target\n"),
                    (" ", ""),
                    ("\t", ""),
                ] {
                    let prose = endings(&format!("prose\n{separator}\n{following}"), ending);
                    let plain = endings("- source\n", ending);
                    let (source, target) = if survivor {
                        (plain, prose)
                    } else {
                        (prose, plain)
                    };
                    cases.push(Case {
                        source,
                        target,
                        org: false,
                        rename,
                        refusal: None,
                    });
                }
            }
        }
    }
    assert_eq!(cases.len(), 64);
    run(cases);
}

fn endings(text: &str, pattern: usize) -> String {
    let mut result = String::new();
    for (n, line) in text.split_inclusive('\n').enumerate() {
        if let Some(content) = line.strip_suffix('\n') {
            result.push_str(content);
            result.push_str(match pattern {
                0 => "\n",
                1 => "\r\n",
                2 => "\r",
                _ => ["\r\n", "\r", "\n"][n % 3],
            });
        } else {
            result.push_str(line);
        }
    }
    result
}

#[test]
fn generated_native_bodies_merge_or_explain_the_limitation_before_the_check() {
    let mut cases = Vec::new();
    let indents = [
        "", " ", "  ", "   ", "    ", "     ", "      ", "\t", " \t", "\t ", "\x0c",
    ];
    for (i, first) in indents.iter().enumerate() {
        for (j, later) in indents.iter().enumerate() {
            for survivor in [false, true] {
                let pattern = (i * 3 + j + usize::from(survivor)) % 8;
                let body = match pattern {
                    0 => format!("{first}- first [[SOURCE]]\n{later}- second\n{later}    code\n"),
                    1 => format!("{first}- first\n{later}- second\n{later}  ```\n{later}    code\n{later}  ```\n"),
                    2 => format!("{first}- first\n{first}  - child\n{first}    continuation\n{later}- second\n"),
                    3 => format!("{first}- title:: SOURCE\n{first}  - child\n{later}- second\n"),
                    4 => format!("k:: source\n\n{first}- first\n{later}- second\n{later}  k:: v\n"),
                    5 => format!("{first}- first\n\n{later}- second\n{later}  \n{later}  tail\n\n"),
                    6 => format!("{first}- first\n{later}- second\n{later}\t  tail\n"),
                    _ => format!("{first}- first\n{later}- second\n{later}  ```\n{later}    unfinished\n"),
                };
                let simple = "k:: target\n\n- plain [[SOURCE]]\n".to_owned();
                let (source, target) = if survivor {
                    (
                        simple.replace("target", "source"),
                        body.replace("SOURCE", "TARGET"),
                    )
                } else {
                    (body, simple)
                };
                let eol = if (i + j) % 2 == 0 { "\r\n" } else { "\n" };
                let ending = |text: String| {
                    let text = if (i + j) % 3 == 0 {
                        text.trim_end_matches('\n').to_owned()
                    } else {
                        text
                    };
                    text.replace('\n', eol)
                };
                cases.push(Case {
                    source: ending(source),
                    target: ending(target),
                    org: false,
                    rename: (i + j) % 7 == 0,
                    refusal: None,
                });
            }
        }
    }
    for survivor in [false, true] {
        for (n, header) in [
            "- title:: SOURCE\n  k:: source\n  ```\n  text\n  ```\n  - child\n",
            "- title:: SOURCE\n  k:: source\n  note after properties\n  - child\n",
            "- title:: SOURCE\n  k:: source\n  > quote [[SOURCE]]\n  - child\n",
            "- title:: SOURCE\n  k:: source\n  # heading\n  - child\n",
            "- title:: SOURCE\n  k:: source\n  :PROPERTIES:\n  :K: v\n  :END:\n  - child\n",
            "k:: source\n\nprose\n  indented prose\n\n",
            "- title:: SOURCE\n  k:: source\n\n  - child\n",
            "k:: source\nk:: repeated\n",
        ]
        .iter()
        .enumerate()
        {
            let body = format!("{header}- last [[SOURCE]]\n");
            let simple = "k:: target\n\n- plain\n".to_owned();
            let (source, target) = if survivor {
                (simple, body.replace("SOURCE", "TARGET"))
            } else {
                (body, simple)
            };
            cases.push(Case {
                source,
                target,
                org: false,
                rename: n % 2 == 0,
                refusal: None,
            });
        }
        for (n, head) in [
            "#+title: SOURCE\n#+K: source\n",
            ":PROPERTIES:\n:K: source\n:END:\n",
            "#+K: first\n#+K: second\n",
            "#+END_QUOTE\n#+title: SOURCE\n",
            "#+BEGIN_QUOTE\n#+title: SOURCE\n#+END_QUOTE\n",
            "#+K: source\nprose [[SOURCE]]\n",
            "#+K: source\n#+BEGIN_SRC\n#+K: inside\n#+END_SRC\n",
            "#+title: SOURCE\n",
        ]
        .iter()
        .enumerate()
        {
            for crlf in [false, true] {
                let body = format!("{head}* first\n** child\n continuation\n* second\n#+BEGIN_SRC\n code\n#+END_SRC\n");
                let simple = "#+K: target\n* plain [[SOURCE]]\n".to_owned();
                let (source, target) = if survivor {
                    (simple, body.replace("SOURCE", "TARGET"))
                } else {
                    (body, simple)
                };
                let ending = |text: String| {
                    if crlf {
                        text.replace('\n', "\r\n")
                    } else {
                        text.trim_end_matches('\n').to_owned()
                    }
                };
                cases.push(Case {
                    source: ending(source),
                    target: ending(target),
                    org: true,
                    rename: n % 2 == 0,
                    refusal: match (survivor, n) {
                        (false, 1) => Some(("HiddenOccurrence", true)),
                        (false, 3) => Some(("DirectiveEscape", true)),
                        _ => None,
                    },
                });
            }
        }
    }
    for rename in [false, true] {
        for eol in ["\n", "\r\n"] {
            cases.push(Case {
                source: format!("* source{eol}"),
                target: format!("* target{eol}{eol}"),
                org: true,
                rename,
                refusal: None,
            });
        }
    }
    for org in [false, true] {
        for rename in [false, true] {
            for ending in 0..4 {
                for role in 0..3 {
                    for (n, pattern) in [
                        "prose [[SOURCE]]\n \n- block\n",
                        "prose [[SOURCE]]\n\t\n",
                        "prose [[SOURCE]]\n \t \n\n- block\n",
                        "prose [[SOURCE]]\n \n\t\n \t\n",
                        "prose [[SOURCE]]\n \ninside\n\t\n- block\n",
                        "prose [[SOURCE]]",
                    ]
                    .iter()
                    .enumerate()
                    {
                        let mut rich = if org {
                            pattern.replace("- block", "* block")
                        } else {
                            (*pattern).to_owned()
                        };
                        if n % 2 == 1 {
                            rich =
                                format!("{}\n\n{rich}", if org { "#+K: café" } else { "k:: café" });
                        }
                        let plain = if org { "* plain\n" } else { "- plain\n" };
                        cases.push(Case {
                            source: endings(if role == 1 { plain } else { &rich }, ending),
                            target: endings(if role == 0 { plain } else { &rich }, ending),
                            org,
                            rename,
                            refusal: None,
                        });
                    }
                }
                // Reader-owned header lines feed the leftover too, including
                // first-root continuation layout and canonical line endings.
                for (source, target) in [
                    (
                        "- title:: SOURCE\n \u{c}k:: source\n  - child\n",
                        "k:: target\n- target\n",
                    ),
                    (
                        "k:: source\nk:: repeated\nprose\n \n",
                        "k:: target\n- target\n",
                    ),
                ] {
                    let render = |text: &str| {
                        if org {
                            text.replace(
                                "- title:: SOURCE\n \u{c}k:: source",
                                "#+title: SOURCE\n#+K: source",
                            )
                            .replace("k:: source", "#+K: source")
                            .replace("k:: repeated", "#+K: repeated")
                            .replace("k:: target", "#+K: target")
                            .replace("  - child", "* child")
                            .replace("- target", "* target")
                        } else {
                            text.into()
                        }
                    };
                    for survivor in [false, true] {
                        let (source, target) = if survivor {
                            (render(target), render(source).replace("SOURCE", "TARGET"))
                        } else {
                            (render(source), render(target))
                        };
                        cases.push(Case {
                            source: endings(&source, ending),
                            target: endings(&target, ending),
                            org,
                            rename,
                            refusal: None,
                        });
                    }
                }
                if org {
                    cases.push(Case {
                        source: endings("#+K: v\n \n", ending),
                        target: endings("* target\n", ending),
                        org,
                        rename,
                        refusal: None,
                    });
                    cases.push(Case {
                        source: endings("* source\n", ending),
                        target: endings("#+K: v\n \n", ending),
                        org,
                        rename,
                        refusal: None,
                    });
                }
            }
        }
    }
    assert_eq!(cases.len(), 662);
    run(cases);
}

#[test]
fn trailing_blank_lines_merge_in_both_entries() {
    let mut cases = Vec::new();
    for rename in [false, true] {
        for eol in ["\n", "\r\n"] {
            for target in [
                "* target",
                "* target\n body",
                "* target\n:PROPERTIES:\n:K: v\n:END:",
                "#+K: target",
                ":PROPERTIES:\n:K: target\n:END:",
                "#+K: target\n* target\n** child",
            ] {
                for tail in ["\n", "\n\n\n", "\n \n", "\n \n\n"] {
                    for source in ["* source\n", "* source\n\n"] {
                        cases.push(Case {
                            source: source.replace('\n', eol),
                            target: format!("{target}{tail}").replace('\n', eol),
                            org: true,
                            rename,
                            refusal: None,
                        });
                    }
                }
            }
            for target in ["- target\n\n", "- target\n \n"] {
                for source in ["- source\n", "- source\n\n"] {
                    cases.push(Case {
                        source: source.replace('\n', eol),
                        target: target.replace('\n', eol),
                        org: false,
                        rename,
                        refusal: None,
                    });
                }
            }
            cases.push(Case {
                source: "#+S: source\n \n\n".replace('\n', eol),
                target: "* target\n\n".replace('\n', eol),
                org: true,
                rename,
                refusal: None,
            });
            cases.push(Case {
                source: "* source".to_owned(),
                target: "* target\n\n".replace('\n', eol),
                org: true,
                rename,
                refusal: None,
            });
            cases.push(Case {
                source: "* source\r".to_owned(),
                target: "* target\n\n".replace('\n', eol),
                org: true,
                rename,
                refusal: None,
            });
        }
    }
    assert_eq!(cases.len(), 220);
    run(cases);
}
