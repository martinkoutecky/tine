//! Replace the graph's live page, journal, and asset-sidecar set from backup files.

use crate::store::{Area, FileId, GraphRev, Store};
use crate::IoError;
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const ASSET_RECOVERY: &str = ".tine-restore-recovery";
static RECOVERY_SEQ: AtomicU64 = AtomicU64::new(0);
static COPY_SEQ: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "test-faults")]
static RESTORE_BOUNDARY: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "test-faults")]
fn restore_abort_boundary() {
    let index = RESTORE_BOUNDARY.fetch_add(1, Ordering::Relaxed);
    if std::env::var("TINE_RESTORE_ABORT_BOUNDARY")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        == Some(index)
    {
        std::process::abort();
    }
}

/// One caller-supplied snapshot file. `rel` is relative to `area`; `bytes`
/// are the content the caller already verified against its backup (restore
/// writes exactly these bytes and never re-reads a backup).
pub struct RestoreFile {
    /// Destination graph area.
    pub area: Area,
    /// Destination name relative to `area`.
    pub rel: String,
    /// The verified content to write.
    pub bytes: Vec<u8>,
}

/// Completed work and recovery locations, including after a partial failure.
#[derive(Debug)]
pub struct RestoreReport {
    /// Number of input files copied into the graph. A test oracle (SURFACE.txt
    /// rule 4): the app reports recovery locations, never this count.
    #[cfg(any(test, feature = "test-faults"))]
    pub restored: u64,
    /// Same-filesystem recovery directories holding retired files.
    pub recovery: Vec<PathBuf>,
    /// Live targets left in place when no-replace copying found a concurrent
    /// target. Its bytes need not differ from the restore baseline.
    pub kept_external: Vec<FileId>,
    /// Revision published for a changed disk state, or the current revision
    /// if restore made no change. During a failed initial load this is the
    /// unchanged current revision even if restore wrote files: no view covers
    /// those writes until recovery's first view does.
    pub graph_rev: GraphRev,
}

/// A restore stopped at `phase`; `done` describes work already completed.
#[derive(Debug)]
pub struct RestoreFailed {
    /// Human-readable phase description. This is not a stable enum or a value
    /// suitable for programmatic branching.
    pub phase: String,
    /// Error that stopped the restore.
    pub cause: IoError,
    /// Work completed before the failure; recovery locations remain available.
    pub done: RestoreReport,
}

struct Recovery {
    root_path: PathBuf,
    root: Dir,
    dir: Dir,
    path: PathBuf,
}

fn fail(phase: &str, cause: io::Error, done: RestoreReport) -> RestoreFailed {
    RestoreFailed {
        phase: phase.into(),
        cause: cause.into(),
        done,
    }
}

impl Store {
    /// Restore page and journal text, asset `.edn` sidecars, and config from
    /// open files whose supplied lengths are checked before copying. The
    /// caller is responsible for stronger source verification. The input is
    /// the complete desired set in those
    /// areas: every unlisted live page, journal, and asset sidecar is retired
    /// into same-filesystem recovery roots. Graph files are retired under
    /// `logseq/.tine-trash/<restore-id>`; asset sidecars under
    /// `assets/.tine-restore-recovery/<restore-id>`. The returned `recovery`
    /// paths locate them; the store has no restore-import or cleanup call.
    /// Passing only one page retires every other live page and journal; use a
    /// transaction to replace one file. The app passes every file from a
    /// verified whole-graph backup snapshot. Hidden asset recovery folders
    /// are omitted by `scan_area`, but their reported paths can be inspected
    /// with ordinary filesystem access or raw `Store::read` file ids.
    /// Replaced files are retired too. An unlisted `config.edn` and
    /// `custom.css` stay live. Only `config.edn` is accepted in the Meta area;
    /// Trash targets and non-`.edn` assets are refused. A supplied config is
    /// refused if it exceeds 64 MiB, is not UTF-8, fails the same parse-input
    /// admission as graph load, names unsafe managed directories, or changes
    /// the current pages/journals directories: this restore places
    /// files in the current directories. Any `.edn` file under
    /// assets counts as a sidecar, regardless of a matching PDF. The store
    /// copies sidecar bytes and supplies no EDN parser or sidecar schema.
    /// Other asset files are left in place. The method then copies new files
    /// with a no-replace publish. If another writer creates a target after it
    /// was retired, that live file is kept, its id is the only entry in
    /// `kept_external`, and restore stops with `RestoreFailed`: later inputs are
    /// not copied and later extras are not retired. `kept_external` is non-empty
    /// only on failure. An empty or text-free input is accepted and retires all
    /// live pages and journals; the safety snapshot and recovery roots retain
    /// their prior bytes. It blocks saves and transactions
    /// for the full operation. Cost includes all input bytes, all live page,
    /// journal, and sidecar bytes hashed for baseline and publication, and an
    /// asset-tree walk excluding earlier `.tine-restore-recovery` sidecars,
    /// and a whole-graph reparse under the writer lock, even
    /// for a small input. The file lists and baseline stamps retain O(live +
    /// input file count) memory; payloads are copied through files. A changed
    /// restore publishes one
    /// `Origin::Own` revision for the final disk state, including `Removed`
    /// tuples for retired live files and a `logseq/config.edn` file tuple when
    /// config changed.
    /// The config is reloaded before the resulting view is published. A changed
    /// partial result on failure publishes the final disk state after a
    /// successful initial load. After a failed initial load, writes remain
    /// guarded but publication waits for successful `refresh()` recovery.
    /// An in-flight save holding the writer lock
    /// finishes before this restore; a later save checks against restored
    /// bytes. After any failure, check `done.recovery` and
    /// `done.kept_external` when reconciling disk state; a retry retires the
    /// partial result again. Existing `WholeGraph` views
    /// remain captured snapshots until the final publication. `page()` and the
    /// watcher wait for the writer lock; `scan_area()` can observe intermediate
    /// files because it reads disk without that lock.
    /// A crash can leave a partial restore with whole individual files and
    /// recovery directories; there is no store import or cleanup call. Each
    /// retired entry, and every recovery directory created for it, is synced
    /// before any replacement is published, so power loss cannot keep the
    /// replacement while losing the retired original (one directory sync per
    /// retired file plus one per created directory).
    /// An editor must separately
    /// preserve its unsaved buffer and compare its base revision before saving.
    ///
    /// `graph_text: Some(hidden)` restores a whole-graph snapshot instead: text
    /// arrives as `Area::Graph` files at their graph-relative paths (Pages and
    /// Journals inputs are refused), and the unlisted live text retired is
    /// every file in the graph-text scope as recorded at snapshot time, with
    /// `hidden` in place of the live `:hidden`, into `<restore-id>/graph/`.
    /// `None` is the configured-roots restore described above.
    ///
    /// Two inputs that would be one file at their actual destination (names
    /// differing in letter case where that directory folds case, or on Apple
    /// volumes in Unicode normalization) refuse before anything is reserved,
    /// retired or written; see `refuse_destination_aliases`.
    pub fn restore(
        &self,
        _kind: crate::EditKind,
        mut files: Vec<RestoreFile>,
        graph_text: Option<&[String]>,
    ) -> Result<RestoreReport, RestoreFailed> {
        let _writer = self.writer.lock().unwrap();
        let mut done = RestoreReport {
            #[cfg(any(test, feature = "test-faults"))]
            restored: 0,
            recovery: Vec::new(),
            kept_external: Vec::new(),
            graph_rev: self.changes.rev(),
        };
        if self.is_closed() {
            return Err(fail(
                "restore",
                io::Error::new(io::ErrorKind::BrokenPipe, "store closed"),
                done,
            ));
        }
        // The recorded scope decides what a whole-graph restore may write and
        // retire, so a later `:hidden` edit cannot retire text it never saw.
        let scope = graph_text.map(|hidden| {
            let mut config = (*self.graph.current_config()).clone();
            config.hidden = hidden.to_vec();
            // The recorded list alone decides (`[""]` records a fail-closed scope).
            config.hidden_parse_failed_closed = false;
            config
        });
        for file in &files {
            let allowed = match file.area {
                Area::Pages | Area::Journals => {
                    scope.is_none() && crate::file_kind::is_graph_text_path(Path::new(&file.rel))
                }
                Area::Graph => scope.as_ref().is_some_and(|scope| {
                    crate::model::graph_text_relative_eligible(&file.rel, scope)
                }),
                Area::Assets => {
                    crate::file_kind::is_asset_sidecar_path(Path::new(&file.rel))
                        && !file.rel.split('/').any(|part| part == ASSET_RECOVERY)
                }
                Area::Meta => file.rel == "config.edn",
                Area::Trash => false,
            };
            if !allowed || (file.area != Area::Graph && self.file_id(file.area, &file.rel).is_err())
            {
                return Err(fail(
                    "restore",
                    io::Error::new(io::ErrorKind::InvalidInput, "unsafe restore file"),
                    done,
                ));
            }
            if file.area == Area::Meta {
                let checked = (|| -> io::Result<()> {
                    let bytes = &file.bytes;
                    if bytes.len() as u64 > crate::model::PARSE_INPUT_MAX_BYTES {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "config input too large",
                        ));
                    }
                    crate::model::validate_parse_bytes_for_path(bytes, Path::new("config.edn"))?;
                    let text = std::str::from_utf8(bytes)
                        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                    let config = tine_core::config::Config::parse(text);
                    self.graph.validate_config_layout(&config)?;
                    let current = self.graph.current_config();
                    if config.pages_dir != current.pages_dir
                        || config.journals_dir != current.journals_dir
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "restore config changes managed directories",
                        ));
                    }
                    Ok(())
                })();
                if let Err(error) = checked {
                    return Err(fail("restore", error, done));
                }
            }
        }
        let baseline = self.watch.restore_baseline();
        let root_path = self.graph.root.clone();
        let assets_path = self.graph.assets_path();
        for (label, path) in [
            (
                "journals",
                root_path.join(&self.graph.current_config().journals_dir),
            ),
            (
                "pages",
                root_path.join(&self.graph.current_config().pages_dir),
            ),
            ("config", root_path.join("logseq/config.edn")),
        ] {
            if let Err(error) = ensure_target_within_root(&root_path, &path) {
                return Err(fail(&format!("unsafe live {label} path"), error, done));
            }
        }
        if crate::path_identity::canonical_existing_path(&root_path.join("assets"))
            .ok()
            .as_ref()
            != Some(&assets_path)
        {
            return Err(fail(
                "unsafe live assets path",
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "external assets directory changed",
                ),
                done,
            ));
        }
        // Destination identity has one owner (REVIEW-backup-zip-2 R2, E2):
        // resolved here, before any recovery area is reserved.
        let destinations = files.iter().map(|file| match file.area {
            Area::Pages => root_path
                .join(&self.graph.current_config().pages_dir)
                .join(&file.rel),
            Area::Journals => root_path
                .join(&self.graph.current_config().journals_dir)
                .join(&file.rel),
            Area::Assets => assets_path.join(&file.rel),
            Area::Graph => root_path.join(&file.rel),
            Area::Meta | Area::Trash => root_path.join("logseq/config.edn"),
        });
        if let Err(error) = refuse_destination_aliases(destinations) {
            return Err(fail("restore", error, done));
        }
        let recovery_id = format!(
            "{}-pre-restore-extras-{}-{}",
            tine_core::date::utc_backup_stamp(),
            std::process::id(),
            RECOVERY_SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let graph = match reserve(&root_path, Path::new("logseq/.tine-trash"), &recovery_id) {
            Ok(value) => value,
            Err(error) => return Err(fail("couldn't create restore recovery area", error, done)),
        };
        done.recovery.push(graph.path.clone());
        let assets = match reserve(&assets_path, Path::new(ASSET_RECOVERY), &recovery_id) {
            Ok(value) => value,
            Err(error) => {
                return Err(fail(
                    "couldn't create asset restore recovery area",
                    error,
                    done,
                ))
            }
        };
        done.recovery.push(assets.path.clone());
        #[cfg(feature = "test-faults")]
        if let Err(error) = pause_after_binding_for_test(&root_path) {
            return Err(fail("restore", error, done));
        }
        let mut changed = false;

        // The old protocol completes one area before starting the next one.
        let config = self.graph.current_config();
        let text_areas = match scope {
            None => vec![
                (
                    Area::Journals,
                    "restore journals failed",
                    config.journals_dir.as_str(),
                    "journals",
                ),
                (
                    Area::Pages,
                    "restore pages failed",
                    config.pages_dir.as_str(),
                    "pages",
                ),
            ],
            Some(_) => vec![(Area::Graph, "restore graph text failed", "", "graph")],
        };
        for (area, phase, live_prefix, recovery_prefix) in
            text_areas
                .into_iter()
                .chain([(Area::Assets, "restore asset sidecars failed", "", "")])
        {
            let bound = if area == Area::Assets {
                &assets
            } else {
                &graph
            };
            let mut restored = HashSet::new();
            for file in files.iter_mut().filter(|file| file.area == area) {
                let live_rel = if live_prefix.is_empty() {
                    PathBuf::from(&file.rel)
                } else {
                    Path::new(live_prefix).join(&file.rel)
                };
                let recover_rel = if recovery_prefix.is_empty() {
                    PathBuf::from(&file.rel)
                } else {
                    Path::new(recovery_prefix).join(&file.rel)
                };
                let mut copying = false;
                let result: io::Result<()> = (|| {
                    if move_if_present(bound, &live_rel, &recover_rel)? {
                        changed = true;
                    }
                    copying = true;
                    copy_new(bound, &live_rel, &file.bytes)?;
                    changed = true;
                    Ok(())
                })();
                if let Err(error) = result {
                    if copying && error.kind() == io::ErrorKind::AlreadyExists {
                        if area == Area::Graph {
                            done.kept_external.push(FileId::from(file.rel.clone()));
                        } else if let Ok(id) = self.file_id(area, &file.rel) {
                            done.kept_external.push(id);
                        }
                    }
                    if changed {
                        self.graph.invalidate_cache();
                        done.graph_rev = self.watch.publish_restore(&baseline);
                    }
                    return Err(fail(phase, error, done));
                }
                restored.insert(PathBuf::from(&file.rel));
                #[cfg(any(test, feature = "test-faults"))]
                {
                    done.restored += 1;
                }
            }
            let live_dir = if live_prefix.is_empty() {
                Path::new("")
            } else {
                Path::new(live_prefix)
            };
            if let Err(error) = retire_extras(
                bound,
                live_dir,
                Path::new(recovery_prefix),
                &restored,
                area,
                scope.as_ref(),
                &mut changed,
            ) {
                if changed {
                    self.graph.invalidate_cache();
                    done.graph_rev = self.watch.publish_restore(&baseline);
                }
                return Err(fail(phase, error, done));
            }
        }
        if let Some(file) = files.iter_mut().find(|file| file.area == Area::Meta) {
            let live = Path::new("logseq/config.edn");
            if let Err(error) = real_parent(&graph.root, Path::new("logseq"), Some(&root_path)) {
                if changed {
                    self.graph.invalidate_cache();
                    done.graph_rev = self.watch.publish_restore(&baseline);
                }
                return Err(fail("couldn't prepare live config directory", error, done));
            }
            match move_if_present(&graph, live, live) {
                Ok(true) => changed = true,
                Ok(false) => {}
                Err(error) => {
                    if changed {
                        self.graph.invalidate_cache();
                        done.graph_rev = self.watch.publish_restore(&baseline);
                    }
                    return Err(fail("recover current config failed", error, done));
                }
            }
            if let Err(error) = copy_new(&graph, live, &file.bytes) {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    if let Ok(id) = self.file_id(Area::Meta, "config.edn") {
                        done.kept_external.push(id);
                    }
                }
                if changed {
                    self.graph.invalidate_cache();
                    done.graph_rev = self.watch.publish_restore(&baseline);
                }
                return Err(fail("restore config failed", error, done));
            }
            changed = true;
            #[cfg(any(test, feature = "test-faults"))]
            {
                done.restored += 1;
            }
        }
        if changed {
            self.graph.invalidate_cache();
        }
        done.graph_rev = self.watch.publish_restore(&baseline);
        Ok(done)
    }
}

#[cfg(test)]
mod config_directory_tests {
    use super::*;
    use std::fs;

    #[test]
    fn restore_refuses_directory_switch_and_accepts_current_layout() {
        let root = std::env::temp_dir().join(format!(
            "tine-restore-layout-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::create_dir_all(root.join("journals")).unwrap();
        fs::create_dir_all(root.join("assets")).unwrap();
        fs::write(root.join("pages/A.md"), "- keep me\n").unwrap();
        let source = root.join("candidate.edn");
        fs::write(&source, "{:pages-directory \"other\"}\n").unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        store.whole_graph().unwrap();
        let candidate = |path: &Path| RestoreFile {
            area: Area::Meta,
            rel: "config.edn".into(),
            bytes: fs::read(path).unwrap(),
        };
        assert!(store
            .restore(crate::EditKind::ReplacePage, vec![candidate(&source)], None)
            .is_err());
        assert_eq!(fs::read(root.join("pages/A.md")).unwrap(), b"- keep me\n");
        fs::write(
            &source,
            "{:pages-directory \"pages\" :journals-directory \"journals\"}\n",
        )
        .unwrap();
        let page_source = root.join("page-source.md");
        fs::write(&page_source, "- keep me\n").unwrap();
        let result = store.restore(
            crate::EditKind::ReplacePage,
            vec![
                candidate(&source),
                RestoreFile {
                    area: Area::Pages,
                    rel: "A.md".into(),
                    bytes: fs::read(&page_source).unwrap(),
                },
            ],
            None,
        );
        assert!(result.is_ok(), "{result:?}");
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn restore_refuses_unloadable_config_before_retiring_pages() {
        let root = std::env::temp_dir().join(format!(
            "tine-restore-invalid-config-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::create_dir_all(root.join("assets")).unwrap();
        fs::write(root.join("pages/A.md"), b"- keep me\n").unwrap();
        let source = root.join("candidate.edn");
        let store = Store::open(&root, Default::default()).unwrap().0;
        for bytes in [
            vec![0xff, 0xfe],
            format!("{}\n", "(".repeat(129) + &")".repeat(129)).into_bytes(),
        ] {
            fs::write(&source, &bytes).unwrap();
            let input = RestoreFile {
                area: Area::Meta,
                rel: "config.edn".into(),
                bytes: bytes.clone(),
            };
            assert!(store
                .restore(crate::EditKind::ReplacePage, vec![input], None)
                .is_err());
            assert_eq!(fs::read(root.join("pages/A.md")).unwrap(), b"- keep me\n");
        }
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod destination_tests {
    use super::*;
    use std::fs;

    fn graph(tag: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("tine-restore-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let root = base.join("graph");
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::write(root.join("pages/A.md"), "- keep me\n").unwrap();
        // The assets target: external where the host can link it.
        #[cfg(unix)]
        let target = {
            let target = base.join("folding-target");
            fs::create_dir_all(&target).unwrap();
            std::os::unix::fs::symlink(&target, root.join("assets")).unwrap();
            target
        };
        #[cfg(not(unix))]
        let target = {
            fs::create_dir_all(root.join("assets")).unwrap();
            root.join("assets")
        };
        fs::write(target.join("old.edn"), "{:old true}").unwrap();
        (base, root)
    }

    fn open(root: &Path) -> Store {
        let assets = root.join("assets");
        let external = fs::symlink_metadata(&assets).unwrap().is_symlink();
        let options = crate::OpenOptions {
            approved_external_assets: external.then(|| fs::canonicalize(&assets).unwrap()),
            ..Default::default()
        };
        let store = Store::open(root, options).unwrap().0;
        store.whole_graph().unwrap();
        store
    }

    fn inputs() -> Vec<RestoreFile> {
        let file = |area, rel: &str, bytes: &str| RestoreFile {
            area,
            rel: rel.into(),
            bytes: bytes.as_bytes().to_vec(),
        };
        vec![
            file(Area::Pages, "A.md", "- restored\n"),
            file(Area::Assets, "Pic.edn", "{:upper true}"),
            file(Area::Assets, "pic.edn", "{:lower true}"),
        ]
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// E2 (REVIEW-backup-zip-2 R2): a case-sensitive graph root with a
    /// case-insensitive assets target. The host cannot make such a volume,
    /// so the probe's answer is injected at this seam: the assets target
    /// folds, the graph root does not. The sidecar pair refuses before
    /// anything is retired or reserved.
    #[test]
    fn a_pair_aliasing_only_at_the_assets_target_refuses_before_mutation() {
        let (base, root) = graph("alias-mixed");
        FOLD_PROBE.with(|probe| {
            probe.set(Some(|dir: &Path| {
                matches!(
                    dir.file_name().and_then(|name| name.to_str()),
                    Some("folding-target" | "assets")
                )
            }))
        });
        let store = open(&root);
        let target = store.graph.assets_path();
        let result = store.restore(crate::EditKind::ReplacePage, inputs(), None);
        FOLD_PROBE.with(|probe| probe.set(None));
        let error = result.expect_err("a destination alias must refuse");
        assert_eq!(error.cause.kind, io::ErrorKind::InvalidInput);
        assert!(
            error.done.recovery.is_empty(),
            "E2/I-2: refused before any recovery area is reserved (exemplar restore.rs \
             refuse_destination_aliases)"
        );
        assert_eq!(fs::read(root.join("pages/A.md")).unwrap(), b"- keep me\n");
        assert_eq!(names(&root.join("pages")), ["A.md"]);
        assert_eq!(names(&target), ["old.edn"]);
        assert!(!root.join("logseq/.tine-trash").exists());
        store.close();
        drop(store);
        fs::remove_dir_all(base).unwrap();
    }

    /// The same pair where both destinations tell case apart restores both
    /// files. Uses the real probe, so it runs where the host's temporary
    /// volume is case-sensitive (Linux); elsewhere it says so and stops.
    #[test]
    fn the_same_pair_on_case_sensitive_destinations_restores_both() {
        let (base, root) = graph("alias-exact");
        let store = open(&root);
        let target = store.graph.assets_path();
        if folds_case(&target) || folds_case(&root.join("pages")) {
            eprintln!("skipped: this host's temporary volume folds case");
        } else {
            let result = store.restore(crate::EditKind::ReplacePage, inputs(), None);
            assert!(result.is_ok(), "{result:?}");
            assert_eq!(fs::read(target.join("Pic.edn")).unwrap(), b"{:upper true}");
            assert_eq!(fs::read(target.join("pic.edn")).unwrap(), b"{:lower true}");
            assert_eq!(fs::read(root.join("pages/A.md")).unwrap(), b"- restored\n");
        }
        store.close();
        drop(store);
        fs::remove_dir_all(base).unwrap();
    }

    /// D8: the probe measures the directory without writing. Linux test
    /// volumes tell case apart; the Windows and macOS runners' default
    /// volumes fold it. A directory with nothing to probe counts as folding.
    #[test]
    fn the_probe_measures_a_directory_without_writing() {
        let dir = std::env::temp_dir().join(format!("tine-restore-probe-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert!(folds_case(&dir), "nothing to probe: the refusing side");
        fs::create_dir(dir.join("pages")).unwrap();
        let probed = folds_case(&dir);
        if cfg!(target_os = "linux") {
            assert!(!probed);
        } else if cfg!(any(windows, target_os = "macos")) {
            assert!(probed);
        }
        assert_eq!(names(&dir), ["pages"], "the probe writes nothing");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn names_fold_as_their_directory_compares_them() {
        let fold = |name: &str, case, nfc| fold_name(std::ffi::OsStr::new(name), case, nfc);
        assert_ne!(fold("A.md", false, false), fold("a.md", false, false));
        assert_eq!(fold("A.md", true, false), fold("a.md", true, false));
        assert_eq!(fold("Č.md", true, false), fold("č.md", true, false));
        // Precomposed vs decomposed: distinct on NTFS and ext4, one on APFS.
        let (nfc, nfd) = ("caf\u{e9}.md", "cafe\u{301}.md");
        assert_ne!(fold(nfc, true, false), fold(nfd, true, false));
        assert_eq!(fold(nfc, false, true), fold(nfd, false, true));
        assert_eq!(fold("CAFE\u{301}.md", true, true), fold(nfc, true, true));
    }
}

fn ensure_target_within_root(root: &Path, target: &Path) -> io::Result<()> {
    let canonical_root = crate::path_identity::canonical_existing_path(root)?;
    let (existing, canonical_existing) = crate::model::canonical_existing_ancestor(target)?;
    let expected = existing
        .strip_prefix(root)
        .map(|rel| canonical_root.join(rel))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "target is outside graph root"))?;
    if canonical_existing == expected {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "target escapes graph root",
        ))
    }
}

/// Refuse inputs that would be one file at their actual destination
/// (REVIEW-backup-zip-2 R2, E2; refusal row `tine-store::restore` in
/// docs/storage-contract.md). Scenario: a snapshot taken where `assets/Pic.edn` and
/// `assets/pic.edn` were two files (malformed import, or a case-sensitive
/// volume) restored where a destination folds case, for example a
/// case-insensitive external assets target under a case-sensitive graph
/// root: the second copy would retire the first restored file, or fail
/// after the graph was already changed. Each destination's nearest existing
/// ancestor directory is resolved on disk (so case variants of existing
/// directories meet by identity); the names below it, which restore creates
/// there, are compared under that directory's probed folding. Nothing is
/// written: the caller runs this before reserving any recovery area. Known
/// gap (approved with D8): Unicode lowercasing approximates a volume's own
/// table (NTFS's upcase table differs on a few characters); a pair only
/// that table equates passes here, and the second copy then retires the
/// first restored file into recovery.
fn refuse_destination_aliases(destinations: impl IntoIterator<Item = PathBuf>) -> io::Result<()> {
    use std::collections::HashMap;
    let normalizes = cfg!(any(target_os = "macos", target_os = "ios"));
    let mut existing: HashMap<PathBuf, Option<usize>> = HashMap::new();
    let mut identities: HashMap<same_file::Handle, usize> = HashMap::new();
    let mut folds: Vec<bool> = Vec::new();
    let mut seen = HashSet::new();
    for destination in destinations {
        let mut found = None;
        for dir in destination.ancestors().skip(1) {
            let index = match existing.get(dir) {
                Some(index) => *index,
                None => {
                    let index = std::fs::metadata(dir)
                        .ok()
                        .filter(std::fs::Metadata::is_dir)
                        .and_then(|_| same_file::Handle::from_path(dir).ok())
                        .map(|handle| {
                            *identities.entry(handle).or_insert_with(|| {
                                folds.push(folds_case(dir));
                                folds.len() - 1
                            })
                        });
                    existing.insert(dir.to_path_buf(), index);
                    index
                }
            };
            if let Some(index) = index {
                found = Some((dir, index));
                break;
            }
        }
        let (dir, index) = found.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no existing restore destination")
        })?;
        let below: Vec<String> = destination
            .strip_prefix(dir)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "unsafe restore file"))?
            .components()
            .map(|part| fold_name(part.as_os_str(), folds[index], normalizes))
            .collect();
        if !seen.insert((index, below)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "two snapshot names are one file at their destination (letter case or Unicode \
                 form); restore it by hand",
            ));
        }
    }
    Ok(())
}

/// A name as a directory that folds case (Unicode lowercase) and, on Apple
/// volumes, Unicode normalization (NFC) compares it.
fn fold_name(name: &std::ffi::OsStr, folds_case: bool, normalizes: bool) -> String {
    use unicode_normalization::UnicodeNormalization;
    let name = name.to_string_lossy();
    let name = if folds_case {
        name.to_lowercase()
    } else {
        name.into_owned()
    };
    if normalizes {
        name.nfc().collect()
    } else {
        name
    }
}

#[cfg(test)]
thread_local! {
    /// Tests stand in for volumes the host cannot create (a case-insensitive
    /// directory beside a case-sensitive one).
    static FOLD_PROBE: std::cell::Cell<Option<fn(&Path) -> bool>> = const { std::cell::Cell::new(None) };
}

/// Whether directory `dir` folds letter case, probed without writing
/// (REVIEW-backup-zip D8): an entry whose ASCII letters are case-flipped
/// either is not found (the directory tells case apart) or resolves to the
/// same file (it folds case). Every case-insensitive directory folds ASCII.
/// No usable entry, or an error, counts as folding: the side that refuses.
fn folds_case(dir: &Path) -> bool {
    #[cfg(test)]
    if let Some(probe) = FOLD_PROBE.with(std::cell::Cell::get) {
        return probe(dir);
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return true;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let flipped: String = name
            .chars()
            .map(|c| match c {
                'a'..='z' => c.to_ascii_uppercase(),
                'A'..='Z' => c.to_ascii_lowercase(),
                _ => c,
            })
            .collect();
        if flipped == name || entry.file_type().map_or(true, |kind| kind.is_symlink()) {
            continue;
        }
        let variant = dir.join(&flipped);
        return match std::fs::symlink_metadata(&variant) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Ok(_) => same_file::is_same_file(entry.path(), &variant).unwrap_or(true),
            Err(_) => true,
        };
    }
    true
}

/// Create the recovery root `<parent_rel>/<id>` and make its entry, and every
/// ancestor entry up to `root_path`, durable before any file is retired into
/// it. Scenario: power loss after restore published the replacement bytes —
/// a recovery tree whose directory entries never reached disk loses the only
/// copy of a retired file (an external editor's edit made after the safety
/// snapshot). One sync per path component, once per restore.
fn reserve(root_path: &Path, parent_rel: &Path, id: &str) -> io::Result<Recovery> {
    let root = Dir::open_ambient_dir(root_path, ambient_authority())?;
    let parent = real_parent(&root, parent_rel, Some(root_path))?;
    parent.create_dir(id)?;
    let mut synced = root_path.join(parent_rel);
    loop {
        crate::directory_durability::sync_directory_entry(&synced)?;
        if synced == root_path || !synced.pop() {
            break;
        }
    }
    let dir = parent.open_dir(id)?;
    Ok(Recovery {
        root_path: root_path.to_path_buf(),
        root,
        dir,
        path: root_path.join(parent_rel).join(id),
    })
}

/// Open `rel` under `root` without following symlinks. `create: Some(base)`
/// (`base` is `root`'s own path) creates missing components and syncs the
/// parent of each one it creates, so a file later published inside survives
/// power loss together with its directory chain (scenario as in [`reserve`]).
fn real_parent(root: &Dir, rel: &Path, create: Option<&Path>) -> io::Result<Dir> {
    let mut current = root.try_clone()?;
    let mut current_path = create.map(Path::to_path_buf);
    let path_kind = if create.is_some() {
        "restore recovery"
    } else {
        "live restore"
    };
    for component in rel.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{path_kind} path is not relative"),
            ));
        };
        if let Some(parent_path) = current_path.as_mut() {
            match current.create_dir(name) {
                Ok(()) => crate::directory_durability::sync_directory_entry(parent_path)?,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            parent_path.push(name);
        }
        let meta = current.symlink_metadata(name)?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{path_kind} path contains a non-directory entry"),
            ));
        }
        current = current.open_dir(name)?;
    }
    Ok(current)
}

fn move_if_present(recovery: &Recovery, live: &Path, recover: &Path) -> io::Result<bool> {
    let name = live
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing live file name"))?;
    let live_parent =
        match real_parent(&recovery.root, live.parent().unwrap_or(Path::new("")), None) {
            Ok(parent) => parent,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
    match live_parent.symlink_metadata(name) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Ok(meta) if meta.file_type().is_symlink() => {
            ensure_target_within_root(&recovery.root_path, &recovery.root_path.join(live))?;
        }
        Ok(meta) if meta.is_file() => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "live restore target is not a regular file",
            ))
        }
        Err(error) => return Err(error),
    }
    let recovery_name = recover
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing recovery file name"))?;
    let recovery_parent_path = recovery
        .path
        .join(recover.parent().unwrap_or(Path::new("")));
    let recovery_parent = real_parent(
        &recovery.dir,
        recover.parent().unwrap_or(Path::new("")),
        Some(&recovery.path),
    )?;
    match recovery_parent.symlink_metadata(recovery_name) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "restore recovery destination already exists",
            ))
        }
        Err(error) => return Err(error),
    }
    match crate::no_replace::rename_noreplace_dir(
        &live_parent,
        Path::new(name),
        &recovery_parent,
        recovery_name,
    ) {
        Ok(()) => {
            // The retired file is protected only once its recovery entry is
            // durable; the caller publishes the replacement after this
            // (power-loss scenario in [`reserve`]).
            crate::directory_durability::sync_directory_entry(&recovery_parent_path)?;
            #[cfg(feature = "test-faults")]
            restore_abort_boundary();
            Ok(true)
        }
        Err(rename_error) => {
            let mut source = live_parent.open(name)?.into_std();
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            let mut copy = recovery_parent
                .open_with(recovery_name, &options)?
                .into_std();
            io::copy(&mut source, &mut copy)?;
            copy.sync_all()?;
            crate::directory_durability::sync_directory_entry(&recovery_parent_path)?;
            Err(io::Error::new(rename_error.kind(), format!(
                "live file copied to recovery but could not be atomically detached: {rename_error}")))
        }
    }
}

fn copy_new(recovery: &Recovery, live: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = live
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing live file name"))?;
    let parent = real_parent(
        &recovery.root,
        live.parent().unwrap_or(Path::new("")),
        Some(&recovery.root_path),
    )?;
    let temp = format!(
        ".tine-restore-{}-{}.tmp",
        std::process::id(),
        COPY_SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut output = parent.open_with(&temp, &options)?.into_std();
        io::Write::write_all(&mut output, bytes)?;
        output.sync_all()?;
        drop(output);
        publish_temp(&parent, Path::new(&temp), name)?;
        #[cfg(feature = "test-faults")]
        restore_abort_boundary();
        crate::directory_durability::sync_directory_entry(
            &recovery
                .root_path
                .join(live.parent().unwrap_or(Path::new(""))),
        )?;
        Ok(())
    })();
    if result.is_err() {
        let _ = parent.remove_file(&temp);
    }
    result
}

/// Retire every live text file (or asset sidecar) under `live_dir` that the
/// restore did not write. Directories are queued, not recursed into, so a
/// delivered tree of any depth costs heap, not stack (I-22). The directory
/// popped next (the last one queued) keeps a handle opened from its parent, so
/// a deep chain costs O(depth) opens, not O(depth²), with at most two open.
fn retire_extras(
    recovery: &Recovery,
    live_dir: &Path,
    recovery_prefix: &Path,
    restored: &HashSet<PathBuf>,
    area: Area,
    scope: Option<&tine_core::config::Config>,
    changed: &mut bool,
) -> io::Result<()> {
    // Whole-graph text: the recorded discovery scope, from the graph root.
    let graph = |child: &Path, dir: bool| {
        scope.is_some_and(|scope| {
            if dir {
                crate::model::graph_text_directory_scannable(Path::new(""), child, scope)
            } else {
                crate::model::graph_text_eligible(Path::new(""), child, scope)
            }
        })
    };
    let mut pending: Vec<(PathBuf, Option<Dir>)> = vec![(PathBuf::new(), None)];
    while let Some((rel, handle)) = pending.pop() {
        let current = match handle.map_or_else(
            || real_parent(&recovery.root, &live_dir.join(&rel), None),
            Ok,
        ) {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let queued = pending.len();
        for entry in current.read_dir(".")? {
            let entry = entry?;
            let name = entry.file_name();
            let child = rel.join(&name);
            let kind = entry.file_type()?;
            if kind.is_dir() {
                if area == Area::Assets {
                    if name == ASSET_RECOVERY {
                        continue;
                    }
                } else if name.to_str().is_none_or(|s| s.starts_with('.'))
                    || (area == Area::Graph && !graph(&child, true))
                {
                    continue;
                }
                pending.push((child, None));
            } else if ((area == Area::Assets
                && kind.is_file()
                && crate::file_kind::is_asset_sidecar_path(&child))
                || (area != Area::Assets
                    && (kind.is_file() || kind.is_symlink())
                    && crate::file_kind::is_graph_text_path(&child)
                    && (area != Area::Graph || graph(&child, false))))
                && !restored.contains(&child)
            {
                let live = live_dir.join(&child);
                let recover = recovery_prefix.join(&child);
                if move_if_present(recovery, &live, &recover)? {
                    *changed = true;
                }
            }
        }
        if pending.len() > queued {
            let (rel, handle) = pending.last_mut().expect("queued above");
            let name = rel.file_name().expect("queued child has a name");
            // Any failure here leaves `None`: `real_parent` then repeats the
            // same checks from the root and reports the error.
            *handle = current
                .symlink_metadata(name)
                .ok()
                .filter(|meta| meta.is_dir() && !meta.file_type().is_symlink())
                .and_then(|_| current.open_dir(name).ok());
        }
    }
    Ok(())
}

fn publish_temp(parent: &Dir, temp: &Path, name: &std::ffi::OsStr) -> io::Result<()> {
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "windows"
    ))]
    {
        crate::no_replace::rename_noreplace_dir(parent, temp, parent, name)
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "windows"
    )))]
    {
        parent.hard_link(temp, parent, Path::new(name))?;
        parent.remove_file(temp)
    }
}

#[cfg(feature = "test-faults")]
fn pause_after_binding_for_test(root: &Path) -> io::Result<()> {
    let request = root.join(".tine-restore-test-pause");
    if !request.exists() {
        return Ok(());
    }
    std::fs::write(root.join(".tine-restore-test-paused"), b"ready")?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !root.join(".tine-restore-test-resume").exists() {
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "restore test pause timed out",
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    Ok(())
}
