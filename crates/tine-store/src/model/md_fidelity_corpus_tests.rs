//! Corpus gate for page-save fidelity (GH #110 parts a+b): over a copy of a
//! real graph, the bytes a save would write ([`Graph::prepare_page_content`],
//! the one serializer every page save reaches) must equal the file when
//! nothing changed, and must change nothing outside the one block an edit
//! touched. Pure: no file is written; point it at a read-only copy.
//!
//! `TINE_MD_FIDELITY_CORPUS=<graph>[:<graph>…] cargo test -p tine-store --release \
//!   md_fidelity_corpus -- --ignored --nocapture`
//!
//! Prints counts per content shape only (no names, paths or text) unless
//! `TINE_MD_FIDELITY_PATHS=1`, for local triage. Operations per block: append
//! to its first line, append to its last continuation line, insert a sibling
//! after it, delete it (a leaf). Fails on any byte change outside the edit, a
//! no-edit save that changes the file, a page whose layout retention declines,
//! or a page the whole-page serializer cannot round-trip.

use super::*;
use std::collections::BTreeMap;

const EDIT: &str = "\u{27e6}E\u{27e7}";
const NEW: &str = "\u{27e6}N\u{27e7}";

/// Physical lines with their terminators (`\r\n`, `\n` or a lone `\r`).
fn lines_of(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let (mut out, mut start, mut i) = (Vec::new(), 0, 0);
    while i < bytes.len() {
        let end = match bytes[i] {
            b'\n' => i + 1,
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => i + 2,
            b'\r' => i + 1,
            _ => {
                i += 1;
                continue;
            }
        };
        out.push(&text[start..end]);
        start = end;
        i = end;
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// Content-free shape tags of one file (heuristic line scan, for reporting).
fn shapes(text: &str, doc: &Document, is_org: bool) -> Vec<&'static str> {
    let mut tags = Vec::new();
    if is_org {
        tags.push("org");
        return tags;
    }
    if text.starts_with("---\n") || text.starts_with("---\r\n") {
        tags.push("front-matter");
    }
    let mut fence = false;
    let mut heading = false;
    let mut table = false;
    let mut list = false;
    let mut prose = false;
    let mut blank = false;
    let lines: Vec<&str> = text.split('\n').collect();
    let last_content = lines.iter().rposition(|l| !l.trim().is_empty());
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start_matches([' ', '\t']);
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
            continue;
        }
        if fence {
            continue;
        }
        if tine_core::doc::is_unbulleted_heading_line(line.trim_end_matches('\r')) {
            heading = true;
        }
        if t.starts_with('|') {
            table = true;
        }
        let item = t
            .strip_prefix(['*', '+'])
            .is_some_and(|r| r.starts_with(' '))
            || t.split_once(['.', ')']).is_some_and(|(n, r)| {
                !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && r.starts_with(' ')
            });
        if item {
            list = true;
        }
        if line.trim().is_empty() && last_content.is_some_and(|last| i < last) && i > 0 {
            blank = true;
        }
        let col0 = !line.starts_with([' ', '\t']);
        if col0
            && !t.trim().is_empty()
            && !t.starts_with('-')
            && !t.starts_with('#')
            && !t.contains(":: ")
            && !t.ends_with("::")
            && !t.starts_with('|')
            && !item
        {
            prose = true;
        }
    }
    if heading {
        tags.push("unbulleted-heading");
    }
    if table {
        tags.push("table");
    }
    if list {
        tags.push("non-dash-list");
    }
    if prose {
        tags.push("prose");
    }
    if blank {
        tags.push("blank-lines");
    }
    if doc.roots.is_empty() && doc.pre_block.is_some() {
        tags.push("preamble-only");
    }
    if tags.is_empty() {
        tags.push("outline-only");
    }
    tags
}

/// Every block's child-index path, in pre-order.
fn paths(blocks: &[BlockDto], prefix: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    for (i, block) in blocks.iter().enumerate() {
        prefix.push(i);
        out.push(prefix.clone());
        paths(&block.children, prefix, out);
        prefix.pop();
    }
}

/// A copy of `page` with `f` applied to the sibling list holding the block at
/// `path`, and that block's index in it.
fn with_block(
    page: &PageDto,
    path: &[usize],
    f: impl FnOnce(&mut Vec<BlockDto>, usize),
) -> PageDto {
    let mut page = page.clone();
    let (&last, parents) = path.split_last().expect("a block path");
    let mut list = &mut page.blocks;
    for &i in parents {
        list = &mut list[i].children;
    }
    f(list, last);
    page
}

fn block_at<'a>(page: &'a PageDto, path: &[usize]) -> &'a BlockDto {
    let (&last, parents) = path.split_last().expect("a block path");
    let mut list = &page.blocks;
    for &i in parents {
        list = &list[i].children;
    }
    &list[last]
}

#[derive(Default)]
struct Tally {
    files: usize,
    noedit_identical: usize,
    noedit_changed: usize,
    noedit_refused: usize,
    edits: usize,
    edit_local: usize,
    edit_outside: usize,
    edit_refused: usize,
    files_with_outside_edit: usize,
    retention_declines: usize,
    full_serializer_structural: usize,
    tail_edits: usize,
    tail_local: usize,
    tail_outside: usize,
    tail_refused: usize,
    inserts: usize,
    insert_local: usize,
    insert_outside: usize,
    insert_refused: usize,
    deletes: usize,
    delete_local: usize,
    delete_outside: usize,
    delete_refused: usize,
}

/// `out` differs from `src` only in the one line that carries the marker
/// (the edited block's first line: its text, plus any bullet marker the new
/// text needs, as for a bare `-` or a lone `#`).
fn edited_in_place(src: &str, out: &str) -> bool {
    let a: Vec<&str> = lines_of(src);
    let b: Vec<&str> = lines_of(out);
    a.len() == b.len()
        && a.iter()
            .zip(&b)
            .filter(|(x, y)| x != y)
            .all(|(_, y)| y.contains(EDIT))
}

/// `out` is `src` with exactly one line inserted (an insertion after an
/// unterminated final line also terminates that line).
fn one_line_inserted(src: &str, out: &str, marker: &str) -> bool {
    let mut lines: Vec<&str> = lines_of(out);
    let Some(at) = lines.iter().position(|l| l.contains(marker)) else {
        return false;
    };
    let last = at + 1 == lines.len();
    lines.remove(at);
    let rest = lines.concat();
    rest == src
        || (last
            && [rest.strip_suffix("\r\n"), rest.strip_suffix(['\n', '\r'])].contains(&Some(src)))
}

/// `out` is `src` with one contiguous run of whole lines removed (removing
/// the final lines also drops the new last line's terminator when the file
/// had none).
fn one_run_removed(src: &str, out: &str) -> bool {
    let a: Vec<&str> = lines_of(src);
    let b: Vec<&str> = lines_of(out);
    if b.len() >= a.len() {
        return false;
    }
    let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let tail = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    head + tail >= b.len()
        || (!src.ends_with('\n')
            && src
                .strip_prefix(out)
                .is_some_and(|rest| rest.starts_with(['\n', '\r'])))
}

/// Local triage only (`TINE_MD_FIDELITY_PATHS`): the first differing lines.
fn print_first_difference(src: &str, out: &str) {
    let a: Vec<&str> = lines_of(src);
    let b: Vec<&str> = lines_of(out);
    let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let tail = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count()
        .min(a.len().min(b.len()) - head);
    println!("    @@ line {head}");
    for line in &a[head.saturating_sub(1)..a.len() - tail] {
        println!("    -{line:?}");
    }
    for line in &b[head.saturating_sub(1)..b.len() - tail] {
        println!("    +{line:?}");
    }
}

#[test]
#[ignore = "corpus gate: set TINE_MD_FIDELITY_CORPUS"]
fn md_fidelity_corpus() {
    let corpora = std::env::var("TINE_MD_FIDELITY_CORPUS").expect("TINE_MD_FIDELITY_CORPUS");
    let show = std::env::var("TINE_MD_FIDELITY_PATHS").is_ok();
    let mut failures = 0;
    for root in corpora.split(':').filter(|r| !r.is_empty()) {
        let root = PathBuf::from(root);
        let graph = Graph::open(&root);
        let mut by_shape: BTreeMap<&'static str, Tally> = BTreeMap::new();
        let mut stack = vec![root.clone()];
        let mut files = Vec::new();
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || (dir == root && name == "logseq") {
                    continue;
                }
                if entry.file_type().unwrap().is_dir() {
                    stack.push(path);
                } else if matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("md" | "markdown" | "org")
                ) {
                    files.push(path);
                }
            }
        }
        files.sort();
        for path in files {
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            if content.contains(EDIT) || content.contains(NEW) {
                continue;
            }
            let Ok(Some(dto)) = graph.load_by_validated_path(&path) else {
                continue;
            };
            let is_org = Format::from_path(&path) == Format::Org;
            let parsed = parse_doc(&path, &content);
            let tags = shapes(&content, &parsed, is_org);
            let rel = path.strip_prefix(&root).unwrap().display().to_string();
            let mut t = Tally {
                files: 1,
                ..Tally::default()
            };
            if !is_org {
                let (old, opts) = parse_doc_with_opts(&path, &content);
                let doc = tine_core::projection::page_dto_document(&dto, false);
                if super::layout_retention::serialize(&doc, &content, &old, &opts).is_none()
                    && !(doc.roots.is_empty() && doc.pre_block.is_none())
                {
                    t.retention_declines += 1;
                    if show {
                        println!("  RETENTION-DECLINES {rel}");
                    }
                }
                if parse_doc(&path, &doc::serialize_with(&old, &opts)) != old {
                    t.full_serializer_structural += 1;
                    if show {
                        println!("  FULL-STRUCTURAL {rel}");
                    }
                }
            }
            let save = |page: &PageDto| graph.prepare_page_content(page, &path, Some(&content));
            match save(&dto) {
                Ok((out, _)) if out == content => t.noedit_identical += 1,
                Ok(_) => {
                    t.noedit_changed += 1;
                    if show {
                        println!("  NOEDIT-CHANGED {rel}");
                    }
                }
                Err(_) => t.noedit_refused += 1,
            }
            let mut all = Vec::new();
            paths(&dto.blocks, &mut Vec::new(), &mut all);
            let mut outside_here = 0;
            for (k, path) in all.iter().enumerate() {
                // Edit: append a marker to the block's first line.
                let page = with_block(&dto, path, |list, i| {
                    let raw = &mut list[i].raw;
                    let end = raw.find('\n').unwrap_or(raw.len());
                    raw.insert_str(end, EDIT);
                });
                t.edits += 1;
                match save(&page) {
                    Ok((out, _)) if edited_in_place(&content, &out) => t.edit_local += 1,
                    Ok((out, _)) => {
                        t.edit_outside += 1;
                        outside_here += 1;
                        if show && outside_here == 1 {
                            println!("  EDIT-OUTSIDE {rel} block {k}");
                            print_first_difference(&content, &out.replacen(EDIT, "", 1));
                        }
                    }
                    Err(_) => t.edit_refused += 1,
                }
                // Edit the last line of a multi-line block (a continuation),
                // unless it closes a literal region the marker would reopen.
                let mut tail = false;
                let page = with_block(&dto, path, |list, i| {
                    let raw = &mut list[i].raw;
                    let last = raw.rsplit('\n').next().unwrap_or("");
                    let closes = ["```", "~~~", "#+END", "#+end", ":END:"]
                        .iter()
                        .any(|c| last.contains(c));
                    if raw.contains('\n') && !closes && !last.trim().is_empty() {
                        raw.push_str(EDIT);
                        tail = true;
                    }
                });
                if tail {
                    t.tail_edits += 1;
                    match save(&page) {
                        Ok((out, _)) if edited_in_place(&content, &out) => t.tail_local += 1,
                        Ok((out, _)) => {
                            t.tail_outside += 1;
                            if show {
                                println!("  TAIL-OUTSIDE {rel} block {k}");
                                print_first_difference(&content, &out.replacen(EDIT, "", 1));
                            }
                        }
                        Err(_) => t.tail_refused += 1,
                    }
                }
                // Insert a new empty-ish sibling after the block (Enter at its end).
                let page = with_block(&dto, path, |list, i| {
                    let mut block = list[i].clone();
                    block.raw = NEW.into();
                    block.id = String::new();
                    block.children.clear();
                    list.insert(i + 1, block);
                });
                t.inserts += 1;
                match save(&page) {
                    Ok((out, _)) if one_line_inserted(&content, &out, NEW) => t.insert_local += 1,
                    Ok((out, _)) => {
                        t.insert_outside += 1;
                        if show {
                            println!("  INSERT-OUTSIDE {rel} block {k}");
                            print_first_difference(&content, &out);
                        }
                    }
                    Err(_) => t.insert_refused += 1,
                }
                // Delete the block when it is a leaf.
                let leaf = block_at(&dto, path).children.is_empty();
                if leaf && all.len() > 1 {
                    let page = with_block(&dto, path, |list, i| {
                        list.remove(i);
                    });
                    t.deletes += 1;
                    match save(&page) {
                        Ok((out, _)) if one_run_removed(&content, &out) => t.delete_local += 1,
                        Ok((out, _)) => {
                            t.delete_outside += 1;
                            if show {
                                println!("  DELETE-OUTSIDE {rel} block {k}");
                                print_first_difference(&content, &out);
                            }
                        }
                        Err(_) => t.delete_refused += 1,
                    }
                }
            }
            if outside_here > 0 {
                t.files_with_outside_edit = 1;
            }
            failures += t.noedit_changed
                + t.edit_outside
                + t.tail_outside
                + t.insert_outside
                + t.delete_outside
                + t.retention_declines
                + t.full_serializer_structural;
            for tag in tags.iter().copied().chain(["ALL"]) {
                let s = by_shape.entry(tag).or_default();
                s.add(&t);
            }
        }
        println!("corpus {}", root.file_name().unwrap().to_string_lossy());
        println!(
            "  {:<20} {:>5} | noedit: same/chg/ref | edit: n local outside(files) refused | tail-edit: n local outside refused | insert: n local outside refused | delete: n local outside refused | retention-declines full-structural",
            "shape", "files"
        );
        for (tag, s) in &by_shape {
            println!(
                "  {:<20} {:>5} | {:>4}/{}/{} | {:>6} {:>6} {:>4}({}) {:>4} | {:>6} {:>6} {:>4} {:>4} | {:>6} {:>6} {:>4} {:>4} | {:>6} {:>6} {:>4} {:>4} | {:>4} {:>4}",
                tag,
                s.files,
                s.noedit_identical,
                s.noedit_changed,
                s.noedit_refused,
                s.edits,
                s.edit_local,
                s.edit_outside,
                s.files_with_outside_edit,
                s.edit_refused,
                s.tail_edits,
                s.tail_local,
                s.tail_outside,
                s.tail_refused,
                s.inserts,
                s.insert_local,
                s.insert_outside,
                s.insert_refused,
                s.deletes,
                s.delete_local,
                s.delete_outside,
                s.delete_refused,
                s.retention_declines,
                s.full_serializer_structural,
            );
        }
    }
    assert_eq!(
        failures, 0,
        "a save changed bytes outside the edit (rerun with TINE_MD_FIDELITY_PATHS=1)"
    );
}

impl Tally {
    fn add(&mut self, t: &Tally) {
        self.files += t.files;
        self.noedit_identical += t.noedit_identical;
        self.noedit_changed += t.noedit_changed;
        self.noedit_refused += t.noedit_refused;
        self.edits += t.edits;
        self.edit_local += t.edit_local;
        self.edit_outside += t.edit_outside;
        self.edit_refused += t.edit_refused;
        self.files_with_outside_edit += t.files_with_outside_edit;
        self.retention_declines += t.retention_declines;
        self.full_serializer_structural += t.full_serializer_structural;
        self.tail_edits += t.tail_edits;
        self.tail_local += t.tail_local;
        self.tail_outside += t.tail_outside;
        self.tail_refused += t.tail_refused;
        self.inserts += t.inserts;
        self.insert_local += t.insert_local;
        self.insert_outside += t.insert_outside;
        self.insert_refused += t.insert_refused;
        self.deletes += t.deletes;
        self.delete_local += t.delete_local;
        self.delete_outside += t.delete_outside;
        self.delete_refused += t.delete_refused;
    }
}
