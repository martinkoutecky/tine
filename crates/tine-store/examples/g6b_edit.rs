//! Private-graph G6b probe: DTO export, then guarded saves of frontend-projected edits.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
};
use tine_core::model::PageDto;
use tine_store::{EditKind, PageId, SaveBase, SaveOutcome, Store};

#[derive(Serialize, Deserialize)]
struct Entry {
    id: String,
    #[serde(default)]
    doc: Option<PageDto>,
    #[serde(default)]
    cause: Option<String>,
    #[serde(default)]
    span_lines: usize,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().expect("dump|save");
    let graph = args.next().expect("graph copy");
    let file = args.next().expect("JSONL path");
    let store = Store::open(Path::new(&graph), Default::default())
        .expect("open")
        .0;
    let ids = store.whole_graph().expect("load").parsed_page_ids();
    if mode == "dump" {
        let mut out = BufWriter::new(fs::File::create(file).expect("create dump"));
        for id in ids {
            let entry = match store.page(&id) {
                Ok(read) => Entry {
                    id: id.to_string(),
                    doc: Some(read.doc),
                    cause: None,
                    span_lines: 0,
                },
                Err(_) => Entry {
                    id: id.to_string(),
                    doc: None,
                    cause: Some("unreadable".into()),
                    span_lines: 0,
                },
            };
            serde_json::to_writer(&mut out, &entry).expect("write entry");
            out.write_all(b"\n").expect("newline");
        }
        out.flush().expect("flush dump");
    } else if mode == "save" {
        let original = args.next().expect("original graph copy");
        let marker = "__TINE_G6B_06D__";
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        let mut report = BufWriter::new(
            fs::File::create(
                std::env::var("G6B_REPORT").unwrap_or_else(|_| "G6B-06f-results.tsv".into()),
            )
            .expect("report"),
        );
        for line in BufReader::new(fs::File::open(file).expect("open edits")).lines() {
            let entry: Entry = serde_json::from_str(&line.expect("line")).expect("entry");
            let path_hash = format!("{:x}", Sha256::digest(entry.id.as_bytes()));
            let cause = if let Some(doc) = entry.doc {
                let id = PageId::from(entry.id.as_str());
                match store.page(&id) {
                    Err(_) => "read-failed".to_string(),
                    Ok(read) => {
                        let outcome = single_save(
                            &store,
                            id.clone(),
                            SaveBase::Existing(read.rev),
                            doc.clone(),
                            EditKind::SaveBlock,
                        );
                        match outcome {
                            SaveOutcome::Saved(_) | SaveOutcome::Unchanged(_) => {
                                let before = fs::read(Path::new(&original).join(&entry.id))
                                    .expect("original bytes");
                                let after = fs::read(Path::new(&graph).join(&entry.id))
                                    .expect("saved bytes");
                                if let Some(start) = after
                                    .windows(marker.len())
                                    .position(|part| part == marker.as_bytes())
                                {
                                    let marker_line = after[..start]
                                        .iter()
                                        .filter(|&&byte| byte == b'\n')
                                        .count();
                                    let cause = classify_change(
                                        &before,
                                        &after,
                                        marker_line,
                                        entry.span_lines,
                                    );
                                    // Untouched bytes can survive while the edited
                                    // line re-nests the outline; compare structure too.
                                    match store.page(&id) {
                                        Ok(saved) if cause == "pass" => {
                                            if shape(&saved.doc.blocks) == shape(&doc.blocks) {
                                                cause
                                            } else {
                                                "outline-change".to_string()
                                            }
                                        }
                                        Ok(_) => cause,
                                        Err(_) => "reread-failed".to_string(),
                                    }
                                } else {
                                    "edit-not-written".to_string()
                                }
                            }
                            SaveOutcome::ReadOnly(_) => "read-only".to_string(),
                            SaveOutcome::Conflict { .. } => "conflict".to_string(),
                            SaveOutcome::InvalidTarget(_) => "invalid-target".to_string(),
                            SaveOutcome::Io(_) => "io".to_string(),
                            _ => "save-refused".to_string(),
                        }
                    }
                }
            } else {
                entry.cause.unwrap_or_else(|| "no-editable-leaf".into())
            };
            *counts.entry(cause.clone()).or_default() += 1;
            if cause != "pass" {
                writeln!(report, "{cause}\t{}", &path_hash[..16]).expect("report row");
            }
        }
        report.flush().expect("flush report");
        eprintln!("G6b counts: {counts:?}");
        let failures: usize = counts
            .iter()
            .filter(|(cause, _)| cause.as_str() != "pass" && cause.as_str() != "no-leaf-block")
            .map(|(_, count)| count)
            .sum();
        eprintln!("G6b editable-page failures: {failures}");
        if failures != 0 {
            std::process::exit(1);
        }
    } else {
        panic!("unknown mode");
    }
    store.close();
}

fn shape(blocks: &[tine_core::model::BlockDto]) -> String {
    blocks
        .iter()
        .map(|block| format!("[{}]", shape(&block.children)))
        .collect()
}

fn classify_change(before: &[u8], after: &[u8], marker_line: usize, span_lines: usize) -> String {
    let b = String::from_utf8_lossy(before);
    let a = String::from_utf8_lossy(after);
    let bl: Vec<_> = b.split('\n').collect();
    let al: Vec<_> = a.split('\n').collect();
    if bl.len() != al.len() {
        return "line-count-change".into();
    }
    let outside = bl
        .iter()
        .zip(al.iter())
        .enumerate()
        .filter(|(i, _)| *i < marker_line || *i >= marker_line + span_lines);
    if outside.clone().all(|(_, (x, y))| x == y) {
        return "pass".into();
    }
    if outside
        .clone()
        .all(|(_, (x, y))| x.trim_end() == y.trim_end())
    {
        return "other-block-trailing-space".into();
    }
    if outside
        .clone()
        .all(|(_, (x, y))| x.trim_start() == y.trim_start())
    {
        return "indentation-change".into();
    }
    if b.replace("\r\n", "\n") == a.replace("\r\n", "\n") {
        return "line-ending-change".into();
    }
    "other-serialization".into()
}

/// One-page save through the production door, `Store::save_pages`.
fn single_save(
    store: &tine_store::Store,
    id: tine_store::PageId,
    base: tine_store::SaveBase,
    doc: tine_core::model::PageDto,
    kind: tine_store::EditKind,
) -> tine_store::SaveOutcome {
    match store.save_pages(&[(id, base, doc, vec![kind])]) {
        tine_store::SavePagesOutcome::Ok { mut outcomes, .. } => outcomes.remove(0),
        tine_store::SavePagesOutcome::Failed { outcome, .. } => outcome,
    }
}
