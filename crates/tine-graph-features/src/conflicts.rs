//! Sync-copy discovery, diff, guarded merge, and recoverable trash. The
//! conflict copy is never treated as a graph page or entered into the cache.

use std::collections::HashMap;
use std::io;

use tine_core::date::JournalFormat;
use tine_core::doc::{self, Document};
use tine_core::model::{
    decode_page_name, sync_conflict_base, Format, PageDto, PageKind, SyncConflict,
};
use tine_core::projection::{assign_doc_runtime_ids, block_to_dto};
use tine_core::sync_diff::{self, SyncConflictDiff};
use tine_store::{Area, FileId, FileRev, PageId, SaveBase, Store, StoreError};

use crate::{is_conflict, store_error, tx_error};

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
            if !matches!(
                tail.rsplit_once('.').map(|(_, ext)| ext),
                Some("md" | "org")
            ) {
                return Err(invalid_path());
            }
            return store.file_id(area, tail).map_err(|_| invalid_path());
        }
    }
    Err(invalid_path())
}

fn read_text(store: &Store, id: &FileId) -> io::Result<(String, FileRev)> {
    let (bytes, rev) = store.read(id, None).map_err(store_error)?;
    let content = String::from_utf8(bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "stream did not contain valid UTF-8",
        )
    })?;
    Ok((content, rev))
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
            if !matches!(ext, "md" | "org") {
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
    for _ in 0..4 {
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
        let roots = sync_diff::merge_blocks(&mine_doc.roots, &their_doc.roots, decisions);
        let pre_block = match pre_choice {
            "theirs" => their_doc.pre_block.clone(),
            "mine" => mine_doc.pre_block.clone(),
            _ if fmt == Format::Md => union_pre(
                mine_doc.pre_block.as_deref(),
                their_doc.pre_block.as_deref(),
            ),
            _ => mine_doc.pre_block.clone(),
        };
        let merged = dto(store, &page, Document { pre_block, roots });
        let mut tx = store.transaction();
        tx.save_page(&page, SaveBase::Existing(win_rev), &merged);
        tx.trash(&conf, conf_rev);
        let outcome = tx.commit();
        if is_conflict(&outcome) {
            continue;
        }
        tx_error(outcome)?;
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "conflict files changed repeatedly during merge",
    ))
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
    for _ in 0..4 {
        let rev = match store.read(&conf, None) {
            Ok((_, rev)) => rev,
            Err(StoreError::NotFound) => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "no such conflict file",
                ))
            }
            Err(error) => return Err(store_error(error)),
        };
        let mut tx = store.transaction();
        tx.trash(&conf, rev);
        let outcome = tx.commit();
        if is_conflict(&outcome) {
            continue;
        }
        tx_error(outcome)?;
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "conflict copy changed repeatedly during trash",
    ))
}
