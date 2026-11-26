//! GH #1012: a date picked on a block without title text is written after an
//! empty first line (`-` then `  SCHEDULED: <d>`), because Logseq reads a
//! block's first line as its title and takes no date from it. Enter and merge
//! on that shape keep the date off the first line.
use tine_outline::outline::{self, Format, KeySource, OutlinePage, Verb};

const S: &str = "SCHEDULED: <2026-10-03 Sat>";
const LB: &str = ":LOGBOOK:\nCLOCK: [2026-10-01 Wed 10:00]\n:END:";

fn page(format: Format, a: &str, b: &str) -> OutlinePage {
    serde_json::from_value(serde_json::json!({
        "name": "P", "format": format, "roots": ["a", "b"],
        "blocks": { "a": { "raw": a }, "b": { "raw": b } }
    }))
    .unwrap()
}

fn run(page: OutlinePage, verb: Verb) -> OutlinePage {
    let change = outline::apply(&[page.clone()], &verb, &KeySource::default()).unwrap();
    let mut page = page;
    change.pages[0].apply_to(&mut page);
    page
}

fn split(format: Format, raw: &str, raw_offset: usize) -> OutlinePage {
    run(
        page(format, raw, "next"),
        Verb::Split {
            key: "a".into(),
            raw_offset,
            force_child: false,
            children_visible: false,
            keep_start_in_scope: false,
        },
    )
}

/// Enter at the start of the date line of `\nSCHEDULED: <d>` inserts an empty
/// block before it and leaves the original's empty first line in place (before
/// this, the split took that line away and left the date on line 0). Enter at
/// offset 0 already kept the block whole.
#[test]
fn enter_at_the_date_line_keeps_the_empty_first_line() {
    for format in [Format::Md, Format::Org] {
        for raw in [format!("\n{S}"), format!("\n{S}\n{LB}")] {
            for offset in [0, 1] {
                let after = split(format, &raw, offset);
                assert_eq!(after.blocks["a"].raw, raw, "{format:?} {raw:?} at {offset}");
                assert_eq!(after.roots.len(), 3, "{raw:?}");
                assert_eq!(after.roots[1], "a", "the new empty block goes before");
                assert_eq!(after.blocks[&after.roots[0]].raw, "");
            }
        }
    }
    // A titled block splits as before.
    let after = split(Format::Md, &format!("x\n{S}"), 1);
    assert_eq!(after.blocks["a"].raw, "x");
    assert_eq!(after.blocks[&after.roots[1]].raw, S);
}

/// Merging the new shape into the block before it joins the texts at the empty
/// first line, so the survivor keeps the date on a line of its own.
#[test]
fn merging_the_new_shape_keeps_the_date_on_its_own_line() {
    for format in [Format::Md, Format::Org] {
        let after = run(
            page(format, "x", &format!("\n{S}")),
            Verb::Merge {
                survivor: "a".into(),
                absorbed: "b".into(),
                absorbed_referenced: false,
            },
        );
        assert_eq!(after.blocks["a"].raw, format!("x\n{S}"), "{format:?}");
        let regions = tine_outline::block_regions::parse(&after.blocks["a"].raw, format.is_org());
        assert!(regions.planning.iter().any(|p| p.kind == "Scheduled"));
    }
}
