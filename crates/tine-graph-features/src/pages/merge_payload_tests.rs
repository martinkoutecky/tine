use super::*;

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
