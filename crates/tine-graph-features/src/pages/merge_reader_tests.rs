//! Byte/refusal compatibility of the reader-coordinate redesign.
//! Fingerprints were captured from e70f1466d's exact writer, before redesign;
//! each outcome includes all survivor bytes or the refusal kind and file.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn reader_redesign_preserves_parent_bytes_and_refusal_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let store = tine_store::Store::open(dir.path(), Default::default())
        .unwrap()
        .0;
    let mut digest = Sha256::new();
    let (mut proposals, mut refusals) = (0, 0);
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
                        &Default::default(),
                        org,
                        store.config().file_name_format,
                    );
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
    assert_eq!((proposals, refusals), (22404, 4496));
    assert_eq!(
        hash,
        "77d9b34b43b2e04e049f5b57524ad44708ae31204c52b17cf016f80921290d3d"
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
                    &Default::default(),
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
                                &Default::default(),
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
