//! Seen baselines: "changed since you last looked" (vision decision 9a,
//! Martin 2026-10-04; ADR 0073).
//!
//! **Question answered.** [`seen_baseline`] `load`: which block hashes did the
//! user last mark as seen on this page?
//! **Operations accepted.** `mark` replaces one page's record; `forget` removes
//! it. Nothing else writes it: no edit, page open or close touches this store.
//!
//! **Layout.** One file per tracked page, in app data and never inside the
//! graph: `<app data>/seen/<graph-id>/<sha256(page key)>.bin`, where
//! `<graph-id>` is the graph's session id stem (the drafts / session pattern)
//! and the page key is the page's identity key (`pageIdentityKey`). The file
//! is the magic `TINESEEN`, a little-endian `u32` format version (1), a
//! little-endian `u32` count, then `count` little-endian `u64` hashes, sorted
//! and unique. The hashes are opaque here: the frontend computes them
//! (`src/seen/hash.ts` is the one definition), this module only stores them.
//! Every write is `device_io::atomic_write` (temp + fsync + rename + directory
//! sync).
//!
//! **Bounds.** At most [`MAX_HASHES`] hashes (8 MiB), on read and on write.
//!
//! **Refusals.** A `mark` past the bound is refused and the old record stays
//! (scenario: malformed imported Markdown producing a page with millions of
//! blocks; the bound keeps the record readable by its own reader). A request
//! whose hash is not 16 hex digits, or whose page key is empty, is a decode
//! failure of the IPC payload (scenario: web content is an untrusted input
//! boundary, D-2b). A missing, unreadable, torn, foreign or oversized file is
//! never a refusal: `load` answers "no baseline" (scenario: crash or power loss
//! leaving a torn file, disk error, a sync client delivering another build's
//! file into app data). The record is disposable, so it is left in place and
//! the next `mark` replaces it; nothing is ever shown as an error dialog.
//!
//! **Cost.** `mark`: one file of 16 + 8 × distinct blocks bytes (24 B for a
//! 1-block page, 496 B for 60 blocks). Per edit: nothing. Transport: nothing.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tauri::Manager;

const MAGIC: &[u8; 8] = b"TINESEEN";
const VERSION: u32 = 1;
const HEADER: usize = 16;
pub(crate) const MAX_HASHES: usize = 1 << 20;

#[derive(Deserialize, Debug)]
#[serde(tag = "op", rename_all = "lowercase")]
pub(crate) enum SeenRequest {
    Load {
        graph: String,
        page: String,
    },
    Mark {
        graph: String,
        page: String,
        hashes: Vec<String>,
    },
    Forget {
        graph: String,
        page: String,
    },
}

fn encode(hashes: &[u64]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER + 8 * hashes.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&(hashes.len() as u32).to_le_bytes());
    for hash in hashes {
        bytes.extend_from_slice(&hash.to_le_bytes());
    }
    bytes
}

/// The stored hashes, or `None` for any file that is not exactly one record
/// of this format (torn, foreign, another version, past the bound).
fn decode(bytes: &[u8]) -> Option<Vec<u64>> {
    if bytes.len() < HEADER || &bytes[..8] != MAGIC {
        return None;
    }
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let count = word(12) as usize;
    if word(8) != VERSION || count > MAX_HASHES || bytes.len() != HEADER + 8 * count {
        return None;
    }
    Some(
        bytes[HEADER..]
            .chunks_exact(8)
            .map(|chunk| u64::from_le_bytes(chunk.try_into().unwrap()))
            .collect(),
    )
}

fn parse_hashes(hashes: &[String]) -> Result<Vec<u64>, String> {
    let mut parsed = hashes
        .iter()
        .map(|hash| {
            (hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| u64::from_str_radix(hash, 16).ok())
                .flatten()
                .ok_or_else(|| "a seen hash is 16 hex digits".to_string())
        })
        .collect::<Result<Vec<u64>, String>>()?;
    parsed.sort_unstable();
    parsed.dedup();
    if parsed.len() > MAX_HASHES {
        return Err(format!(
            "this page is too large to track ({} blocks; at most {MAX_HASHES})",
            parsed.len()
        ));
    }
    Ok(parsed)
}

pub(crate) fn load_at(path: &Path) -> Option<Vec<String>> {
    let bytes = crate::device_io::read_bounded(path, (HEADER + 8 * MAX_HASHES) as u64)
        .ok()
        .flatten()?;
    decode(&bytes).map(|hashes| hashes.iter().map(|h| format!("{h:016x}")).collect())
}

pub(crate) fn mark_at(path: &Path, hashes: &[String]) -> Result<(), String> {
    let bytes = encode(&parse_hashes(hashes)?);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    crate::device_io::atomic_write(path, &bytes).map_err(|e| e.to_string())
}

pub(crate) fn forget_at(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => path
            .parent()
            .map_or(
                Ok(()),
                tine_store::directory_durability::sync_directory_entry,
            )
            .map_err(|e| e.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

/// The record of page key `page` in the graph whose root is `graph`, under
/// the app-data directory `dir`.
fn record_path(dir: &Path, graph: &str, page: &str) -> Result<PathBuf, String> {
    if page.is_empty() {
        return Err("a seen record needs a page key".into());
    }
    let id = crate::settings::session_id(Path::new(graph));
    let stem = id.strip_suffix(".json").unwrap_or(&id);
    let name = Sha256::digest(page.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    Ok(dir.join("seen").join(stem).join(format!("{name}.bin")))
}

/// The request names its graph root (captured when the operation began, so a
/// graph switch while it is in flight cannot file it under another graph);
/// the caller must still be a bound graph window. The root only picks the
/// app-data file: nothing under the graph is read or written.
#[tauri::command]
pub(crate) async fn seen_baseline(
    request: SeenRequest,
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<Option<Vec<String>>, String> {
    crate::state::slot_for_context(&state)?;
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    crate::state::off_ui(move || match request {
        SeenRequest::Load { graph, page } => Ok(load_at(&record_path(&dir, &graph, &page)?)),
        SeenRequest::Mark {
            graph,
            page,
            hashes,
        } => mark_at(&record_path(&dir, &graph, &page)?, &hashes).map(|()| None),
        SeenRequest::Forget { graph, page } => {
            forget_at(&record_path(&dir, &graph, &page)?).map(|()| None)
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn hashes(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn a_mark_round_trips_sorted_unique_and_a_forget_removes_it() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("seen/g/p.bin");
        assert_eq!(load_at(&path), None, "no record is no baseline");
        mark_at(
            &path,
            &hashes(&["ffffffffffffffff", "0000000000000001", "0000000000000001"]),
        )
        .unwrap();
        assert_eq!(
            load_at(&path),
            Some(hashes(&["0000000000000001", "ffffffffffffffff"]))
        );
        mark_at(&path, &hashes(&["00000000000000aa"])).unwrap();
        assert_eq!(load_at(&path), Some(hashes(&["00000000000000aa"])));
        forget_at(&path).unwrap();
        assert!(!path.exists());
        forget_at(&path).unwrap();
        assert_eq!(load_at(&path), None);
    }

    #[test]
    fn unit_cost_is_sixteen_bytes_plus_eight_per_distinct_block() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("p.bin");
        mark_at(&path, &hashes(&["0123456789abcdef"])).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), 24);
        let sixty: Vec<String> = (0..60u64).map(|i| format!("{i:016x}")).collect();
        mark_at(&path, &sixty).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), 496);
        assert_eq!(
            fs::read_dir(temp.path()).unwrap().count(),
            1,
            "one file, no leftovers"
        );
    }

    #[test]
    fn a_torn_foreign_or_oversized_file_is_no_baseline_and_is_left_for_the_next_mark() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("p.bin");
        mark_at(&path, &hashes(&["0000000000000001", "0000000000000002"])).unwrap();
        let good = fs::read(&path).unwrap();
        let mut wrong_version = good.clone();
        wrong_version[8] = 9;
        let mut wrong_count = good.clone();
        wrong_count[12] = 3;
        for bytes in [
            good[..good.len() - 3].to_vec(),
            b"not a seen record".to_vec(),
            wrong_version,
            wrong_count,
            Vec::new(),
        ] {
            fs::write(&path, &bytes).unwrap();
            assert_eq!(load_at(&path), None, "{bytes:?}");
            assert_eq!(
                fs::read(&path).unwrap(),
                bytes,
                "the bytes are left as they were"
            );
        }
        let file = fs::File::create(&path).unwrap();
        file.set_len((HEADER + 8 * MAX_HASHES + 8) as u64).unwrap();
        drop(file);
        assert_eq!(load_at(&path), None);
        mark_at(&path, &hashes(&["0000000000000003"])).unwrap();
        assert_eq!(load_at(&path), Some(hashes(&["0000000000000003"])));
    }

    #[test]
    fn a_malformed_hash_or_a_page_past_the_bound_is_refused_and_keeps_the_old_record() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("p.bin");
        mark_at(&path, &hashes(&["0000000000000001"])).unwrap();
        for bad in [
            "12",
            "zzzzzzzzzzzzzzzz",
            "+000000000000001",
            "00000000000000010",
        ] {
            assert!(mark_at(&path, &hashes(&[bad])).is_err(), "{bad}");
        }
        let too_many: Vec<String> = (0..=MAX_HASHES as u64)
            .map(|i| format!("{i:016x}"))
            .collect();
        assert!(mark_at(&path, &too_many).is_err());
        assert_eq!(load_at(&path), Some(hashes(&["0000000000000001"])));
    }

    #[test]
    fn records_are_per_graph_and_per_page_key_and_never_under_the_graph() {
        let dir = Path::new("/app-data");
        let a = record_path(dir, "/home/u/graphs/notes", "plan").unwrap();
        let b = record_path(dir, "/home/u/other/notes", "plan").unwrap();
        let c = record_path(dir, "/home/u/graphs/notes", "other").unwrap();
        assert_ne!(a, b);
        assert_ne!(a, c);
        for path in [&a, &b, &c] {
            assert!(path.starts_with("/app-data/seen"), "{path:?}");
        }
        assert!(record_path(dir, "/home/u/graphs/notes", "").is_err());
    }

    #[test]
    fn the_request_decodes_from_the_frontend_shape() {
        let mark: SeenRequest = serde_json::from_str(
            r#"{"op":"mark","graph":"/g","page":"plan","hashes":["0000000000000001"]}"#,
        )
        .unwrap();
        assert!(matches!(mark, SeenRequest::Mark { ref hashes, .. } if hashes.len() == 1));
        let load: SeenRequest =
            serde_json::from_str(r#"{"op":"load","graph":"/g","page":"plan"}"#).unwrap();
        assert!(matches!(load, SeenRequest::Load { .. }));
        let forget: SeenRequest =
            serde_json::from_str(r#"{"op":"forget","graph":"/g","page":"plan"}"#).unwrap();
        assert!(matches!(forget, SeenRequest::Forget { .. }));
    }
}
