use std::collections::{HashMap, HashSet};
use tine_core::{
    config::FileNameFormat,
    doc::{DocBlock, Document},
    logbook::{self, LogbookFormat, TimestampParts},
    model::Format,
    pdf::{self, Highlight},
    refs,
};

#[test]
fn clock_out_keeps_literal_drawer() {
    let raw = "Task\n```\n:LOGBOOK:\nCLOCK: [2026-09-29 Tue 09:00]\n:END:\n```";
    let now = TimestampParts {
        year: 2026,
        month: 9,
        day: 29,
        weekday: 2,
        hour: 10,
        minute: 0,
        second: 0,
    };
    assert_eq!(
        logbook::clock_out_at(raw, LogbookFormat::Markdown, false, now),
        raw
    );
    let raw = "Task\n```\nSCHEDULED: <2026-09-29 Tue>\n```";
    let next = logbook::clock_in_at(raw, LogbookFormat::Markdown, false, now);
    assert!(next.contains("```\nSCHEDULED: <2026-09-29 Tue>\n```"));
}

#[test]
fn published_identity_requires_the_block_format() {
    let raw = "Target\n:PROPERTIES:\n:id: published-id\n:END:";
    assert_eq!(
        tine_core::refs::block_id(raw, true).as_deref(),
        Some("published-id")
    );
    assert_eq!(tine_core::refs::block_id(raw, false), None);
    let literal = "Target\n#+BEGIN_SRC text\n:PROPERTIES:\n:id: literal\n:END:\n#+END_SRC";
    assert_eq!(tine_core::refs::block_id(literal, true), None);
}

#[test]
fn clock_out_splices_only_the_accepted_row_with_its_line_endings() {
    let now = TimestampParts {
        year: 2026,
        month: 9,
        day: 30,
        weekday: 3,
        hour: 9,
        minute: 5,
        second: 0,
    };
    for format in [LogbookFormat::Markdown, LogbookFormat::Org] {
        for nl in ["\n", "\r\n"] {
            let raw = format!(
                "Task{nl}:LOGBOOK:{nl}  CLOCK: [2026-09-30 Wed 09:00]  {nl}:END:{nl}tail{nl}"
            );
            let expected = raw.replace(
                "CLOCK: [2026-09-30 Wed 09:00]",
                "CLOCK: [2026-09-30 Wed 09:00]--[2026-09-30 Wed 09:05] =>  00:05",
            );
            let out = logbook::clock_out_at(&raw, format, false, now);
            assert_eq!(out, expected);
            assert_eq!(
                logbook::clock_summary_seconds(&out, format == LogbookFormat::Org),
                300
            );
            assert!(logbook::has_logbook_drawer(
                &out,
                format == LogbookFormat::Org
            ));
        }
    }
}

#[test]
fn annotation_refresh_keeps_literal_properties() {
    let raw = "Highlight\nls-type:: annotation\nid:: highlight\nhl-color:: yellow\nhl-page:: 1\n```\nhl-color:: literal\nhl-page:: 99\n```";
    let doc = Document {
        pre_block: None,
        roots: vec![DocBlock::new(raw)],
    };
    let h: Highlight = serde_json::from_value(serde_json::json!({"id":"highlight", "page":2, "color":"blue", "position":{"page":2,"bounding":{"top":0,"left":0,"width":1,"height":1},"rects":[]},"text":"Highlight","image":null})).unwrap();
    let out =
        pdf::merge_hls_page_for_format(Some(&doc), "a.pdf", "a", &[h], &HashSet::new(), Format::Md);
    assert!(out.roots[0]
        .raw()
        .contains("```\nhl-color:: literal\nhl-page:: 99\n```"));
    assert!(out.roots[0].raw().contains("hl-color:: blue"));
}

#[test]
fn rename_keeps_org_inline_literals() {
    let raw = "~[[Old]]~ =[[Old]]= [[Old]]";
    let map = HashMap::from([("old".to_string(), "New".to_string())]);
    assert_eq!(
        refs::rename_refs_multi(raw, &map, true, FileNameFormat::TripleLowbar),
        "~[[Old]]~ =[[Old]]= [[New]]"
    );
}

#[test]
fn keep_both_command_preserves_literal_id() {
    let id = "aaaaaaaa-0000-0000-0000-0000000000cd";
    let mine = vec![DocBlock::new(format!("winner\nid:: {id}"))];
    let raw = format!("their text\n```\nid:: {id}\n```\nid:: {id}");
    let theirs = vec![DocBlock::new(&raw)];
    let decisions = HashMap::from([("0".to_string(), "both".to_string())]);
    let out = tine_core::sync_diff::merge_blocks(&mine, &theirs, &decisions).unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!(out[1].raw(), format!("their text\n```\nid:: {id}\n```"));
    assert_eq!(out[1].property("id"), None);
}
