//! Concord (og family 8): sync-copy and VCS-marker discovery, the derived
//! conflict queue, diffs, guarded merges, and recoverable trash. The conflict
//! copy is never treated as a graph page or entered into the cache. Nothing
//! here is persisted: the queue is recomputed from disk on every request.

use std::collections::HashMap;
use std::io;

use tine_core::concord_queue::{
    decidable_row_count, parse_vcs_marker_sides, vcs_conflict_markers, ConflictInventory,
    ConflictObject, ConflictSide, ConflictSource, MarkerConflictDiff, SideRole, VcsMarkerConflict,
};
use tine_core::date::JournalFormat;
use tine_core::doc::{self, Document};
use tine_core::model::{
    decode_page_name, sync_conflict_base, Format, PageDto, PageKind, SyncConflict,
};
use tine_core::projection::{assign_doc_runtime_ids, block_to_dto};
use tine_core::sync_diff::{self, SyncConflictDiff};
use tine_store::{Area, FileId, FileRev, PageId, SaveBase, Store};

fn invalid_path() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "invalid file path")
}

fn id(store: &Store, rel: &str) -> io::Result<FileId> {
    let config = store.config();
    for (area, dir) in [
        (Area::Pages, &config.pages_dir),
        (Area::Journals, &config.journals_dir),
    ] {
        if let Some(tail) = rel.strip_prefix(&format!("{dir}/")) {
            let file = store.file_id(area, tail).map_err(|_| invalid_path())?;
            return tine_store::is_graph_text(&file)
                .then_some(file)
                .ok_or_else(invalid_path);
        }
    }
    Err(invalid_path())
}

fn read_text(store: &Store, id: &FileId) -> io::Result<(String, FileRev)> {
    crate::parsed_text::read(store, id)
}

fn format(id: &FileId) -> Format {
    if id.as_str().ends_with(".org") {
        Format::Org
    } else {
        Format::Md
    }
}

fn parse(raw: &str, fmt: Format) -> Document {
    if fmt == Format::Org {
        tine_core::org::parse_org(raw)
    } else {
        doc::parse(raw)
    }
}

fn preview(store: &Store, file: &FileId) -> String {
    read_text(store, file)
        .ok()
        .and_then(|(content, _)| {
            content
                .lines()
                .map(|line| {
                    line.trim_start_matches(|ch| matches!(ch, '*' | '-' | ' ' | '\t'))
                        .trim()
                        .to_owned()
                })
                .find(|line| !line.is_empty())
        })
        .map(|line| line.chars().take(80).collect())
        .unwrap_or_default()
}

/// List Syncthing and Dropbox copies in pages and journals. Unreadable scan
/// entries are skipped and unreadable previews are empty. Cost O(P + J +
/// conflict-copy bytes), including winner presence checks.
pub fn list_sync_conflicts(store: &Store) -> Vec<SyncConflict> {
    let config = store.config();
    let journal_format = JournalFormat::new(
        config.journal_file_name_format.as_deref(),
        config.journal_page_title_format.as_deref(),
    );
    let mut out = Vec::new();
    for (area, kind) in [
        (Area::Journals, PageKind::Journal),
        (Area::Pages, PageKind::Page),
    ] {
        let Ok(listing) = store.scan_area(area, None) else {
            continue;
        };
        let present: std::collections::HashSet<_> = listing
            .files
            .iter()
            .map(|entry| entry.rel.clone())
            .collect();
        for entry in listing.files {
            let Some((name_stem, ext)) = entry.rel.rsplit_once('.') else {
                continue;
            };
            if !tine_store::is_graph_text(&entry.id) {
                continue;
            }
            let Some(base_stem) = sync_conflict_base(name_stem) else {
                continue;
            };
            let base_rel = if let Some((parent, _)) = entry.rel.rsplit_once('/') {
                format!("{parent}/{base_stem}.{ext}")
            } else {
                format!("{base_stem}.{ext}")
            };
            let base_path = present
                .contains(base_rel.as_str())
                .then(|| store.file_id(area, &base_rel).ok())
                .flatten()
                .map(|base| base.as_str().to_owned());
            let base_name = if kind == PageKind::Journal {
                journal_format
                    .parse(base_stem)
                    .map(|day| journal_format.title(day))
                    .unwrap_or_else(|| base_stem.to_owned())
            } else {
                decode_page_name(base_stem, config.file_name_format)
            };
            let tag = name_stem[base_stem.len()..]
                .trim_matches(|ch: char| matches!(ch, '.' | ' ' | '(' | ')'))
                .to_owned();
            out.push(SyncConflict {
                path: entry.id.as_str().to_owned(),
                base_name,
                base_path,
                kind,
                tag,
                preview: preview(store, &entry.id),
            });
        }
    }
    out.sort_by(|a, b| {
        a.base_name
            .cmp(&b.base_name)
            .then_with(|| a.path.cmp(&b.path))
    });
    out
}

/// Structural diff of two exact files. A missing file or invalid path returns
/// `None`; undecodable bytes error as in v0.6.5. Cost O(both file bytes + blocks).
pub fn sync_conflict_diff(
    store: &Store,
    winner: &str,
    conflict: &str,
) -> io::Result<Option<SyncConflictDiff>> {
    let (Ok(win), Ok(conf)) = (id(store, winner), id(store, conflict)) else {
        return Ok(None);
    };
    let (mine, base_rev) = match read_text(store, &win) {
        Ok(read) => read,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let (theirs, conflict_rev) = match read_text(store, &conf) {
        Ok(read) => read,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut diff =
        sync_diff::diff_docs(&parse(&mine, format(&win)), &parse(&theirs, format(&conf)));
    diff.base_rev = base_rev.into();
    diff.conflict_rev = conflict_rev.into();
    Ok(Some(diff))
}

fn union_pre(mine: Option<&str>, theirs: Option<&str>) -> Option<String> {
    let mine = mine.unwrap_or("");
    let Some(theirs) = theirs else {
        return (!mine.is_empty()).then(|| mine.to_owned());
    };
    let keys: std::collections::HashSet<_> = mine
        .lines()
        .filter_map(|line| doc::parse_property_line(line).map(|(key, _)| key.to_ascii_lowercase()))
        .collect();
    let extra: Vec<_> = theirs
        .lines()
        .filter(|line| {
            doc::parse_property_line(line)
                .is_some_and(|(key, _)| !keys.contains(&key.to_ascii_lowercase()))
        })
        .collect();
    if extra.is_empty() {
        return (!mine.is_empty()).then(|| mine.to_owned());
    }
    let mut output = mine.to_owned();
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
    for (index, line) in extra.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        output.push_str(line);
    }
    Some(output)
}

fn choose_pre(choice: &str, fmt: Format, mine: &Document, theirs: &Document) -> Option<String> {
    match choice {
        "theirs" => theirs.pre_block.clone(),
        "mine" => mine.pre_block.clone(),
        _ if fmt == Format::Md => union_pre(mine.pre_block.as_deref(), theirs.pre_block.as_deref()),
        _ => mine.pre_block.clone(),
    }
}

fn merge_refused(refusal: sync_diff::MergeRefused) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, refusal.to_string())
}

fn dto(store: &Store, id: &PageId, mut doc: Document) -> PageDto {
    assign_doc_runtime_ids(&mut doc.roots, id.as_str());
    let config = store.config();
    let stem = id
        .as_str()
        .rsplit('/')
        .next()
        .unwrap_or(id.as_str())
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or("");
    let kind = if id
        .as_str()
        .starts_with(&format!("{}/", config.journals_dir))
    {
        PageKind::Journal
    } else {
        PageKind::Page
    };
    let name = if kind == PageKind::Journal {
        let fmt = JournalFormat::new(
            config.journal_file_name_format.as_deref(),
            config.journal_page_title_format.as_deref(),
        );
        fmt.parse(stem)
            .map(|day| fmt.title(day))
            .unwrap_or_else(|| stem.to_owned())
    } else {
        decode_page_name(stem, config.file_name_format)
    };
    PageDto {
        title: name.clone(),
        name,
        kind,
        pre_block: doc.pre_block,
        blocks: doc.roots.iter().map(block_to_dto).collect(),
        rev: None,
        format: format(&id.file()),
        read_only: false,

        guide: false,
    }
}

/// Merge the selected blocks into the winner, then trash the copy in one
/// guarded transaction. Stale UI revisions yield `winner changed on disk` or
/// `conflict copy changed on disk`; Org round-trip refusal is unchanged. Cost
/// O(both file bytes + blocks) per attempt, at most four attempts.
pub fn resolve_sync_conflict(
    store: &Store,
    winner: &str,
    conflict: &str,
    decisions: &HashMap<String, String>,
    base_rev: &str,
    conflict_rev: &str,
    pre_choice: &str,
) -> io::Result<()> {
    let win = id(store, winner)?;
    let conf = id(store, conflict)?;
    if win == conf {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "winner and conflict are the same file",
        ));
    }
    let page = store.as_page(&win).ok_or_else(invalid_path)?;
    crate::retry_on_conflict("conflict files changed repeatedly during merge", || {
        let (mine, win_rev) = read_text(store, &win)?;
        let (theirs, conf_rev) = read_text(store, &conf)?;
        if String::from(win_rev.clone()) != base_rev {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "winner changed on disk",
            ));
        }
        if String::from(conf_rev.clone()) != conflict_rev {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "conflict copy changed on disk",
            ));
        }
        let fmt = format(&win);
        if fmt == Format::Org
            && (!tine_core::org::org_editable(&mine) || !tine_core::org::org_editable(&theirs))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "an org file in this pair does not round-trip; not merging",
            ));
        }
        let mine_doc = parse(&mine, fmt);
        let their_doc = parse(&theirs, format(&conf));
        // A conflict copy carries no common ancestor, so this is the 2-way
        // merge; a forged `"merged"` decision refuses the whole resolve.
        let roots = sync_diff::merge_blocks(&mine_doc.roots, &their_doc.roots, decisions)
            .map_err(merge_refused)?;
        let pre_block = choose_pre(pre_choice, fmt, &mine_doc, &their_doc);
        let merged = dto(store, &page, Document { pre_block, roots });
        let mut tx = store.transaction(Some(tine_store::EditKind::ReplacePage));
        tx.save_page(
            &[
                tine_store::EditKind::ReplacePage,
                tine_store::EditKind::DeletePage,
            ],
            &page,
            SaveBase::Existing(win_rev),
            &merged,
        );
        tx.trash(&conf, conf_rev);
        Ok(crate::commit_retry(tx.commit())?.then_some(()))
    })
}

/// Recoverably trash only a sync copy, with four revision-guard retries. Cost
/// O(copy bytes) per attempt.
pub fn trash_sync_conflict(store: &Store, conflict: &str) -> io::Result<()> {
    let conf = id(store, conflict)?;
    let stem = conf
        .as_str()
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(stem, _)| stem);
    if !stem.is_some_and(|stem| sync_conflict_base(stem).is_some()) {
        return Err(invalid_path());
    }
    crate::retry_on_conflict("conflict copy changed repeatedly during trash", || {
        crate::trash_current(
            store,
            &conf,
            Some(tine_store::PARSE_INPUT_MAX_BYTES),
            "no such conflict file",
        )
    })
}

/// Page and journal files carrying unresolved VCS markers. They stay real,
/// readable pages; the store refuses to save them (R-VCS-MARKERS). Sync-tool
/// copies are listed by [`list_sync_conflicts`] instead. Unreadable files are
/// skipped (no refusal on an inventory read). Cost: one bounded read of every
/// page and journal file, O(graph text bytes); a byte prefilter skips the
/// UTF-8 check and line scan for files without an anchor marker.
fn list_vcs_marker_pages(store: &Store) -> Vec<VcsMarkerConflict> {
    let config = store.config();
    let journal_format = JournalFormat::new(
        config.journal_file_name_format.as_deref(),
        config.journal_page_title_format.as_deref(),
    );
    let mut out = Vec::new();
    for (area, kind) in [
        (Area::Journals, PageKind::Journal),
        (Area::Pages, PageKind::Page),
    ] {
        let Ok(listing) = store.scan_area(area, None) else {
            continue;
        };
        for entry in listing.files {
            let Some((stem, _)) = entry
                .rel
                .rsplit('/')
                .next()
                .and_then(|n| n.rsplit_once('.'))
            else {
                continue;
            };
            if !tine_store::is_graph_text(&entry.id) || sync_conflict_base(stem).is_some() {
                continue;
            }
            let Ok((bytes, _)) = store.read(&entry.id, Some(tine_store::PARSE_INPUT_MAX_BYTES))
            else {
                continue;
            };
            if !has_anchor(&bytes) {
                continue;
            }
            let Ok(text) = std::str::from_utf8(&bytes) else {
                continue;
            };
            let markers = vcs_conflict_markers(text);
            if markers.is_empty() {
                continue;
            }
            let name = if kind == PageKind::Journal {
                journal_format
                    .parse(stem)
                    .map(|day| journal_format.title(day))
                    .unwrap_or_else(|| stem.to_owned())
            } else {
                decode_page_name(stem, config.file_name_format)
            };
            out.push(VcsMarkerConflict {
                path: entry.id.as_str().to_owned(),
                name,
                kind,
                markers: markers.iter().map(|m| m.to_string()).collect(),
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Whether `bytes` contain a column-0 anchor marker line (`<<<<<<< ` or
/// `>>>>>>> `); the exact rules live in `vcs_conflict_markers`.
fn has_anchor(bytes: &[u8]) -> bool {
    bytes
        .split(|&b| b == b'\n')
        .any(|line| line.starts_with(b"<<<<<<< ") || line.starts_with(b">>>>>>> "))
}

/// The Concord conflict queue: ONE derived inventory of everything on disk
/// that needs the user's judgement — sync-tool copies paired with their
/// winner, and marker-bearing pages. Never persisted and never an authority:
/// the same disk state recomputes the same objects with the same ids, and
/// every resolve re-checks the files it writes. A copy whose winner is gone is
/// a stray, not a two-sided conflict, and stays out (Settings discards it).
/// Cost: the two listings plus one diff per queued item (conflicts are few).
pub fn conflict_inventory(store: &Store) -> ConflictInventory {
    let sync_conflicts = list_sync_conflicts(store);
    let vcs_markers = list_vcs_marker_pages(store);
    let mut out = Vec::new();
    for copy in &sync_conflicts {
        let Some(winner) = copy.base_path.clone() else {
            continue;
        };
        let diff = sync_conflict_diff(store, &winner, &copy.path)
            .ok()
            .flatten();
        out.push(ConflictObject {
            id: format!("copy:{}", copy.path),
            source: ConflictSource::SyncCopy,
            page_name: copy.base_name.clone(),
            page_path: winner.clone(),
            kind: copy.kind,
            sides: vec![
                ConflictSide {
                    role: SideRole::Mine,
                    label: "This device".to_string(),
                    path: Some(winner),
                },
                ConflictSide {
                    role: SideRole::Theirs,
                    label: if copy.tag.is_empty() {
                        "Conflict copy".to_string()
                    } else {
                        copy.tag.clone()
                    },
                    path: Some(copy.path.clone()),
                },
            ],
            block_conflicts: diff.as_ref().map(|d| decidable_row_count(&d.rows)),
            markers: Vec::new(),
        });
    }
    for marked in &vcs_markers {
        let parsed = vcs_marker_conflict_diff(store, &marked.path).ok().flatten();
        let label = |pick: fn(&MarkerConflictDiff) -> &str, fallback: &str| {
            parsed
                .as_ref()
                .map(pick)
                .filter(|l| !l.is_empty())
                .unwrap_or(fallback)
                .to_string()
        };
        let mut sides = vec![
            ConflictSide {
                role: SideRole::Mine,
                label: label(|p| p.mine_label.as_str(), "Local side"),
                path: None,
            },
            ConflictSide {
                role: SideRole::Theirs,
                label: label(|p| p.theirs_label.as_str(), "Merged-in side"),
                path: None,
            },
        ];
        if parsed.as_ref().is_some_and(|p| p.diff.three_way) {
            sides.push(ConflictSide {
                role: SideRole::Base,
                label: "Common ancestor".to_string(),
                path: None,
            });
        }
        out.push(ConflictObject {
            id: format!("markers:{}", marked.path),
            source: ConflictSource::VcsMarkers,
            page_name: marked.name.clone(),
            page_path: marked.path.clone(),
            kind: marked.kind,
            sides,
            block_conflicts: parsed.as_ref().map(|p| decidable_row_count(&p.diff.rows)),
            markers: marked.markers.clone(),
        });
    }
    out.sort_by(|a, b| a.page_name.cmp(&b.page_name).then_with(|| a.id.cmp(&b.id)));
    ConflictInventory {
        sync_conflicts,
        vcs_markers,
        queue: out,
    }
}

/// Block diff of a marker-bearing page's own sides: 3-way against the diff3
/// base the markers carry (with Fossil's suggestion as the artifact), else
/// 2-way. Read-only. Both revs are the whole marker file's rev, so the resolve
/// guard rejects decisions made against a version the VCS has since changed.
/// `Ok(None)` if the path is invalid, gone, or not conflicted. Cost O(file
/// bytes + blocks).
pub fn vcs_marker_conflict_diff(
    store: &Store,
    rel: &str,
) -> io::Result<Option<MarkerConflictDiff>> {
    let Ok(file) = id(store, rel) else {
        return Ok(None);
    };
    let (content, rev) = match read_text(store, &file) {
        Ok(read) => read,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some(sides) = parse_vcs_marker_sides(&content) else {
        return Ok(None);
    };
    let fmt = format(&file);
    let mine = parse(&sides.mine, fmt);
    let theirs = parse(&sides.theirs, fmt);
    let mut diff = match sides.base.as_deref() {
        Some(base) => {
            let artifact = sides.suggested.as_deref().map(|text| parse(text, fmt));
            sync_diff::diff3_docs_with_artifact(
                &parse(base, fmt),
                &mine,
                &theirs,
                artifact.as_ref(),
            )
        }
        // No ancestor: no `BothChanged` verdict, so no proposal of either kind.
        None => sync_diff::diff_docs(&mine, &theirs),
    };
    let rev: String = rev.into();
    diff.base_rev = rev.clone();
    diff.conflict_rev = rev;
    Ok(Some(MarkerConflictDiff {
        mine_label: sides.mine_label,
        theirs_label: sides.theirs_label,
        regions: sides.regions,
        diff,
    }))
}

/// Apply the user's per-row decisions to a marker-bearing page and write the
/// clean result: the one write R-VCS-MARKERS permits to such a file, through
/// `SaveBase::ResolvingMarkers`, which stages the pre-resolution bytes in
/// conflict trash in the same transaction. A stale `base_rev` returns
/// `AlreadyExists` ("file changed on disk") and writes nothing. The merge
/// re-derives the SAME alignment (and any merged body) from the guarded bytes,
/// never from the client. Cost O(file bytes + blocks) per attempt, at most
/// four attempts.
pub fn resolve_vcs_marker_conflict(
    store: &Store,
    rel: &str,
    decisions: &HashMap<String, String>,
    base_rev: &str,
    pre_choice: &str,
) -> io::Result<()> {
    let file = id(store, rel)?;
    let page = store.as_page(&file).ok_or_else(invalid_path)?;
    crate::retry_on_conflict("marker file changed repeatedly during resolve", || {
        let (content, rev) = read_text(store, &file)?;
        // Scenario: the VCS or an external editor changed the file after the
        // review was computed; the user must review the new version.
        if String::from(rev.clone()) != base_rev {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "file changed on disk",
            ));
        }
        let Some(sides) = parse_vcs_marker_sides(&content) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "no VCS merge conflict markers to resolve",
            ));
        };
        let fmt = format(&file);
        // Scenario: malformed imported Org — a side that does not round-trip
        // would be rewritten lossily.
        if fmt == Format::Org
            && (!tine_core::org::org_editable(&sides.mine)
                || !tine_core::org::org_editable(&sides.theirs))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "an org side of this merge does not round-trip; not resolving",
            ));
        }
        let mine = parse(&sides.mine, fmt);
        let theirs = parse(&sides.theirs, fmt);
        let base = sides.base.as_deref().map(|text| parse(text, fmt));
        let artifact = sides.suggested.as_deref().map(|text| parse(text, fmt));
        let roots = sync_diff::merge_blocks3(
            base.as_ref().map(|doc| doc.roots.as_slice()),
            &mine.roots,
            &theirs.roots,
            artifact.as_ref().map(|doc| doc.roots.as_slice()),
            decisions,
        )
        .map_err(merge_refused)?;
        let pre_block = choose_pre(pre_choice, fmt, &mine, &theirs);
        let merged = dto(store, &page, Document { pre_block, roots });
        let mut tx = store.transaction(Some(tine_store::EditKind::ReplacePage));
        tx.save_page(
            &[tine_store::EditKind::ReplacePage],
            &page,
            SaveBase::ResolvingMarkers(rev),
            &merged,
        );
        Ok(crate::commit_retry(tx.commit())?.then_some(()))
    })
}
