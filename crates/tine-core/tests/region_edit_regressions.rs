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
    assert_eq!(logbook::clock_out_at(raw, false, now), raw);
    let raw = "Task\n```\nSCHEDULED: <2026-09-29 Tue>\n```";
    let next = logbook::clock_in_at(raw, LogbookFormat::Markdown, false, now);
    assert!(next.contains("```\nSCHEDULED: <2026-09-29 Tue>\n```"));
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
