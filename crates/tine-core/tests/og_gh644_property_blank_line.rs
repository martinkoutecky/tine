//! REG-OG-GH644: lsdoc folds a blank line into a property node when a
//! directive follows it (`a:: 1\n\n#+b: 2`; mldoc drawer.ml folds trailing
//! directives). The region walker trimmed that blank line to an inverted
//! range and sliced it, so `title`/projection/reference reads panicked
//! ("byte range starts at N+1 but ends at N"). In the app that page stopped
//! the initial load and every page lookup: no link or tag opened and `[[`
//! offered nothing. Invariants: I-2 (one bad file must not refuse the graph),
//! I-12 (the region answer is lsdoc's). Exemplar: `block_regions::parse`.

use tine_core::block_regions::{parse, parse_document};
use tine_core::model::{page_title_from_preamble, preamble_read, Format, PreambleRead};

/// Every shape lsdoc folds across a blank or whitespace-only line, in both
/// formats, with LF, CRLF and a lone CR, as a block and as a page preamble.
const SHAPES: &[(&str, bool)] = &[
    ("title:: X\n\n#+description: y\n", false),
    ("alias:: a\n\n#+TITLE: b\n\n- x\n", false),
    ("public:: true\n\n#+include: x\n", false),
    ("title:: X\r\n\r\n#+description: y\r\n", false),
    ("title:: X\n \n#+description: y\n", false),
    ("title:: X\n\t\n#+description: y\n", false),
    ("day\nfoo:: x\n\n#+title: y", false),
    (":PROPERTIES:\n:id: x\n:END:\n\n#+title: y\n", true),
    (
        ":PROPERTIES:\r\n:id: x\r\n:END:\r\n\r\n#+title: y\r\n",
        true,
    ),
    ("#+BEGIN_SRC:\n\n- ", true),
];

fn well_formed(raw: &str, r: &tine_core::block_regions::Range) -> bool {
    r.0 <= r.1 && r.1 <= raw.len() && raw.is_char_boundary(r.0) && raw.is_char_boundary(r.1)
}

#[test]
fn a_blank_line_inside_a_property_node_is_skipped_not_sliced_backwards() {
    for &(raw, org) in SHAPES {
        for regions in [
            std::panic::catch_unwind(|| parse(raw, org)),
            std::panic::catch_unwind(|| parse_document(raw, org)),
        ] {
            let regions = regions.unwrap_or_else(|_| panic!("regions panicked on {raw:?}"));
            for p in &regions.properties {
                assert!(
                    well_formed(raw, &p.value_range) && well_formed(raw, &p.key_range),
                    "{raw:?}: {p:?}"
                );
                // The value span is the parser's value, not a neighbour's bytes.
                if !p.directive {
                    assert_eq!(p.value_range.slice(raw), p.value, "{raw:?}: {p:?}");
                }
            }
        }
    }
}

#[test]
fn the_page_title_reads_through_a_folded_blank_line() {
    let title = |raw: &str, format| {
        std::panic::catch_unwind(|| page_title_from_preamble(raw, format))
            .unwrap_or_else(|_| panic!("title panicked on {raw:?}"))
    };
    assert_eq!(
        title("title:: X\n\n#+description: y\n", Format::Md).as_deref(),
        Some("X")
    );
    assert_eq!(
        title("title:: X\r\n\r\n#+description: y\r\n", Format::Md).as_deref(),
        Some("X")
    );
    assert_eq!(
        title(":PROPERTIES:\n:id: x\n:END:\n\n#+title: y\n", Format::Org).as_deref(),
        Some("y")
    );
    // The streaming preamble reader agrees with the whole-text answer.
    for &(raw, org) in SHAPES {
        let format = if org { Format::Org } else { Format::Md };
        let whole = title(raw, format);
        let mut prefix = String::new();
        let mut streamed = None;
        for line in raw.split_inclusive('\n') {
            prefix.push_str(line);
            if let PreambleRead::Settled(t) = preamble_read(&prefix, format) {
                streamed = Some(t);
                break;
            }
        }
        if let Some(streamed) = streamed {
            assert_eq!(streamed, whole, "{raw:?}");
        }
    }
}
