use super::*;

fn check(expected: &Intended, read: &Header, text: &str) -> Result<(), RealizationFailure> {
    expected.check_header(read, text)
}

fn fields(lines: &[(&str, &str, bool)]) -> Intended {
    Intended {
        lines: lines
            .iter()
            .map(|(raw, key, directive)| (raw.to_string(), Some((key.to_string(), *directive))))
            .collect(),
    }
}

#[test]
fn header_check_rejects_changed_values_order_classification_and_body_leaks() {
    let expected = fields(&[
        ("k:: café 雪", "k", true),
        ("alias:: One, Two", "alias", true),
    ]);
    for raw in [
        "k:: café 雪\nalias:: One, Two\n- body\n",
        "\n- k:: café 雪\n  alias:: One, Two\n  - child\n",
    ] {
        check(&expected, &Header::read(raw, false), raw).unwrap();
    }
    for raw in [
        "k:: café 雨\nalias:: One, Two\n- body\n",
        "alias:: One, Two\nk:: café 雪\n- body\n",
        "k:: café 雪\nalias:: One, Two\nextra:: body\n- body\n",
        "k:: café 雪\n- alias:: One, Two\n",
        "k:: café 雪\nalias:: One, Two\nbody\n- block\n",
        "- plain\n  k:: café 雪\n  alias:: One, Two\n",
    ] {
        assert_eq!(
            check(&expected, &Header::read(raw, false), raw),
            Err(RealizationError::BodyLaw.at(MergePart::Output)),
            "{raw:?}"
        );
    }
    let expected = fields(&[(":K: café 雪", "k", false)]);
    let raw = "\n:PROPERTIES:\n:K: café 雪\n:END:\n* body\n";
    check(&expected, &Header::read(raw, true), raw).unwrap();
    for raw in [":K: café 雪\n* body\n", "#+K: café 雪\n* body\n"] {
        assert!(
            check(&expected, &Header::read(raw, true), raw).is_err(),
            "{raw:?}"
        );
    }
}

#[test]
fn header_comparison_is_linear_and_bounded_by_its_projection() {
    for org in [false, true] {
        for count in [64, 1024, 8192] {
            let fields: Vec<_> = (0..count)
                .map(|n| {
                    let key = format!("k{n}");
                    let raw = if org {
                        format!("#+{key}: café 雪")
                    } else {
                        format!("{key}:: café 雪")
                    };
                    (raw, Some((key, true)))
                })
                .collect();
            let bytes: usize = fields.iter().map(|(raw, _)| raw.len()).sum();
            let text = fields
                .iter()
                .map(|(raw, _)| raw.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            // Production compares the description from its one output read.
            // As with the former Document argument, build it before timing the
            // acceptance comparison. Full-reader cost has its own merge gates.
            let read = Header::read(&text, org);
            let expected = Intended { lines: fields };
            CHECK_BYTES.with(|n| n.set(0));
            check(&expected, &read, &text).unwrap();
            assert_eq!(CHECK_BYTES.with(|n| n.get()), bytes);
            let best = |f: &mut dyn FnMut()| {
                (0..3)
                    .map(|_| {
                        let start = std::time::Instant::now();
                        f();
                        start.elapsed()
                    })
                    .min()
                    .unwrap()
            };
            let projection = best(&mut || {
                std::hint::black_box(tine_core::block_regions::parse_document(&text, org));
            });
            let check = best(&mut || check(&expected, &read, &text).unwrap());
            eprintln!("{count} header fields, org={org}: {bytes} bytes, check {check:?}, projection {projection:?}");
            assert!(check <= projection * 6 + std::time::Duration::from_millis(50));
        }
    }
}
