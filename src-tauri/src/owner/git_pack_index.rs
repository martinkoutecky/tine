//! Read-only loose-object census and SHA-1 pack-index decoding for repacking.
use git2::Oid;
use std::collections::HashSet;
use std::path::Path;

pub(super) fn loose_object_count(objects: &Path) -> usize {
    let mut count = 0;
    for byte in 0..=255u8 {
        if let Ok(dir) = std::fs::read_dir(objects.join(format!("{byte:02x}"))) {
            count += dir.count();
        }
    }
    count
}

/// The ids of the loose objects under `objects`.
pub(super) fn loose_objects(objects: &Path) -> Vec<Oid> {
    let mut out = Vec::new();
    for byte in 0..=255u8 {
        let hex = format!("{byte:02x}");
        let Ok(entries) = std::fs::read_dir(objects.join(&hex)) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(rest) = name.to_str().filter(|r| r.len() == 38) else {
                continue;
            };
            if let Ok(oid) = Oid::from_str(&format!("{hex}{rest}")) {
                out.push(oid);
            }
        }
    }
    out
}

/// The object ids a version-2 SHA-1 pack index lists.
pub(super) fn idx_oids(idx: &[u8]) -> Result<HashSet<[u8; 20]>, String> {
    const HEADER: &[u8] = b"\xfftOc\0\0\0\x02";
    let bad = || "not a version-2 pack index".to_string();
    if idx.len() < 8 + 1024 || &idx[..8] != HEADER {
        return Err(bad());
    }
    let fanout = |i: usize| {
        let at = 8 + i * 4;
        u32::from_be_bytes([idx[at], idx[at + 1], idx[at + 2], idx[at + 3]]) as usize
    };
    let count = fanout(255);
    let start = 8 + 1024;
    let end = start + count * 20;
    // Ids, CRCs, offsets, then two 20-byte trailers at least.
    if idx.len() < end + count * 8 + 40 {
        return Err(bad());
    }
    Ok(idx[start..end]
        .chunks_exact(20)
        .map(|c| c.try_into().expect("20 bytes"))
        .collect())
}
