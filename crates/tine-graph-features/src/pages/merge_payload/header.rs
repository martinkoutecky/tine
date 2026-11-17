//! Allocation's intended logical lines, checked against the assembled reader.
use super::{Header, MergePart, RealizationError, RealizationFailure, Writer};

#[cfg(test)]
thread_local! {
    static CHECK_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Default)]
pub(super) struct Expected {
    // Full payload plus parser-owned key and native form. Matching the payload
    // and its property classification also pins its value, without reparsing
    // an expected header made from the output under test.
    lines: Vec<(String, Option<(String, bool)>)>,
}

impl Expected {
    /// Record the input reader's payload before placing it. A united alias
    /// line records the field planner's kept tokens and inserted delimiters.
    /// Drawer frames and reader-excluded layout carry no content expectation.
    pub fn emit(
        &mut self,
        writer: &mut Writer,
        file: usize,
        header: &Header,
        n: usize,
        united: bool,
        additions: &[Vec<usize>],
    ) -> Result<(), RealizationFailure> {
        let line = &header.lines[n];
        if header.delimiter(n) {
            return writer.original(file, line.content.clone());
        }
        let raw = if united {
            writer.field(file, line, additions)?
        } else {
            let mut raw = String::new();
            writer.payload(file, line.payload.clone(), &mut raw)?;
            writer.original(file, line.content.clone())?;
            raw
        };
        if !tine_core::text_trace::blank(&raw) {
            self.lines.push((
                raw,
                line.property
                    .as_ref()
                    .map(|(key, _)| (key.clone(), line.drawer.is_none())),
            ));
        }
        Ok(())
    }

    /// Reuse the writer's output document read; only its logical header is
    /// projected. O(header bytes), no outline walk, I/O or model-check call.
    pub fn check(&self, raw: &str, preamble: bool, org: bool) -> Result<(), RealizationFailure> {
        let refused = || RealizationError::BodyLaw.at(MergePart::Output);
        let regions = tine_core::block_regions::parse_document(raw, org);
        // All input pre-block prose was promoted to a checked outline block.
        // Any prose here is a header line displaced into the body (or vice versa).
        if preamble && tine_core::page_properties::preamble_range_in(raw, org, &regions).is_some() {
            return Err(refused());
        }
        let mut properties = regions.page_properties().peekable();
        let boxes = regions.property_drawer_boundaries(raw);
        let (mut drawer, mut at) = (0, 0);
        let mut expected = self.lines.iter();
        for line in raw.split_inclusive('\n') {
            while boxes.get(drawer).is_some_and(|&(_, close)| close < at) {
                drawer += 1;
            }
            let frame = boxes
                .get(drawer)
                .is_some_and(|&(open, close)| at == open || at == close);
            let payload = line.trim_end_matches('\n');
            if !frame && !tine_core::text_trace::blank(payload) {
                let property = properties.next_if(|p| p.line.0 == at);
                let found = property.map(|p| {
                    (
                        tine_core::doc::property_key_norm(&p.key),
                        !org || p.directive,
                    )
                });
                let Some((raw, key)) = expected.next() else {
                    return Err(refused());
                };
                #[cfg(test)]
                CHECK_BYTES.with(|count| count.set(count.get() + payload.len()));
                if raw != payload || key != &found {
                    return Err(refused());
                }
            }
            at += line.len();
        }
        if expected.next().is_some() || properties.next().is_some() {
            return Err(refused());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(
        expected: &Expected,
        doc: &tine_core::doc::Document,
        org: bool,
    ) -> Result<(), RealizationFailure> {
        let header = tine_core::page_header::page_header_text(
            doc.pre_block.as_deref(),
            doc.roots.first().map(|b| b.raw()),
        )
        .unwrap_or("");
        expected.check(header, doc.pre_block.is_some(), org)
    }

    fn fields(lines: &[(&str, &str, bool)]) -> Expected {
        Expected {
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
            check(&expected, &tine_core::doc::parse(raw), false).unwrap();
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
                check(&expected, &tine_core::doc::parse(raw), false),
                Err(RealizationError::BodyLaw.at(MergePart::Output)),
                "{raw:?}"
            );
        }
        let expected = fields(&[(":K: café 雪", "k", false)]);
        check(
            &expected,
            &tine_core::org::parse_org("\n:PROPERTIES:\n:K: café 雪\n:END:\n* body\n"),
            true,
        )
        .unwrap();
        for raw in [":K: café 雪\n* body\n", "#+K: café 雪\n* body\n"] {
            assert!(
                check(&expected, &tine_core::org::parse_org(raw), true).is_err(),
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
                let doc = if org {
                    tine_core::org::parse_org(&text)
                } else {
                    tine_core::doc::parse(&text)
                };
                let expected = Expected { lines: fields };
                CHECK_BYTES.with(|n| n.set(0));
                check(&expected, &doc, org).unwrap();
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
                let check = best(&mut || check(&expected, &doc, org).unwrap());
                eprintln!("{count} header fields, org={org}: {bytes} bytes, check {check:?}, projection {projection:?}");
                assert!(check <= projection * 6 + std::time::Duration::from_millis(50));
            }
        }
    }
}
