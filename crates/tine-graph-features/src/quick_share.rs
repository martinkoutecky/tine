//! Quick share, generation 1, writer side (`docs/contracts/quick-share.md`).
//!
//! `prepare` turns a reviewed export query into the gzip-compressed share
//! document `P`: the query's owner pages minus the export rules (`public::
//! false`, the exports page), projected as a published snapshot (schema 1)
//! from a **private build graph** that holds only those pages. Backlinks,
//! aliases, icons, block reference counts and baked queries are answered by
//! that graph alone, so no byte of an unshared page can reach the share
//! (closure). `seal` encrypts `P` into the envelope text uploaded as
//! `tine-share.bin`; `open` is its inverse, used by tests and fixtures.
//!
//! Nothing here writes the graph or reaches the network. The private build
//! graph is a copy of the selected pages under a caller-given scratch
//! directory, removed when `prepare` returns.

use crate::publish_query::{
    resolve_plan, snapshot_at, ExportPage, LeftOutPage, QueryExportRequest,
};
use crate::render;
use base64::Engine as _;
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use ring::hkdf;
use ring::rand::{SecureRandom, SystemRandom};
use serde::Serialize;
use std::io::{self, Read, Write};
use std::path::Path;
use tine_store::Store;

/// The generation this module writes: link path `/s/1/`, envelope version 1,
/// document `quick_share: 1`, snapshot schema 1.
pub const GENERATION: u32 = 1;
// Limits mirror `docs/contracts/quick-share-limits.json`, which the viewer
// reads; a unit test below keeps the two equal.
/// Pages in one share (the snapshot may add one home page listing them).
pub const MAX_PAGES: usize = 200;
/// Share document JSON, the gunzipped `P`.
pub const MAX_DOCUMENT_BYTES: usize = 32 * 1024 * 1024;
/// Compressed share document `P`.
pub const MAX_PAYLOAD_BYTES: usize = 6 * 1024 * 1024;
/// Envelope text (`tine-share.bin`).
pub const MAX_ENVELOPE_BYTES: usize = 8 * 1024 * 1024;

const MAGIC: &[u8; 4] = b"TSH1";
const ENVELOPE_VERSION: u8 = 1;
const HEADER_LEN: usize = 4 + 1 + 16;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const ENC_INFO: &[u8] = b"tine-share/1/enc";

/// What a share would carry, shown before anything is uploaded.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharePreview {
    pub include_comments: bool,
    pub title: String,
    pub pages: Vec<ExportPage>,
    pub left_out: Vec<LeftOutPage>,
    /// Bytes of `P` and of the envelope text it becomes.
    pub payload_bytes: usize,
    pub envelope_bytes: usize,
}

/// A prepared share: its preview and the exact `P` a later create uploads.
pub struct PreparedShare {
    pub preview: SharePreview,
    pub payload: Vec<u8>,
}

/// An envelope with the fresh secrets it was sealed under.
pub struct Sealed {
    pub envelope: String,
    pub key: [u8; 32],
    pub share_id: [u8; 16],
}

fn refusal(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

/// Canonical RFC 4648 §5 base64url without padding.
pub fn b64u(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Decode canonical base64url: padding, whitespace, foreign characters and
/// non-zero trailing bits are refused.
pub fn b64u_decode(text: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text)
        .ok()
}

/// `https://tine.page/s/1/#<login>/<gist_id>/<key>`.
pub fn link(login: &str, gist_id: &str, key: &[u8; 32]) -> String {
    format!(
        "https://tine.page/s/{GENERATION}/#{login}/{gist_id}/{}",
        b64u(key)
    )
}

fn cipher_key(key: &[u8; 32], share_id: &[u8; 16]) -> LessSafeKey {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, share_id).extract(key);
    let okm = prk
        .expand(&[ENC_INFO], &AES_256_GCM)
        .expect("HKDF-SHA256 yields 32 bytes");
    LessSafeKey::new(UnboundKey::from(okm))
}

fn header(share_id: &[u8; 16]) -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[..4].copy_from_slice(MAGIC);
    h[4] = ENVELOPE_VERSION;
    h[5..].copy_from_slice(share_id);
    h
}

/// Encrypt `payload` under the given secrets: `b64u(H || nonce || C)`.
/// Deterministic, for fixtures; production seals through `seal_fresh_as`.
pub fn seal(payload: &[u8], key: &[u8; 32], share_id: &[u8; 16], nonce: &[u8; 12]) -> String {
    let h = header(share_id);
    let mut sealed = payload.to_vec();
    cipher_key(key, share_id)
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(*nonce),
            Aad::from(&h),
            &mut sealed,
        )
        .expect("AES-256-GCM seals any payload under 64 GiB");
    let mut file = Vec::with_capacity(HEADER_LEN + NONCE_LEN + sealed.len());
    file.extend_from_slice(&h);
    file.extend_from_slice(nonce);
    file.extend_from_slice(&sealed);
    b64u(&file)
}

/// Re-share preserves the link key and authenticated share id, with a fresh
/// OS-random nonce for every complete snapshot.
pub fn seal_update(payload: &[u8], key: &[u8; 32], share_id: &[u8; 16]) -> io::Result<String> {
    let mut nonce = [0; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| io::Error::other("the system random source failed"))?;
    Ok(seal(payload, key, share_id, &nonce))
}

/// A fresh share id from the OS CSPRNG whose base64url spelling is never a
/// gist id (`[0-9a-f]{20,32}`): a pending marker `tine.share:: pending/<id>`
/// (D-20261018-QS3) then never reads as `<login>/<gist_id>`, even for a GitHub
/// login `pending`. An all-lowercase-hex 22-character spelling (chance
/// (16/64)^22) is drawn again; the id is not secret.
pub fn fresh_share_id() -> io::Result<[u8; 16]> {
    let rng = SystemRandom::new();
    loop {
        let mut share_id = [0u8; 16];
        rng.fill(&mut share_id)
            .map_err(|_| io::Error::other("the system random source failed"))?;
        if !looks_like_gist_id(&b64u(&share_id)) {
            return Ok(share_id);
        }
    }
}

/// Whether `text` has a gist id's shape, `[0-9a-f]{20,32}`.
pub fn looks_like_gist_id(text: &str) -> bool {
    (20..=32).contains(&text.len()) && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// [`seal`] under a fresh key, share id and nonce from the OS CSPRNG.
#[cfg(any(test, feature = "test-faults"))]
pub fn seal_fresh(payload: &[u8]) -> io::Result<Sealed> {
    seal_fresh_as(payload, fresh_share_id()?)
}

/// [`seal`] under the given share id (chosen at prepare, so the share dialog
/// can mark its query block before the upload) and a fresh key and nonce.
pub fn seal_fresh_as(payload: &[u8], share_id: [u8; 16]) -> io::Result<Sealed> {
    let rng = SystemRandom::new();
    let mut key = [0u8; 32];
    let mut nonce = [0u8; 12];
    for buf in [&mut key[..], &mut nonce[..]] {
        rng.fill(buf)
            .map_err(|_| io::Error::other("the system random source failed"))?;
    }
    let envelope = seal(payload, &key, &share_id, &nonce);
    Ok(Sealed {
        envelope,
        key,
        share_id,
    })
}

/// Why an envelope did not open. The viewer distinguishes the same cases.
#[derive(Debug, PartialEq, Eq)]
pub enum OpenError {
    TooLarge,
    NotBase64url,
    Truncated,
    NotAShare,
    NewerEnvelope(u8),
    /// Wrong key, or the file was changed.
    Unauthentic,
}

/// Decrypt an envelope to `P`, checking in the viewer's order.
pub fn open(envelope: &str, key: &[u8; 32]) -> Result<Vec<u8>, OpenError> {
    if envelope.len() > MAX_ENVELOPE_BYTES {
        return Err(OpenError::TooLarge);
    }
    let file = b64u_decode(envelope).ok_or(OpenError::NotBase64url)?;
    if file.len() < HEADER_LEN + NONCE_LEN + TAG_LEN {
        return Err(OpenError::Truncated);
    }
    if &file[..4] != MAGIC {
        return Err(OpenError::NotAShare);
    }
    if file[4] != ENVELOPE_VERSION {
        return Err(OpenError::NewerEnvelope(file[4]));
    }
    let share_id: [u8; 16] = file[5..HEADER_LEN].try_into().expect("16 bytes");
    let nonce: [u8; 12] = file[HEADER_LEN..HEADER_LEN + NONCE_LEN]
        .try_into()
        .expect("12 bytes");
    let mut sealed = file[HEADER_LEN + NONCE_LEN..].to_vec();
    let plain = cipher_key(key, &share_id)
        .open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(&file[..HEADER_LEN]),
            &mut sealed,
        )
        .map_err(|_| OpenError::Unauthentic)?;
    Ok(plain.to_vec())
}

/// Gunzip `P` with a running cap on the expanded size.
pub fn gunzip_bounded(payload: &[u8], max: usize) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    GzDecoder::new(payload)
        .take(max as u64 + 1)
        .read_to_end(&mut out)?;
    if out.len() > max {
        return Err(refusal("share document exceeds its expanded size limit"));
    }
    Ok(out)
}

/// The viewer's admission checks (`quick-share-limits.json` `viewer`), run on
/// the writer side so a share the viewer would refuse is refused at prepare.
pub fn validate_document(document: &[u8]) -> io::Result<()> {
    if document.len() > MAX_DOCUMENT_BYTES {
        return Err(refusal("the shared pages exceed the 32 MiB share limit"));
    }
    if !json_nesting_within(document, VIEWER_MAX_DEPTH) {
        return Err(refusal("the shared pages nest too deeply to share"));
    }
    // serde's default ceiling is 128 containers, and a block costs two (its
    // object and its `children` array), so the default would refuse pages the
    // viewer admits (a page 62 blocks deep). The scan above bounds the
    // recursion at the viewer's own depth instead.
    let value = {
        let mut reader = serde_json::Deserializer::from_slice(document);
        reader.disable_recursion_limit();
        serde::Deserialize::deserialize(&mut reader)
            .and_then(|value: serde_json::Value| reader.end().map(|()| value))
            .map_err(|_| refusal("the share document is not valid JSON"))?
    };
    let snapshot = &value["snapshot"];
    let count = |key: &str| snapshot[key].as_array().map_or(0, Vec::len);
    if count("pages") > VIEWER_MAX_PAGES {
        return Err(refusal("too many pages to share"));
    }
    if count("queries") > VIEWER_MAX_QUERIES {
        return Err(refusal("the shared pages hold too many queries"));
    }
    // A share of several pages adds a home page named after the share; it must
    // not shadow a shared page or alias, or links to that page open the list.
    let pages = snapshot["pages"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if let Some(home) = pages.iter().find(|p| p["path"] == "") {
        let key = tine_core::refs::page_key(home["name"].as_str().unwrap_or(""));
        let names = pages
            .iter()
            .filter(|p| p["path"] != "")
            .filter_map(|p| p["name"].as_str())
            .chain(
                snapshot["aliases"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|a| a[0].as_str()),
            );
        if let Some(clash) = names
            .into_iter()
            .find(|name| tine_core::refs::page_key(name) == key)
        {
            return Err(refusal(format!(
                "a shared page is already called \"{clash}\"; give the share another name"
            )));
        }
    }
    // Bounded by `json_nesting_within` above.
    fn walk(v: &serde_json::Value) -> Result<(), &'static str> {
        match v {
            serde_json::Value::String(s) if s.len() > VIEWER_MAX_STRING_BYTES => {
                Err("a shared block is larger than 1 MiB")
            }
            serde_json::Value::Array(items) => items.iter().try_for_each(walk),
            serde_json::Value::Object(map) => map.values().try_for_each(walk),
            _ => Ok(()),
        }
    }
    walk(&value).map_err(refusal)?;
    fn count_blocks(v: &serde_json::Value) -> usize {
        v.as_array().map_or(0, |items| {
            items.iter().map(|b| 1 + count_blocks(&b["children"])).sum()
        })
    }
    let total: usize = snapshot["pages"].as_array().map_or(0, |pages| {
        pages.iter().map(|p| count_blocks(&p["blocks"])).sum()
    });
    if total > VIEWER_MAX_BLOCKS {
        return Err(refusal("the shared pages hold too many blocks"));
    }
    Ok(())
}

/// Whether `json` holds at most `max` containers inside one another, counted
/// without recursion (the viewer's "JSON depth"). Strings are skipped, so a
/// bracket in text does not count.
fn json_nesting_within(json: &[u8], max: usize) -> bool {
    let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
    for &byte in json {
        if quoted {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => quoted = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => quoted = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > max {
                    return false;
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    true
}

const VIEWER_MAX_PAGES: usize = 201;
const VIEWER_MAX_QUERIES: usize = 256;
const VIEWER_MAX_DEPTH: usize = 512;
const VIEWER_MAX_STRING_BYTES: usize = 1024 * 1024;
const VIEWER_MAX_BLOCKS: usize = 200_000;

/// Prepare a share of the pages `request` selects, stamped `created_at`
/// (RFC 3339 UTC). `scratch` is a directory the private build graph may be
/// created under (app cache); it is removed before returning. The
/// request's `include_query_text` and destination fields are ignored: a share
/// never carries its selecting query.
pub fn prepare(
    store: &Store,
    request: &QueryExportRequest,
    created_at: &str,
    scratch: &Path,
) -> io::Result<PreparedShare> {
    let graph = store
        .whole_graph()
        .map_err(|error| io::Error::other(format!("{error:?}")))?;
    let mut request = request.clone();
    request.include_query_text = false;
    let planned = resolve_plan(store, &graph, &request)?;
    let selected = &planned.selected;
    if selected.pages.is_empty() {
        return Err(refusal("this query selects no pages to share"));
    }
    if selected.pages.len() > MAX_PAGES {
        return Err(refusal(format!(
            "a share holds at most {MAX_PAGES} pages; this query selects {}",
            selected.pages.len()
        )));
    }

    // The private build graph: the selected documents as they were reviewed,
    // plus the graph's parse configuration (journal formats, property rules).
    let (title, snapshot) = crate::build_graph::with_build_graph(
        store,
        &selected.pages,
        scratch,
        "the shared pages did not read back as the same pages; nothing was shared",
        |private_graph, meta, mut corpus| {
            // The private store lists its pages in directory order, which differs
            // by filesystem (hash order on ext4, name order on NTFS). The share
            // lists them in the order the review showed: by path, as the plan.
            corpus
                .pages
                .sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
            // The export rules hold for the bytes actually shared, not only for
            // the whole-graph view the plan was made from.
            if corpus
                .pages
                .iter()
                .any(|page| render::export_left_out(page).is_some())
            {
                return Err(refusal(
                    "a shared page is private or the exports page; nothing was shared",
                ));
            }
            let (title, home, home_query) = if corpus.pages.len() == 1 {
                let name = corpus.pages[0].name.clone();
                (name.clone(), name, None)
            } else {
                let title = request.name.trim().to_owned();
                // The synthetic home must not shadow a name the build graph
                // answers (a shared page or journal, an alias of one or of
                // several shared pages): its links would open the list.
                // Threat: malformed input, a share name colliding with a
                // shared name. Recovery: the user names the share otherwise.
                let inventory = private_graph.inventory(tine_store::InventoryScope::All);
                let spellings = private_graph
                    .inventory(tine_store::InventoryScope::PublishedV1Spellings(&inventory));
                if crate::published_answers::home_name_taken(&inventory, &spellings, &title) {
                    return Err(refusal(format!(
                        "a shared page or alias is already called \"{title}\"; give the share another name"
                    )));
                }
                (
                    title.clone(),
                    title,
                    Some((&request, &planned.parsed, &planned.result)),
                )
            };
            let snapshot = snapshot_at(
                private_graph,
                meta,
                &corpus,
                &title,
                &home,
                home_query,
                created_at,
            )?;
            Ok((title, snapshot))
        },
    )?;

    let mut document = Vec::with_capacity(snapshot.len() + 128);
    document.extend_from_slice(
        format!(
            "{{\"quick_share\":{GENERATION},\"created_at\":{},\"title\":{},\"snapshot\":",
            serde_json::to_string(created_at).map_err(io::Error::other)?,
            serde_json::to_string(&title).map_err(io::Error::other)?,
        )
        .as_bytes(),
    );
    document.extend_from_slice(&snapshot);
    document.push(b'}');
    validate_document(&document)?;
    let mut gz = GzEncoder::new(Vec::new(), Compression::new(9));
    gz.write_all(&document)?;
    let payload = gz.finish()?;
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(refusal("the compressed share exceeds its 6 MiB limit"));
    }
    // Unpadded base64url of `H || nonce || C`.
    let envelope_bytes = ((HEADER_LEN + NONCE_LEN + payload.len() + TAG_LEN) * 4).div_ceil(3);
    if envelope_bytes > MAX_ENVELOPE_BYTES {
        return Err(refusal("the encrypted share exceeds its 8 MiB limit"));
    }
    Ok(PreparedShare {
        preview: SharePreview {
            include_comments: planned.plan.include_comments,
            title,
            pages: planned.plan.pages.clone(),
            left_out: planned.plan.left_out.clone(),
            payload_bytes: payload.len(),
            envelope_bytes,
        },
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page's blocks cost two containers a level (the block and its
    /// `children`), so the viewer's 512 admits a page 253 blocks deep. Parsed
    /// on the default test stack in a debug build, the worst case for the
    /// recursion the depth scan bounds.
    #[test]
    fn a_document_nests_as_deep_as_the_viewer_admits() {
        let document = |levels: usize| {
            let mut blocks = String::from("[]");
            for _ in 0..levels {
                blocks = format!("[{{\"children\":{blocks}}}]");
            }
            format!("{{\"snapshot\":{{\"pages\":[{{\"path\":\"p\",\"blocks\":{blocks}}}]}}}}")
        };
        for levels in [1, 62, 63, 100, 253] {
            validate_document(document(levels).as_bytes())
                .unwrap_or_else(|e| panic!("{levels} levels: {e}"));
        }
        let refused = validate_document(document(254).as_bytes()).unwrap_err();
        assert!(refused.to_string().contains("nest too deeply"), "{refused}");
        let arrays = |depth: usize| format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        validate_document(arrays(VIEWER_MAX_DEPTH).as_bytes()).unwrap();
        let refused = validate_document(arrays(VIEWER_MAX_DEPTH + 1).as_bytes()).unwrap_err();
        assert!(refused.to_string().contains("nest too deeply"), "{refused}");
        // The scan counts containers, not brackets inside text.
        assert!(json_nesting_within(br#"{"a":"[[[[\"[[["}"#, 1));
        assert!(!json_nesting_within(br#"{"a":[{"b":1}]}"#, 2));
        assert!(json_nesting_within(br#"{"a":[{"b":1}]}"#, 3));
    }

    #[test]
    fn limits_match_the_contract_file() {
        let limits: serde_json::Value = serde_json::from_str(include_str!(
            "../../../docs/contracts/quick-share-limits.json"
        ))
        .unwrap();
        let w = &limits["writer"];
        let v = &limits["viewer"];
        assert_eq!(limits["generation"], GENERATION);
        assert_eq!(w["pages"], MAX_PAGES);
        assert_eq!(w["document_bytes"], MAX_DOCUMENT_BYTES);
        assert_eq!(w["payload_bytes"], MAX_PAYLOAD_BYTES);
        assert_eq!(w["envelope_bytes"], MAX_ENVELOPE_BYTES);
        assert_eq!(v["pages"], VIEWER_MAX_PAGES);
        assert_eq!(v["queries"], VIEWER_MAX_QUERIES);
        assert_eq!(v["depth"], VIEWER_MAX_DEPTH);
        assert_eq!(v["string_bytes"], VIEWER_MAX_STRING_BYTES);
        assert_eq!(v["blocks"], VIEWER_MAX_BLOCKS);
        assert_eq!(v["document_bytes"], MAX_DOCUMENT_BYTES);
        // The viewer admits whatever the writer may produce.
        assert!(v["envelope_text_bytes"].as_u64().unwrap() >= MAX_ENVELOPE_BYTES as u64);
        assert!(v["envelope_decoded_bytes"].as_u64().unwrap() >= (MAX_PAYLOAD_BYTES + 49) as u64);
    }
}
