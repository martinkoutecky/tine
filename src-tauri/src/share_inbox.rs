//! Share inbox (ADR 0073): what the OS share sheet, an App Intent or an
//! Android `ACTION_SEND` hands to Tine waits here, in storage the app owns
//! (iOS: the App Group container; Android: the app's private files dir), and
//! never in the graph. Native code only ever writes this inbox. The running
//! app ingests each item through the single graph writer
//! (`appendToTodayJournal`, src/shareIngest.ts) and only then commits
//! (removes) it, so a share is never lost to a failed or interrupted save.
//!
//! Layout under the inbox root, one directory per item:
//! - `<id>/item.json` + the item's resource files: written by the native
//!   producer into `.tmp-<id>/`, then published by one directory rename;
//! - `<id>/prepared.json`: written here, through the audited
//!   [`crate::device_io::atomic_write`], by the frontend's ingest: the graph
//!   and journal day the item is bound to, its shaped Markdown and imported
//!   assets, and (once an append is armed) the journal's revision and match
//!   count just before it, which make recovery after a crash loss-free
//!   (see `src/shareIngest.ts` and ADR 0073);
//! - `.trash-<id>/`: a committed item between its rename and its removal;
//! - `.rejected-<id>/`: an item that could not be read, kept for the user.
//!
//! Refusals (ADR 0073 refusal table; the threat model is
//! specs/notes/2026-08-07-trust-model-and-threat-model-decision.md):
//! - an undecodable or oversized `item.json`, a resource name that is not a
//!   plain file name, or a missing resource file: a producer interrupted by a
//!   crash or power loss on a file system without atomic directory renames,
//!   or a disk error. The item is moved aside to `.rejected-<id>`, never
//!   deleted, and the frontend reports it;
//! - an undecodable `prepared.json`: a disk error (the write is atomic). The
//!   item is moved aside the same way, because re-preparing an item whose
//!   journal write may already have landed could duplicate it;
//! - a stale `.tmp-<id>` older than [`STALE_TMP`]: a producer that crashed
//!   mid-write. It never became an item, and the user saw no confirmation.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const ITEM_FILE: &str = "item.json";
const PREPARED_FILE: &str = "prepared.json";
const MAX_ITEM_BYTES: u64 = 1024 * 1024;
const MAX_PREPARED_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RESOURCES: usize = 32;
/// A producer finishes an item in well under a second; one that is still a
/// `.tmp-` directory after a day was interrupted.
const STALE_TMP: Duration = Duration::from_secs(24 * 60 * 60);

/// One resource as the producer described it (`SharedResource` in OG's
/// `ios/App/ShareViewController/SharedData.swift`).
#[derive(Debug, Deserialize)]
struct ItemResource {
    /// File name inside the item directory.
    file: String,
    /// Display name, e.g. the original file name.
    #[serde(default)]
    name: Option<String>,
    /// MIME type.
    #[serde(default, rename = "type")]
    mime: Option<String>,
}

/// `item.json` as the native producers write it (ADR 0073).
#[derive(Debug, Deserialize)]
struct ItemFile {
    version: u32,
    /// Which OG share path the item follows: `android` (the legacy
    /// `SendIntent` result, `intent.cljs` `handle-result`) or `ios` (the
    /// share-sheet payload, `handle-payload`).
    source: String,
    #[serde(default)]
    created: Option<i64>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    resources: Vec<ItemResource>,
}

/// The ingest's durable record for one item (ADR 0073), written before any
/// graph write it describes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Prepared {
    /// The bound graph (its root) the item is ingested into; another graph
    /// never receives or acknowledges it.
    pub graph: String,
    /// The journal title the append targets, frozen once.
    pub day: String,
    /// The shaped outline Markdown, asset links resolved; `None` while the
    /// item's files are being imported.
    pub markdown: Option<String>,
    /// Asset file names imported into `graph` for this item.
    pub assets: Vec<String>,
    /// Set inside the admitted read just before the append.
    pub armed: Option<Armed>,
}

/// The journal as the append found it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Armed {
    /// The day file's revision (`None`: the day had no file).
    pub before: Option<String>,
    /// Blocks on the day equal to the appended block.
    pub matches: u32,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResourceWire {
    /// Absolute path of the resource file, for `import_asset`.
    path: String,
    name: Option<String>,
    #[serde(rename = "type")]
    mime: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ItemWire {
    id: String,
    source: String,
    created: Option<i64>,
    text: Option<String>,
    title: Option<String>,
    url: Option<String>,
    resources: Vec<ResourceWire>,
    prepared: Option<Prepared>,
}

#[derive(Debug, Default, Serialize)]
pub(crate) struct Listing {
    items: Vec<ItemWire>,
    /// Items moved aside to `.rejected-<id>` by this listing.
    rejected: usize,
}

/// An item id: what the producers mint (a UUID), and nothing that could name
/// another directory.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A resource name must be a plain file name inside its item directory.
fn valid_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name != "."
        && name != ".."
        && name != ITEM_FILE
        && name != PREPARED_FILE
        && !name.contains(['/', '\\', '\0'])
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path, max: u64) -> Result<T, String> {
    let bytes = crate::device_io::read_bounded(path, max)
        .map_err(|error| format!("{}: {error}", path.display()))?
        .ok_or_else(|| format!("{}: larger than {max} bytes", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))
}

fn read_item(dir: &Path, id: &str) -> Result<ItemWire, String> {
    let item: ItemFile = read_json(&dir.join(ITEM_FILE), MAX_ITEM_BYTES)?;
    if item.version != 1 {
        return Err(format!("unknown item version {}", item.version));
    }
    if item.source != "android" && item.source != "ios" {
        return Err(format!("unknown item source {:?}", item.source));
    }
    if item.resources.len() > MAX_RESOURCES {
        return Err(format!("more than {MAX_RESOURCES} resources"));
    }
    let mut resources = Vec::with_capacity(item.resources.len());
    for resource in item.resources {
        if !valid_file_name(&resource.file) {
            return Err(format!(
                "resource name {:?} is not a file name",
                resource.file
            ));
        }
        let path = dir.join(&resource.file);
        if !std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_file()) {
            return Err(format!("resource {:?} is missing", resource.file));
        }
        resources.push(ResourceWire {
            path: path.display().to_string(),
            name: resource.name,
            mime: resource.mime,
        });
    }
    let prepared_path = dir.join(PREPARED_FILE);
    let prepared = match std::fs::symlink_metadata(&prepared_path) {
        Ok(_) => Some(read_json::<Prepared>(&prepared_path, MAX_PREPARED_BYTES)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("{}: {error}", prepared_path.display())),
    };
    let text = item.text.filter(|text| !text.trim().is_empty());
    let url = item.url.filter(|url| !url.trim().is_empty());
    if text.is_none() && url.is_none() && resources.is_empty() {
        return Err("the item carries no text, link or file".into());
    }
    Ok(ItemWire {
        id: id.to_owned(),
        source: item.source,
        created: item.created,
        text,
        title: item.title.filter(|title| !title.trim().is_empty()),
        url,
        resources,
        prepared,
    })
}

fn older_than(path: &Path, age: Duration) -> bool {
    std::fs::symlink_metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|elapsed| elapsed > age)
}

/// Move an unreadable item aside, keeping its bytes for the user.
fn set_aside(root: &Path, id: &str) {
    let to = root.join(format!(".rejected-{id}"));
    if let Err(error) = crate::device_io::publish_directory_entry(&root.join(id), &to) {
        crate::debug::diag_private("share-inbox-set-aside-failed", &format!("{id}: {error}"));
    }
}

/// Every complete item, oldest first, after sweeping interrupted producer
/// directories and removing committed ones. Cost: O(items + resources)
/// metadata reads plus each `item.json`/`prepared.json`.
pub(crate) fn list(root: &Path) -> Result<Listing, String> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Listing::default()),
        Err(error) => return Err(format!("{}: {error}", root.display())),
    };
    let mut listing = Listing::default();
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", root.display()))?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let path = entry.path();
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        if name.starts_with(".trash-") {
            let _ = std::fs::remove_dir_all(&path);
            continue;
        }
        if name.starts_with(".tmp-") {
            if older_than(&path, STALE_TMP) {
                let _ = std::fs::remove_dir_all(&path);
            }
            continue;
        }
        if name.starts_with('.') || !valid_id(&name) {
            continue;
        }
        match read_item(&path, &name) {
            Ok(item) => listing.items.push(item),
            Err(error) => {
                crate::debug::diag_private(
                    "share-inbox-item-rejected",
                    &format!("{name}: {error}"),
                );
                set_aside(root, &name);
                listing.rejected += 1;
            }
        }
    }
    listing
        .items
        .sort_by(|a, b| (a.created, &a.id).cmp(&(b.created, &b.id)));
    Ok(listing)
}

fn item_dir(root: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err(format!("not a share inbox item id: {id:?}"));
    }
    Ok(root.join(id))
}

/// Durably record the frontend's shaping of item `id`.
pub(crate) fn prepare(root: &Path, id: &str, prepared: &Prepared) -> Result<(), String> {
    let dir = item_dir(root, id)?;
    if !dir.join(ITEM_FILE).is_file() {
        return Err(format!("share inbox item {id} is gone"));
    }
    let bytes = serde_json::to_vec(prepared).map_err(|error| error.to_string())?;
    crate::device_io::atomic_write(&dir.join(PREPARED_FILE), &bytes)
        .map_err(|error| format!("share inbox item {id}: {error}"))
}

/// Remove item `id` after its journal write reached disk. Idempotent: an
/// item that is already gone is committed.
pub(crate) fn commit(root: &Path, id: &str) -> Result<(), String> {
    let dir = item_dir(root, id)?;
    let trash = root.join(format!(".trash-{id}"));
    if trash.exists() {
        std::fs::remove_dir_all(&trash).map_err(|error| error.to_string())?;
    }
    match crate::device_io::publish_directory_entry(&dir, &trash) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("share inbox item {id}: {error}")),
    }
    // The rename is the commit; a failed removal is retried by the next list.
    let _ = std::fs::remove_dir_all(&trash);
    Ok(())
}

// ---- Platform split: every shipped target is named. ----

/// Mobile: the inbox the native producers write (native_integrations.rs).
#[cfg(any(target_os = "ios", target_os = "android"))]
fn inbox_root(app: &tauri::AppHandle) -> Result<Option<PathBuf>, String> {
    crate::native_integrations::inbox_root(app).map(Some)
}

/// Desktop has no share sheet producer; its inbox is always empty.
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
fn inbox_root(_app: &tauri::AppHandle) -> Result<Option<PathBuf>, String> {
    Ok(None)
}

#[tauri::command]
pub(crate) async fn share_inbox_list(app: tauri::AppHandle) -> Result<Listing, String> {
    crate::state::off_ui(move || match inbox_root(&app)? {
        Some(root) => list(&root),
        None => Ok(Listing::default()),
    })
    .await
}

#[tauri::command]
pub(crate) async fn share_inbox_prepare(
    app: tauri::AppHandle,
    id: String,
    prepared: Prepared,
) -> Result<(), String> {
    crate::state::off_ui(move || {
        let root = inbox_root(&app)?.ok_or("this platform has no share inbox")?;
        prepare(&root, &id, &prepared)
    })
    .await
}

#[tauri::command]
pub(crate) async fn share_inbox_commit(app: tauri::AppHandle, id: String) -> Result<(), String> {
    crate::state::off_ui(move || {
        let root = inbox_root(&app)?.ok_or("this platform has no share inbox")?;
        commit(&root, &id)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn publish(root: &Path, id: &str, item: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let tmp = root.join(format!(".tmp-{id}"));
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join(ITEM_FILE), item).unwrap();
        for (name, bytes) in files {
            fs::write(tmp.join(name), bytes).unwrap();
        }
        let dir = root.join(id);
        fs::rename(&tmp, &dir).unwrap();
        dir
    }

    #[test]
    fn lists_complete_items_oldest_first_with_resource_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        publish(
            root,
            "b",
            r#"{"version":1,"source":"android","created":2,"text":"second"}"#,
            &[],
        );
        let dir = publish(
            root,
            "a",
            r#"{"version":1,"source":"android","created":1,"url":"https://x.org","title":"X",
                "resources":[{"file":"p.png","name":"p.png","type":"image/png"}]}"#,
            &[("p.png", b"png")],
        );
        let listing = list(root).unwrap();
        assert_eq!(listing.rejected, 0);
        let ids: Vec<_> = listing.items.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
        let first = &listing.items[0];
        assert_eq!(first.url.as_deref(), Some("https://x.org"));
        assert_eq!(first.title.as_deref(), Some("X"));
        assert_eq!(
            first.resources[0].path,
            dir.join("p.png").display().to_string()
        );
        assert_eq!(first.resources[0].mime.as_deref(), Some("image/png"));
        assert!(first.prepared.is_none());
    }

    #[test]
    fn a_missing_inbox_is_empty() {
        let temp = tempfile::tempdir().unwrap();
        let listing = list(&temp.path().join("absent")).unwrap();
        assert!(listing.items.is_empty());
    }

    #[test]
    fn an_unfinished_producer_directory_is_not_an_item_and_is_swept_only_when_stale() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let tmp = root.join(".tmp-x");
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join(ITEM_FILE), "{\"vers").unwrap();
        assert!(list(root).unwrap().items.is_empty());
        assert!(tmp.exists(), "a producer may still be writing it");
        let old = SystemTime::now() - STALE_TMP - Duration::from_secs(60);
        fs::File::options()
            .write(true)
            .open(&tmp)
            .or_else(|_| fs::File::open(&tmp))
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(list(root).unwrap().items.is_empty());
        assert!(
            !tmp.exists(),
            "a day-old producer directory was interrupted"
        );
    }

    /// Refusal: an unreadable item (interrupted or damaged producer write) is
    /// kept aside, never deleted, and counted for the user.
    #[test]
    fn unreadable_items_are_set_aside_not_deleted() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        publish(root, "torn", "{\"version\":1,\"te", &[]);
        publish(
            root,
            "escape",
            r#"{"version":1,"source":"android","resources":[{"file":"../x"}]}"#,
            &[],
        );
        publish(
            root,
            "missing",
            r#"{"version":1,"source":"android","resources":[{"file":"gone.png"}]}"#,
            &[],
        );
        publish(root, "empty", r#"{"version":1,"source":"android","text":"  "}"#, &[]);
        publish(root, "nosource", r#"{"version":1,"text":"t"}"#, &[]);
        publish(root, "badsource", r#"{"version":1,"source":"web","text":"t"}"#, &[]);
        publish(root, "ok", r#"{"version":1,"source":"android","text":"kept"}"#, &[]);
        let listing = list(root).unwrap();
        assert_eq!(listing.rejected, 6);
        assert_eq!(listing.items.len(), 1);
        for id in ["torn", "escape", "missing", "empty", "nosource", "badsource"] {
            assert!(root
                .join(format!(".rejected-{id}"))
                .join(ITEM_FILE)
                .is_file());
            assert!(!root.join(id).exists());
        }
        assert_eq!(list(root).unwrap().rejected, 0, "set aside once");
    }

    #[test]
    fn prepare_is_durable_and_listed_back() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        publish(root, "a", r#"{"version":1,"source":"android","text":"t"}"#, &[]);
        let prepared = Prepared {
            graph: "/g".into(),
            day: "Oct 10th, 2026".into(),
            markdown: Some("- t".into()),
            assets: vec!["p_1.png".into()],
            armed: Some(Armed {
                before: None,
                matches: 2,
            }),
        };
        prepare(root, "a", &prepared).unwrap();
        assert_eq!(
            list(root).unwrap().items[0].prepared.as_ref(),
            Some(&prepared)
        );
        assert!(prepare(root, "gone", &prepared).is_err());
        assert!(prepare(root, "../a", &prepared).is_err());
    }

    #[test]
    fn commit_removes_the_item_and_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        publish(root, "a", r#"{"version":1,"source":"android","text":"t"}"#, &[("f", b"x")]);
        commit(root, "a").unwrap();
        assert!(!root.join("a").exists());
        assert!(!root.join(".trash-a").exists());
        commit(root, "a").unwrap();
        assert!(list(root).unwrap().items.is_empty());
        assert!(commit(root, "../etc").is_err());
    }

    #[test]
    fn a_committed_item_left_by_a_crash_is_removed_and_never_listed() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let dir = publish(root, "a", r#"{"version":1,"source":"android","text":"t"}"#, &[]);
        fs::rename(dir, root.join(".trash-a")).unwrap();
        assert!(list(root).unwrap().items.is_empty());
        assert!(!root.join(".trash-a").exists());
    }

    /// AGENTS.md section 2: a platform `cfg` list names every shipped target.
    #[test]
    fn every_shipped_target_is_named_exactly_once() {
        let source = include_str!("share_inbox.rs");
        let production = source.split("#[cfg(test)]").next().unwrap();
        let mut named: Vec<&str> = production
            .lines()
            .filter(|line| line.trim_start().starts_with("#[cfg(any(target_os"))
            .flat_map(|line| line.split('"').skip(1).step_by(2))
            .collect();
        named.sort_unstable();
        assert_eq!(
            named,
            ["android", "ios", "linux", "macos", "windows"],
            "the inbox platform split must name Linux, Windows, macOS, iOS and Android exactly \
             once (AGENTS.md section 2; exemplar defender.rs)"
        );
    }
}
