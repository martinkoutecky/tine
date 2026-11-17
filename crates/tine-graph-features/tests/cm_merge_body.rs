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
    run_pinned(cases, &|_| None);
}

/// As `run`, and a merged case with a pinned survivor must write exactly it.
fn run_pinned(cases: Vec<Case>, survivor: &dyn Fn(&Case) -> Option<String>) {
    // Each transaction needs only its pair. Bound the fixture graph so the
    // matrix measures the writer rather than repeatedly indexing other cases.
    let failures: usize = cases.chunks(64).map(|c| run_batch(c, survivor)).sum();
    assert_eq!(failures, 0, "unexpected matrix outcomes");
}

fn run_batch(cases: &[Case], survivor: &dyn Fn(&Case) -> Option<String>) -> usize {
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
                _ => failures.push(format!("{}: {error:?}", label())),
            }
            continue;
        }
        if case.refusal.is_some() {
            failures.push(format!("unexpected merge: {}", label()));
        }
        merged += 1;
        assert!(!root.path().join(a).exists(), "{}", label());
        let after = fs::read_to_string(root.path().join(b)).unwrap();
        if let Some(expected) = survivor(case).filter(|expected| *expected != after) {
            failures.push(format!("{}: wrote {after:?}, pinned {expected:?}", label()));
        }
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
    if !failures.is_empty() {
        eprintln!("{} failures:\n{}", failures.len(), failures.join("\n"));
    }
    assert!(merged > 0);
    failures.len()
}

#[test]
fn parent_header_layout_merges_and_saves_through_both_entries() {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/cm-merge-header-parent.json")).unwrap();
    assert_eq!(rows.len(), 16);
    run(rows
        .iter()
        .flat_map(|row| {
            [false, true].map(|rename| Case {
                source: row["source"].as_str().unwrap().into(),
                target: row["target"]
                    .as_str()
                    .unwrap()
                    .replace("title:: dst", "title:: TARGET"),
                org: false,
                rename,
                refusal: None,
            })
        })
        .collect());
}

#[test]
fn reviewed_eighty_eight_alias_links_merge_through_both_entries() {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/cm-merge-header-links.json")).unwrap();
    assert_eq!(rows.len(), 88);
    let cases = rows
        .iter()
        .flat_map(|row| {
            [false, true].map(|rename| Case {
                source: row["source"].as_str().unwrap().into(),
                target: row["target"].as_str().unwrap().into(),
                org: false,
                rename,
                refusal: row["refusal"].as_str().map(|reason| {
                    assert_eq!(reason, "HeaderChildren");
                    ("HeaderChildren", true)
                }),
            })
        })
        .collect();
    // A file link declares no alias member (Logseq's reading), so a first-bullet
    // `alias:: [label](../pages/A.md)` is other header text: O6 makes it the
    // leftover line, and the header's child hangs under that leftover block.
    let pinned: std::collections::HashMap<_, _> = rows
        .iter()
        .filter_map(|row| {
            let survivor = row.get("survivor")?.as_str()?;
            Some((
                (
                    row["source"].as_str().unwrap(),
                    row["target"].as_str().unwrap(),
                ),
                survivor.to_owned(),
            ))
        })
        .collect();
    assert_eq!(pinned.len(), 16);
    run_pinned(cases, &|case| {
        pinned
            .get(&(case.source.as_str(), case.target.as_str()))
            .cloned()
    });
}

#[test]
fn native_alias_member_siblings_have_pinned_outcomes() {
    let mut cases = Vec::new();
    for org in [false, true] {
        for rename in [false, true] {
            for ending in 0..4 {
                for key in ["alias", "aliases"] {
                    for value in [
                        "[[One, Two]]",
                        "#[[One, Two]]",
                        "One, \"literal\"",
                        "\"One, Two\"",
                        "C\\#, C#",
                        "[[C\\#]]",
                        "\\#x, a\\[b",
                        "a\\[\\[b",
                        "a\\]\\]b",
                        "[**x**_y_](A.md)",
                        "{{embed x}}",
                        "#a\\_b",
                    ] {
                        let rich = if org {
                            format!("#+{key}: {value}\n")
                        } else {
                            format!("{key}:: {value}\n")
                        };
                        let plain = if org { "* plain\n" } else { "- plain\n" };
                        for role in 0..3 {
                            cases.push(Case {
                                source: endings(if role == 1 { plain } else { &rich }, ending),
                                target: endings(if role == 0 { plain } else { &rich }, ending),
                                org, rename,
                                // A Markdown tag with an escape declares the
                                // member `a_b`, which no contiguous native
                                // run spells: the writer refuses rather than
                                // guess its coordinates.
                                refusal: if !org && value == "#a\\_b" {
                                    Some((if key == "alias" {
                                        "MissingNativeNameRange { declaration: \"alias:: #a\\\\_b\" }"
                                    } else {
                                        "MissingNativeNameRange { declaration: \"aliases:: #a\\\\_b\" }"
                                    }, role == 0))
                                } else if org && role == 2 && value == "[**x**_y_](A.md)" {
                                    Some((if key == "alias" { "MissingNativeNameRange { declaration: \"#+alias: [**x**_y_](A.md)\" }" } else { "MissingNativeNameRange { declaration: \"#+aliases: [**x**_y_](A.md)\" }" }, true))
                                } else { None },
                            });
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases.len(), 1152);
    // A Markdown file link and an embed (macros skipped) declare no alias
    // member (Logseq's reading): the survivor keeps its own declaration as
    // header text, and the source's is other header text, so O6 writes it as
    // the leftover block's line (an Org leftover quotes it).
    run_pinned(cases, &|case| {
        let value = ["[**x**_y_](A.md)", "{{embed x}}"]
            .into_iter()
            .filter(|v| !case.org || *v == "{{embed x}}")
            .find(|v| case.source.contains(v) || case.target.contains(v))?;
        let line = |text: &str| text.split(['\r', '\n']).next().unwrap().to_owned();
        let e = ["\r\n", "\n", "\r"]
            .into_iter()
            .find(|e| case.source.ends_with(e))
            .unwrap();
        let leftover = |text: &str| {
            if case.org {
                format!("*{e}#+BEGIN_QUOTE{e}{}{e}#+END_QUOTE{e}", line(text))
            } else {
                format!("-{e}  {}{e}", line(text))
            }
        };
        let plain = if case.org { "* plain" } else { "- plain" };
        Some(if !case.target.contains(value) {
            format!("{plain}{e}{}", leftover(&case.source))
        } else if !case.source.contains(value) {
            format!("{}{e}{plain}{e}", line(&case.target))
        } else {
            format!("{}{e}{}", line(&case.target), leftover(&case.source))
        })
    });
}

#[test]
fn review_alias_classification_uses_the_reader() {
    let mut cases = Vec::new();
    for rename in [false, true] {
        for ending in 0..4 {
            for value in ["[x](A.md)", "[x](a)", "[x](file:A.md)"] {
                for key in ["alias", "aliases"] {
                    for role in 0..3 {
                        let rich = format!("{key}:: {value}\n");
                        cases.push(Case {
                            source: endings(if role == 1 { "- source\n" } else { &rich }, ending),
                            target: endings(if role == 0 { "- target\n" } else { &rich }, ending),
                            org: false,
                            rename,
                            refusal: None,
                        });
                    }
                }
            }
        }
    }
    assert_eq!(cases.len(), 144);
    run(cases);
}

#[test]
fn headers_with_blank_gaps_merge_with_ogs_page_properties() {
    // These pages once refused `HeaderProseOverlap`: two readers disagreed on
    // where the header ends. One owner now answers OG's rule (the first
    // property node), so a later node is prose and both merge.
    let mut cases = Vec::new();
    for rename in [false, true] {
        for ending in 0..4 {
            for rich in ["\na::", "a:: x\n \nb:: y\n"] {
                for source in [false, true] {
                    cases.push(Case {
                        source: endings(if source { rich } else { "- source\n" }, ending),
                        target: endings(if source { "- target\n" } else { rich }, ending),
                        org: false,
                        rename,
                        refusal: None,
                    });
                }
            }
        }
    }
    // Text before the first node makes the whole pre-block opening text, and a
    // folded `#+` line ends the page properties: the merge keeps that prose as
    // a block (REG-CM-HEADER-TEXT-HIDDEN).
    for source in ["text [[P]]\na:: x\n- body\n", "a:: x\n#+b: y\n- body\n"] {
        cases.push(Case {
            source: source.into(),
            target: "- target\n".into(),
            org: false,
            rename: false,
            refusal: None,
        });
    }
    assert_eq!(cases.len(), 34);
    run_pinned(cases, &|case| {
        let pinned = match (case.source.as_str(), case.target.as_str()) {
            // The second node is the preamble's prose, kept as a block.
            ("a:: x\n \nb:: y\n", "- target\n") => "a:: x\n- target\n-  \n  b:: y\n",
            ("- source\n", "a:: x\n \nb:: y\n") => "a:: x\n-  \n  b:: y\n- source\n",
            // A leading blank line is not header text: the field joins.
            ("\na::", "- target\n") => "a::\n- target\n",
            ("- source\n", "\na::") => "\na::\n- source\n",
            ("text [[P]]\na:: x\n- body\n", _) => "- target\n- text [[P]]\n  a:: x\n- body\n",
            ("a:: x\n#+b: y\n- body\n", _) => "a:: x\n- target\n- #+b: y\n- body\n",
            _ => return None,
        };
        Some(pinned.to_owned())
    });
}

#[test]
fn opening_text_between_directives_and_directive_lines_merge_whole() {
    // Org text between opening directives is prose, with page properties on
    // both sides of it (REG-CM-ORG-TEXT-BETWEEN-DIRECTIVES); a Markdown `#+`
    // line's link is a reference (REG-CM-MD-DIRECTIVE-LINK); a Markdown page
    // with no blocks is Markdown (REG-CM-MD-PAGE-READ-AS-ORG). Every byte
    // survives the merge, in both entries and both directions (a page whose
    // opening text is promoted to a block when it is the survivor, as in
    // Markdown). The title row is checked, not pinned: its names vary.
    let pages = [
        ("#+ALIAS: y\n[[P]] text\n#+b: z\n* body\n", true),
        ("#+TITLE: NAME\n[[P]]\n#+ALIAS: y\n* body\n", true),
        ("#+b: z\n[[P]]\n#+c: w\nmore [[Q]]\n* body\n", true),
        ("k:: x\n#+b: [[P]]\n- body\n", false),
        ("#+b: [[P]]\n- body\n", false),
        ("[[P]]\n#+b: y", false),
    ];
    let mut cases = Vec::new();
    for (page, org) in pages {
        let plain = if org { "* other\n" } else { "- other\n" };
        for rename in [false, true] {
            for source in [false, true] {
                // A title names its own file's page.
                let page = page.replace("NAME", if source { "SOURCE" } else { "TARGET" });
                cases.push(Case {
                    source: if source { page.clone() } else { plain.into() },
                    target: if source { plain.into() } else { page },
                    org,
                    rename,
                    refusal: None,
                });
            }
        }
    }
    // Org text before a directive is the page's opening text: it merges as a
    // block, the source's directive as a leftover header line.
    for source in [
        "free text\n\n#+k: s\n* source\n",
        "#+k: s\nfree [[X]]\n#+j: r\n* source\n",
    ] {
        cases.push(Case {
            source: source.into(),
            target: "#+k: t\n* target\n".into(),
            org: true,
            rename: false,
            refusal: None,
        });
    }
    assert_eq!(cases.len(), 26);
    run_pinned(cases, &|case| {
        let pinned = match (case.source.as_str(), case.target.as_str()) {
            ("free text\n\n#+k: s\n* source\n", _) => {
                "#+k: t\n* target\n*\n#+BEGIN_QUOTE\n#+k: s\n#+END_QUOTE\n* free text\n* source\n"
            }
            // The directive after the text is a page property: it joins.
            ("#+k: s\nfree [[X]]\n#+j: r\n* source\n", _) => {
                "#+k: t\n#+j: r\n* target\n*\n#+BEGIN_QUOTE\n#+k: s\n#+END_QUOTE\n* free [[X]]\n* source\n"
            }
            ("#+ALIAS: y\n[[P]] text\n#+b: z\n* body\n", "* other\n") => {
                "#+ALIAS: y\n#+b: z\n* other\n* [[P]] text\n* body\n"
            }
            ("* other\n", "#+ALIAS: y\n[[P]] text\n#+b: z\n* body\n") => {
                "#+ALIAS: y\n#+b: z\n* [[P]] text\n* body\n* other\n"
            }
            ("#+b: z\n[[P]]\n#+c: w\nmore [[Q]]\n* body\n", "* other\n") => {
                "#+b: z\n* other\n* [[P]]\n#+c: w\nmore [[Q]]\n* body\n"
            }
            ("* other\n", "#+b: z\n[[P]]\n#+c: w\nmore [[Q]]\n* body\n") => {
                "#+b: z\n* [[P]]\n#+c: w\nmore [[Q]]\n* body\n* other\n"
            }
            ("k:: x\n#+b: [[P]]\n- body\n", "- other\n") => "k:: x\n- other\n- #+b: [[P]]\n- body\n",
            ("- other\n", "k:: x\n#+b: [[P]]\n- body\n") => "k:: x\n- #+b: [[P]]\n- body\n- other\n",
            ("#+b: [[P]]\n- body\n", "- other\n") => "- other\n- #+b: [[P]]\n- body\n",
            ("- other\n", "#+b: [[P]]\n- body\n") => "- #+b: [[P]]\n- body\n- other\n",
            ("[[P]]\n#+b: y", "- other\n") => "- other\n- [[P]]\n  #+b: y\n",
            ("- other\n", "[[P]]\n#+b: y") => "- [[P]]\n  #+b: y\n- other\n",
            _ => return None,
        };
        Some(pinned.to_owned())
    });
}

#[test]
fn review_header_layout_merges_in_both_entries() {
    let mut cases = Vec::new();
    for rename in [false, true] {
        for ending in 0..4 {
            cases.push(Case {
                source: endings("- plain\n", ending),
                target: endings("\n- title:: TARGET\n \u{c}k:: café 雪\n  - child\n", ending),
                org: false,
                rename,
                refusal: None,
            });
        }
    }
    assert_eq!(cases.len(), 8);
    run(cases);
}

#[test]
fn generated_headers_merge_or_explain_the_limitation_before_the_check() {
    let mut cases = Vec::new();
    for org in [false, true] {
        for rename in [false, true] {
            for ending in 0..4 {
                for (lead, blank) in ["", "\n", " \n", "\u{c}\n", " \n\t\n", "\n \n\u{c}\n"]
                    .iter()
                    .enumerate()
                {
                    for (variant, indent) in
                        ["  ", "\t", "\u{c} ", " \u{c}", " \t"].iter().enumerate()
                    {
                        let header = if org {
                            if variant % 2 == 0 {
                                format!("{blank}#+title: SOURCE\n#+K: café 雪\n{blank}#+K: repeated\n#+alias: One, [[Two Words]], one\n#+aliases: Three\n#+tags: Red, [[Blue Sky]]\n")
                            } else {
                                format!("{blank}:PROPERTIES:\n:TITLE: SOURCE\n:K: café 雪\n:ALIAS: One, [[Two Words]], one\n:END:\n{blank}:PROPERTIES:\n:K: repeated\n:ALIASES: Three\n:TAGS: Red, [[Blue Sky]]\n:END:\n")
                            }
                        } else if variant % 2 == 0 {
                            format!("{blank}- title:: SOURCE\n{indent}k:: café 雪\n{indent}k:: repeated\n{indent}alias:: One, [[Two Words]], one\n{indent}aliases:: Three\n{indent}tags:: Red, [[Blue Sky]]\n")
                        } else {
                            format!("{blank}title:: SOURCE\nk:: café 雪\n{blank}k:: repeated\nalias:: One, [[Two Words]], one\naliases:: Three\ntags:: Red, [[Blue Sky]]\n")
                        };
                        let tail = match (lead + variant) % 3 {
                            0 => "",
                            1 if org => "* body\n** child\n",
                            1 if variant % 2 == 0 => "  - child\n",
                            1 => "- body\n  - child\n",
                            _ if org => "body café 雪\n* body\n",
                            _ if variant % 2 == 0 => "- body café 雪\n",
                            _ => "body café 雪\n- body\n",
                        };
                        let rich = header + tail;
                        let plain = if org {
                            "#+S: plain\n* plain\n"
                        } else {
                            "s:: plain\n- plain\n"
                        };
                        for role in 0..3 {
                            cases.push(Case {
                                source: endings(if role == 1 { plain } else { &rich }, ending),
                                target: endings(
                                    &if role == 0 {
                                        plain.to_owned()
                                    } else {
                                        rich.replace("SOURCE", "TARGET")
                                    },
                                    ending,
                                ),
                                org,
                                rename,
                                refusal: if org
                                    && variant % 2 == 1
                                    && (role == 2
                                        || (role == 0
                                            && [true, false, false, false, true, true][lead]))
                                {
                                    Some(("HiddenOccurrence", true))
                                } else {
                                    None
                                },
                            });
                        }
                    }
                }
            }
        }
    }
    for rename in [false, true] {
        for ending in 0..4 {
            for (lead, blank) in ["", "\n", " \n\u{c}\n", "\u{c}\n \n\t\n"]
                .iter()
                .enumerate()
            {
                for indent in ["\t", " \u{c}", "\u{c}", "\t \u{c}"] {
                    let rich = format!(
                        "{blank}- title:: SOURCE\n{indent}k:: café 雪\n{}",
                        if lead % 2 == 0 { "  - child\n" } else { "" }
                    );
                    for role in 0..3 {
                        cases.push(Case {
                            source: endings(if role == 1 { "- plain\n" } else { &rich }, ending),
                            target: endings(
                                &if role == 0 {
                                    "- plain\n".to_owned()
                                } else {
                                    rich.replace("SOURCE", "TARGET")
                                },
                                ending,
                            ),
                            org: false,
                            rename,
                            refusal: None,
                        });
                    }
                }
            }
        }
    }
    assert_eq!(cases.len(), 1824);
    run(cases);
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
                        // A stray `#+END_QUOTE` before the title is opening
                        // text, now its own block (REG-CM-ORG-TEXT-BETWEEN-DIRECTIVES);
                        // the escape refusal is pinned in merge_text_trace.rs.
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
