//! Byte/refusal compatibility of the reader-coordinate redesign.
//! Fingerprints were captured from e70f1466d's exact writer, before redesign,
//! and re-captured after the reader merge (3c1844a19) with its moved outcomes
//! counted; each outcome includes all survivor bytes or the refusal kind and file.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn reader_redesign_preserves_parent_bytes_and_refusal_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let store = tine_store::Store::open(dir.path(), Default::default())
        .unwrap()
        .0;
    let mut digest = Sha256::new();
    let (mut proposals, mut refusals, mut link_text_kept) = (0, 0, 0);
    for org in [false, true] {
        let heads: &[&str] = if org {
            &[
                "",
                "#+K: v\n",
                "#+K: w\n",
                "#+alias: A, B\n",
                "#+aliases: B, C\n",
                ":PROPERTIES:\n:K: v\n:END:\n",
                "#+TITLE: Src\n",
                "prose\n \n",
                "#+K: v\n \n",
                "#+BEGIN_QUOTE\n#+K: v\n#+END_QUOTE\n",
            ]
        } else {
            &[
                "",
                "k:: v\n",
                "k:: w\n",
                "alias:: A, B\n",
                "aliases:: B, C\n",
                "alias:: [x](A.md), B\n",
                "title:: Src\n",
                "prose\n \n",
                "k:: v\n \n",
                "k:: v\n \nb:: y\n",
                "alias:: a **b**\n",
                "alias:: \\#x, a\\[b\n",
                "\n- k:: v\n  aliases:: B, C\n",
            ]
        };
        let bodies: &[&str] = if org {
            &[
                "",
                "* body\n",
                "* parent\n** child\n",
                "* body\n\n",
                "* code\n#+BEGIN_SRC\n* literal\n#+END_SRC\n",
            ]
        } else {
            &[
                "",
                "- body\n",
                "- parent\n  - child\n",
                "  - body\n    continuation\n",
                "- code\n  ```\n  - literal\n  ```\n",
            ]
        };
        for ending in ["\n", "\r\n", "\r", "mixed"] {
            let texts: Vec<_> = heads
                .iter()
                .flat_map(|h| bodies.iter().map(move |b| format!("{h}{b}")))
                .map(|s| {
                    if ending == "mixed" {
                        s.replace('\n', "\r\n").replacen("\r\n", "\n", 1)
                    } else {
                        s.replace('\n', ending)
                    }
                })
                .collect();
            let pages: Vec<_> = texts
                .iter()
                .map(|s| {
                    store
                        .page_of(
                            &tine_store::PageId::from(if org {
                                "pages/Src.org"
                            } else {
                                "pages/Src.md"
                            }),
                            s.as_bytes(),
                        )
                        .unwrap()
                        .doc
                })
                .collect();
            for (i, target) in texts.iter().enumerate() {
                for (j, source) in texts.iter().enumerate() {
                    let result = propose(
                        &pages[i],
                        target,
                        &pages[j],
                        source,
                        &Default::default(),
                        org,
                        store.config().file_name_format,
                    );
                    if let Err(RealizationFailure {
                        part: MergePart::Source,
                        reason: RealizationError::MissingNativeNameRange { declaration },
                    }) = &result
                    {
                        if declaration == "alias:: [x](A.md), B" {
                            link_text_kept += 1;
                        }
                    }
                    let outcome = match result {
                        Ok(p) => {
                            proposals += 1;
                            format!("ok:{:?}", p.text)
                        }
                        Err(e) => {
                            refusals += 1;
                            format!("err:{e:?}")
                        }
                    };
                    digest.update(outcome.as_bytes());
                    digest.update([0]);
                }
            }
        }
    }
    let hash = format!("{:x}", digest.finalize());
    eprintln!("parent compatibility: {proposals} proposals, {refusals} refusals; {hash}");
    // The reader merge (Logseq's reading: a file link declares no alias
    // member) moved exactly the 600 outcomes that combine a source
    // `alias:: [x](A.md), B` (member `B` only) into a survivor declaring
    // members (6 heads x 5 bodies x 5 source bodies x 4 endings). Dropping the
    // non-member `[x](A.md)` would lose text (P4), so they refuse as
    // `MissingNativeNameRange`; before, they were 600 of 22404 proposals.
    assert_eq!(link_text_kept, 600);
    // cm-header-boundary (one owner of where a page header ends, OG's rule)
    // moved, against the hash 56437893…bf5d: 2220 HeaderProseOverlap to
    // proposals (Markdown `k:: v`, blank, `b:: y`: `b:: y` is the prose block
    // now), 228 + 52 HeaderProseOverlap to the BodyLaw / IndentedBody refusals
    // every prose-headed page with an indented body already gets, 600 BodyLaw
    // to proposals and 1300 proposals' bytes (an Org `#+K:` inside a quote is
    // no page property, so the quote stays whole as prose instead of losing
    // its directive to the header). Every moved proposal passes P2 and keeps
    // every non-space character of both inputs.
    assert_eq!(
        (proposals, refusals),
        (22404 - 600 + 2220 + 600, 4496 + 600 - 2220 - 600)
    );
    assert_eq!(
        hash,
        "07eda2c4e1847c474d71e241554bf65d1282098e6126fcea1cf11b210ed980b3"
    );
}

#[test]
fn merge_reads_each_input_and_the_output_outline_once() {
    let dir = tempfile::tempdir().unwrap();
    let store = tine_store::Store::open(dir.path(), Default::default())
        .unwrap()
        .0;
    for org in [false, true] {
        for count in [1, 60] {
            for titled in [false, true] {
                let mut text = if org {
                    format!("#+alias: A\n{}", "* body\n".repeat(count))
                } else {
                    format!("alias:: A\n{}", "- body\n".repeat(count))
                };
                if titled {
                    text = format!(
                        "{}{text}",
                        if org {
                            "#+title: Src\n"
                        } else {
                            "title:: Src\n"
                        }
                    );
                }
                let target = if titled {
                    text.replace("Src", "Dst")
                } else {
                    text.clone()
                };
                let page = store
                    .page_of(
                        &tine_store::PageId::from(if org {
                            "pages/Src.org"
                        } else {
                            "pages/Src.md"
                        }),
                        text.as_bytes(),
                    )
                    .unwrap()
                    .doc;
                let dst = store
                    .page_of(
                        &tine_store::PageId::from(if org {
                            "pages/Dst.org"
                        } else {
                            "pages/Dst.md"
                        }),
                        target.as_bytes(),
                    )
                    .unwrap()
                    .doc;
                let links = if titled {
                    HashMap::from([("src".into(), "Dst".into())])
                } else {
                    HashMap::new()
                };
                tine_core::doc::native::take_outline_reads();
                propose(
                    &dst,
                    &target,
                    &page,
                    &text,
                    &links,
                    org,
                    store.config().file_name_format,
                )
                .unwrap();
                let reads = tine_core::doc::native::take_outline_reads();
                eprintln!("org={org}, titled={titled}, {count} roots: {reads} outline reads");
                assert_eq!(reads, 3, "one read per input and one output");
            }
        }
    }
}

#[test]
fn reader_redesign_preserves_drawer_context_and_mapped_titles() {
    let dir = tempfile::tempdir().unwrap();
    let store = tine_store::Store::open(dir.path(), Default::default())
        .unwrap()
        .0;
    let mut digest = Sha256::new();
    let mut cases = 0;
    for blank in ["", "\n", " \n", "\u{c}\n", " \n\t\n", "\n \n\u{c}\n"] {
        for drawer in [false, true] {
            let rich = if drawer {
                format!("{blank}:PROPERTIES:\n:TITLE: SOURCE\n:K: café 雪\n:ALIAS: One, [[Two Words]], one\n:END:\n{blank}:PROPERTIES:\n:K: repeated\n:ALIASES: Three\n:TAGS: Red, [[Blue Sky]]\n:END:\n")
            } else {
                format!("{blank}#+title: SOURCE\n#+K: café 雪\n{blank}#+K: repeated\n#+alias: One, [[Two Words]], one\n#+aliases: Three\n#+tags: Red, [[Blue Sky]]\n")
            };
            for tail in ["", "* body\n** child\n", "body café 雪\n* body\n"] {
                for ending in ["\n", "\r\n", "\r", "mixed"] {
                    for role in 0..3 {
                        for rename in [false, true] {
                            let rich = format!("{rich}{tail}");
                            let plain = "#+S: plain\n* plain\n";
                            let render = |s: &str| {
                                let s = s.replace("SOURCE", "Src").replace("TARGET", "Dst");
                                if ending == "mixed" {
                                    s.replace('\n', "\r\n").replacen("\r\n", "\n", 1)
                                } else {
                                    s.replace('\n', ending)
                                }
                            };
                            let source = render(if role == 1 { plain } else { &rich });
                            let target = render(&if role == 0 {
                                plain.into()
                            } else {
                                rich.replace("SOURCE", "TARGET")
                            });
                            let src = store
                                .page_of(
                                    &tine_store::PageId::from("pages/Src.org"),
                                    source.as_bytes(),
                                )
                                .unwrap()
                                .doc;
                            let dst = store
                                .page_of(
                                    &tine_store::PageId::from("pages/Dst.org"),
                                    target.as_bytes(),
                                )
                                .unwrap()
                                .doc;
                            let links = if rename {
                                HashMap::from([("src".into(), "Dst".into())])
                            } else {
                                HashMap::new()
                            };
                            let outcome = match propose(
                                &dst,
                                &target,
                                &src,
                                &source,
                                &links,
                                true,
                                store.config().file_name_format,
                            ) {
                                Ok(p) => format!("ok:{:?}", p.text),
                                Err(e) => format!("err:{e:?}"),
                            };
                            digest.update(outcome.as_bytes());
                            digest.update([0]);
                            cases += 1;
                        }
                    }
                }
            }
        }
    }
    let hash = format!("{:x}", digest.finalize());
    eprintln!("extended {cases}: {hash}");
    assert_eq!(
        hash,
        "00fe80612e7ac9ac6add7d0ff3b8a7f213822b72702ec60ea4ecf1b3b48f3da0"
    );
    assert_eq!(cases, 864);
}

/// SPEC O6, item 5 of the source-map lane: an identical clashing directive in
/// both files merges, the source's line kept as the leftover quote. Its
/// references stay references there (the model check observes both links in
/// the source entry and in the quote), so the old writer's `HiddenOccurrence`
/// refusal, which read the entry outside its context, was an over-refusal.
/// The 36 outcomes of the review's 10,260-pair generator, with exact bytes.
#[test]
fn identical_clashing_directive_merges_as_a_leftover_quote() {
    use tine_core::{cm_check, model::Format};
    let cases = [
        ("#+id: [[One]], [[Two Words]]\n", "#+id: [[One]], [[Two Words]]\n*\n#+BEGIN_QUOTE\n#+id: [[One]], [[Two Words]]\n#+END_QUOTE\n"),
        ("#+id: [[One]], [[Two Words]]\r\n", "#+id: [[One]], [[Two Words]]\r\n*\r\n#+BEGIN_QUOTE\r\n#+id: [[One]], [[Two Words]]\r\n#+END_QUOTE\r\n"),
        ("#+id: [[One]], [[Two Words]]\r", "#+id: [[One]], [[Two Words]]\r*\r#+BEGIN_QUOTE\r#+id: [[One]], [[Two Words]]\r#+END_QUOTE\r"),
        ("#+id: [[One]], [[Two Words]]\r\n", "#+id: [[One]], [[Two Words]]\r\n*\r\n#+BEGIN_QUOTE\r\n#+id: [[One]], [[Two Words]]\r\n#+END_QUOTE\r\n"),
        ("#+id: [[One]], [[Two Words]]\n#+k: again\n* body\n", "#+id: [[One]], [[Two Words]]\n#+k: again\n* body\n*\n#+BEGIN_QUOTE\n#+id: [[One]], [[Two Words]]\n#+k: again\n#+END_QUOTE\n* body\n"),
        ("#+id: [[One]], [[Two Words]]\r\n#+k: again\r\n* body\r\n", "#+id: [[One]], [[Two Words]]\r\n#+k: again\r\n* body\r\n*\r\n#+BEGIN_QUOTE\r\n#+id: [[One]], [[Two Words]]\r\n#+k: again\r\n#+END_QUOTE\r\n* body\r\n"),
        ("#+id: [[One]], [[Two Words]]\r#+k: again\r* body\r", "#+id: [[One]], [[Two Words]]\r#+k: again\r* body\r*\r#+BEGIN_QUOTE\r#+id: [[One]], [[Two Words]]\r#+k: again\r#+END_QUOTE\r* body\r"),
        ("#+id: [[One]], [[Two Words]]\r\n#+k: again\r* body\n", "#+id: [[One]], [[Two Words]]\r\n#+k: again\r* body\n*\r\n#+BEGIN_QUOTE\r\n#+id: [[One]], [[Two Words]]\r\n#+k: again\r#+END_QUOTE\r\n* body\n"),
        ("#+id: [[One]], [[Two Words]]\nprose [[A]]\n* body\n", "#+id: [[One]], [[Two Words]]\n* prose [[A]]\n* body\n*\n#+BEGIN_QUOTE\n#+id: [[One]], [[Two Words]]\n#+END_QUOTE\n* prose [[A]]\n* body\n"),
        ("#+id: [[One]], [[Two Words]]\r\nprose [[A]]\r\n* body\r\n", "#+id: [[One]], [[Two Words]]\r\n* prose [[A]]\r\n* body\r\n*\r\n#+BEGIN_QUOTE\r\n#+id: [[One]], [[Two Words]]\r\n#+END_QUOTE\r\n* prose [[A]]\r\n* body\r\n"),
        ("#+id: [[One]], [[Two Words]]\rprose [[A]]\r* body\r", "#+id: [[One]], [[Two Words]]\r* prose [[A]]\r* body\r*\r#+BEGIN_QUOTE\r#+id: [[One]], [[Two Words]]\r#+END_QUOTE\r* prose [[A]]\r* body\r"),
        ("#+id: [[One]], [[Two Words]]\r\nprose [[A]]\r* body\n", "#+id: [[One]], [[Two Words]]\r\n* prose [[A]]\r* body\n*\r\n#+BEGIN_QUOTE\r\n#+id: [[One]], [[Two Words]]\r\n#+END_QUOTE\r\n* prose [[A]]\r* body\n"),
        ("#+id: #[[Two Words]]\n", "#+id: #[[Two Words]]\n*\n#+BEGIN_QUOTE\n#+id: #[[Two Words]]\n#+END_QUOTE\n"),
        ("#+id: #[[Two Words]]\r\n", "#+id: #[[Two Words]]\r\n*\r\n#+BEGIN_QUOTE\r\n#+id: #[[Two Words]]\r\n#+END_QUOTE\r\n"),
        ("#+id: #[[Two Words]]\r", "#+id: #[[Two Words]]\r*\r#+BEGIN_QUOTE\r#+id: #[[Two Words]]\r#+END_QUOTE\r"),
        ("#+id: #[[Two Words]]\r\n", "#+id: #[[Two Words]]\r\n*\r\n#+BEGIN_QUOTE\r\n#+id: #[[Two Words]]\r\n#+END_QUOTE\r\n"),
        ("#+id: #[[Two Words]]\n#+k: again\n* body\n", "#+id: #[[Two Words]]\n#+k: again\n* body\n*\n#+BEGIN_QUOTE\n#+id: #[[Two Words]]\n#+k: again\n#+END_QUOTE\n* body\n"),
        ("#+id: #[[Two Words]]\r\n#+k: again\r\n* body\r\n", "#+id: #[[Two Words]]\r\n#+k: again\r\n* body\r\n*\r\n#+BEGIN_QUOTE\r\n#+id: #[[Two Words]]\r\n#+k: again\r\n#+END_QUOTE\r\n* body\r\n"),
        ("#+id: #[[Two Words]]\r#+k: again\r* body\r", "#+id: #[[Two Words]]\r#+k: again\r* body\r*\r#+BEGIN_QUOTE\r#+id: #[[Two Words]]\r#+k: again\r#+END_QUOTE\r* body\r"),
        ("#+id: #[[Two Words]]\r\n#+k: again\r* body\n", "#+id: #[[Two Words]]\r\n#+k: again\r* body\n*\r\n#+BEGIN_QUOTE\r\n#+id: #[[Two Words]]\r\n#+k: again\r#+END_QUOTE\r\n* body\n"),
        ("#+id: #[[Two Words]]\nprose [[A]]\n* body\n", "#+id: #[[Two Words]]\n* prose [[A]]\n* body\n*\n#+BEGIN_QUOTE\n#+id: #[[Two Words]]\n#+END_QUOTE\n* prose [[A]]\n* body\n"),
        ("#+id: #[[Two Words]]\r\nprose [[A]]\r\n* body\r\n", "#+id: #[[Two Words]]\r\n* prose [[A]]\r\n* body\r\n*\r\n#+BEGIN_QUOTE\r\n#+id: #[[Two Words]]\r\n#+END_QUOTE\r\n* prose [[A]]\r\n* body\r\n"),
        ("#+id: #[[Two Words]]\rprose [[A]]\r* body\r", "#+id: #[[Two Words]]\r* prose [[A]]\r* body\r*\r#+BEGIN_QUOTE\r#+id: #[[Two Words]]\r#+END_QUOTE\r* prose [[A]]\r* body\r"),
        ("#+id: #[[Two Words]]\r\nprose [[A]]\r* body\n", "#+id: #[[Two Words]]\r\n* prose [[A]]\r* body\n*\r\n#+BEGIN_QUOTE\r\n#+id: #[[Two Words]]\r\n#+END_QUOTE\r\n* prose [[A]]\r* body\n"),
        ("#+id: [[A]] and [[B]]\n", "#+id: [[A]] and [[B]]\n*\n#+BEGIN_QUOTE\n#+id: [[A]] and [[B]]\n#+END_QUOTE\n"),
        ("#+id: [[A]] and [[B]]\r\n", "#+id: [[A]] and [[B]]\r\n*\r\n#+BEGIN_QUOTE\r\n#+id: [[A]] and [[B]]\r\n#+END_QUOTE\r\n"),
        ("#+id: [[A]] and [[B]]\r", "#+id: [[A]] and [[B]]\r*\r#+BEGIN_QUOTE\r#+id: [[A]] and [[B]]\r#+END_QUOTE\r"),
        ("#+id: [[A]] and [[B]]\r\n", "#+id: [[A]] and [[B]]\r\n*\r\n#+BEGIN_QUOTE\r\n#+id: [[A]] and [[B]]\r\n#+END_QUOTE\r\n"),
        ("#+id: [[A]] and [[B]]\n#+k: again\n* body\n", "#+id: [[A]] and [[B]]\n#+k: again\n* body\n*\n#+BEGIN_QUOTE\n#+id: [[A]] and [[B]]\n#+k: again\n#+END_QUOTE\n* body\n"),
        ("#+id: [[A]] and [[B]]\r\n#+k: again\r\n* body\r\n", "#+id: [[A]] and [[B]]\r\n#+k: again\r\n* body\r\n*\r\n#+BEGIN_QUOTE\r\n#+id: [[A]] and [[B]]\r\n#+k: again\r\n#+END_QUOTE\r\n* body\r\n"),
        ("#+id: [[A]] and [[B]]\r#+k: again\r* body\r", "#+id: [[A]] and [[B]]\r#+k: again\r* body\r*\r#+BEGIN_QUOTE\r#+id: [[A]] and [[B]]\r#+k: again\r#+END_QUOTE\r* body\r"),
        ("#+id: [[A]] and [[B]]\r\n#+k: again\r* body\n", "#+id: [[A]] and [[B]]\r\n#+k: again\r* body\n*\r\n#+BEGIN_QUOTE\r\n#+id: [[A]] and [[B]]\r\n#+k: again\r#+END_QUOTE\r\n* body\n"),
        ("#+id: [[A]] and [[B]]\nprose [[A]]\n* body\n", "#+id: [[A]] and [[B]]\n* prose [[A]]\n* body\n*\n#+BEGIN_QUOTE\n#+id: [[A]] and [[B]]\n#+END_QUOTE\n* prose [[A]]\n* body\n"),
        ("#+id: [[A]] and [[B]]\r\nprose [[A]]\r\n* body\r\n", "#+id: [[A]] and [[B]]\r\n* prose [[A]]\r\n* body\r\n*\r\n#+BEGIN_QUOTE\r\n#+id: [[A]] and [[B]]\r\n#+END_QUOTE\r\n* prose [[A]]\r\n* body\r\n"),
        ("#+id: [[A]] and [[B]]\rprose [[A]]\r* body\r", "#+id: [[A]] and [[B]]\r* prose [[A]]\r* body\r*\r#+BEGIN_QUOTE\r#+id: [[A]] and [[B]]\r#+END_QUOTE\r* prose [[A]]\r* body\r"),
        ("#+id: [[A]] and [[B]]\r\nprose [[A]]\r* body\n", "#+id: [[A]] and [[B]]\r\n* prose [[A]]\r* body\n*\r\n#+BEGIN_QUOTE\r\n#+id: [[A]] and [[B]]\r\n#+END_QUOTE\r\n* prose [[A]]\r* body\n"),
    ];
    let dir = tempfile::tempdir().unwrap();
    let store = tine_store::Store::open(dir.path(), Default::default())
        .unwrap()
        .0;
    let names = store.config().file_name_format;
    for (text, merged) in cases {
        let page = |path: &str| {
            store
                .page_of(&tine_store::PageId::from(path), text.as_bytes())
                .unwrap()
                .doc
        };
        let proposal = propose(
            &page("pages/Dst.org"),
            text,
            &page("pages/Src.org"),
            text,
            &Default::default(),
            true,
            names,
        )
        .unwrap_or_else(|e| panic!("{text:?}: {e:?}"));
        assert_eq!(proposal.text, merged, "{text:?}");
        let observe = |t: &str| cm_check::observe(t, Format::Org, names);
        let expected = cm_check::expected(
            &cm_check::Rename::none(None, names),
            Format::Org,
            cm_check::Expect::Survivor {
                source: &observe(text),
                target: &observe(text),
            },
        );
        assert!(
            cm_check::matches(&expected, &observe(merged), true),
            "{text:?}"
        );
    }
}

/// A survivor header line holding only a no-break space is content to the
/// reader (mldoc's spacing is ` `, tab, form feed, SUB): joined fields go
/// after it, as after any other header line. The writer used to place them by
/// Unicode `trim`, before it, and its own acceptance then refused the merge.
#[test]
fn a_no_break_space_header_line_is_content_for_insertion() {
    use tine_core::{cm_check, model::Format};
    let dir = tempfile::tempdir().unwrap();
    let store = tine_store::Store::open(dir.path(), Default::default())
        .unwrap()
        .0;
    let names = store.config().file_name_format;
    for eol in ["\n", "\r\n", "\r"] {
        let target = format!("k:: v{eol}\u{a0}{eol}- body{eol}");
        let source = format!("- plain{eol}");
        let page = |path: &str, text: &str| {
            store
                .page_of(&tine_store::PageId::from(path), text.as_bytes())
                .unwrap()
                .doc
        };
        let proposal = propose(
            &page("pages/Dst.md", &target),
            &target,
            &page("pages/Src.md", &source),
            &source,
            &Default::default(),
            false,
            names,
        )
        .unwrap();
        assert_eq!(proposal.text, format!("{target}{source}"));
        let observe = |t: &str| cm_check::observe(t, Format::Md, names);
        let expected = cm_check::expected(
            &cm_check::Rename::none(None, names),
            Format::Md,
            cm_check::Expect::Survivor {
                source: &observe(&source),
                target: &observe(&target),
            },
        );
        assert!(cm_check::matches(&expected, &observe(&proposal.text), true));
    }
}
