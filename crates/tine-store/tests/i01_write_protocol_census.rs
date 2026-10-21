use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use syn::visit::{self, Visit};

#[derive(Default)]
struct Mutations {
    owner: String,
    hits: BTreeSet<(String, String)>,
}

use crate::test_cfg::is_test as test_only;

impl<'ast> Visit<'ast> for Mutations {
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if !test_only(&node.attrs) {
            visit::visit_item_mod(self, node);
        }
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if test_only(&node.attrs) {
            return;
        }
        let old = std::mem::replace(&mut self.owner, node.sig.ident.to_string());
        visit::visit_item_fn(self, node);
        self.owner = old;
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        if test_only(&node.attrs) {
            return;
        }
        let old = std::mem::replace(&mut self.owner, node.sig.ident.to_string());
        visit::visit_impl_item_fn(self, node);
        self.owner = old;
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = node.func.as_ref() {
            let names: Vec<_> = path
                .path
                .segments
                .iter()
                .map(|part| part.ident.to_string())
                .collect();
            if names.len() >= 2 {
                let method = names[names.len() - 1].as_str();
                let parent = names[names.len() - 2].as_str();
                if (matches!(parent, "fs" | "File")
                    && matches!(
                        method,
                        "write"
                            | "rename"
                            | "copy"
                            | "remove_file"
                            | "remove_dir"
                            | "remove_dir_all"
                            | "create"
                            | "create_dir"
                            | "create_dir_all"
                    ))
                    || (parent == "OpenOptions" && method == "new")
                    || (parent == "libc"
                        && matches!(method, "syscall" | "renameatx_np" | "renameat"))
                    || (parent == "FileSystem" && method == "MoveFileExW")
                {
                    self.hits
                        .insert((self.owner.clone(), format!("{parent}::{method}")));
                }
            }
        }
        visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let method = node.method.to_string();
        if matches!(
            method.as_str(),
            "rename"
                | "remove_file"
                | "remove_dir"
                | "remove_dir_all"
                | "hard_link"
                | "create_new"
                | "create_dir"
                | "create_dir_all"
        ) {
            self.hits.insert((self.owner.clone(), method));
        }
        visit::visit_expr_method_call(self, node);
    }
}

#[test]
fn every_content_mutation_has_a_reviewed_owner() {
    let _integration_guard = crate::integration_lock();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let files = [
        "crates/tine-store/src/model.rs",
        "crates/tine-store/src/no_replace.rs",
        "crates/tine-store/src/transaction.rs",
        "crates/tine-store/src/restore.rs",
        "crates/tine-store/src/publish.rs",
        "crates/tine-store/src/store.rs",
        "crates/tine-graph-features/src/assets.rs",
        "crates/tine-graph-features/src/pages.rs",
        "crates/tine-graph-features/src/pdf.rs",
        "crates/tine-graph-features/src/guide.rs",
        "src-tauri/src/backup.rs",
        // Split from backup.rs (og-B); a seam split never shrinks the census.
        "src-tauri/src/backup/restore.rs",
        // Zip backups (og-backup-zip): the archive codec and the names are
        // in-memory only; the namespace lock opens its lock file.
        "src-tauri/src/backup/archive.rs",
        "src-tauri/src/backup/naming.rs",
        "src-tauri/src/file_lock.rs",
        "src-tauri/src/commands.rs",
        "src-tauri/src/commands/concord.rs",
        "src-tauri/src/graph.rs",
        "src-tauri/src/state.rs",
        "src-tauri/src/watcher.rs",
        "src-tauri/src/device_io.rs",
        // Git autocommit and CLI run commits (ADR 0078): `.git` only, never
        // the work tree (review F7).
        "src-tauri/src/owner/git_run.rs",
        "src-tauri/src/owner/git_autocommit.rs",
    ];
    let mut found = BTreeSet::new();
    for rel in files {
        let source = fs::read_to_string(repo.join(rel)).unwrap();
        let syntax = syn::parse_file(&source).unwrap_or_else(|error| panic!("{rel}: {error}"));
        let mut visitor = Mutations::default();
        visitor.visit_file(&syntax);
        for (owner, operation) in visitor.hits {
            found.insert((rel.to_string(), owner, operation));
        }
    }
    // Each name is a reviewed protocol owner, not an unrestricted file allowlist.
    // A new owner must state its crash matrix before being admitted here.
    let reviewed: BTreeSet<(&str, &str)> = [
        ("crates/tine-store/src/model.rs", "atomic_copy_file_new"),
        ("crates/tine-store/src/model.rs", "atomic_copy_new"),
        ("crates/tine-store/src/model.rs", "atomic_write_new"),
        ("crates/tine-store/src/model.rs", "atomic_write_with_check"),
        ("crates/tine-store/src/no_replace.rs", "move_at"),
        ("crates/tine-store/src/no_replace.rs", "move_windows"),
        (
            "crates/tine-store/src/publish.rs",
            "commit_publish_stage_report",
        ),
        ("crates/tine-store/src/publish.rs", "drop"),
        (
            "crates/tine-store/src/publish.rs",
            "write_publish_stage_file",
        ),
        ("crates/tine-store/src/restore.rs", "copy_new"),
        ("crates/tine-store/src/restore.rs", "move_if_present"),
        ("crates/tine-store/src/restore.rs", "publish_temp"),
        // `cfg(feature = "test-faults")` only (a dev-dependency feature, never
        // in a shipped build), censused since the cfg parser (N2) stopped
        // reading "test-faults" as test: writes the `.tine-restore-test-paused`
        // handshake marker in a test graph's root, never graph content. Crash
        // matrix: a crash leaves a stray marker in a temporary test directory.
        (
            "crates/tine-store/src/restore.rs",
            "pause_after_binding_for_test",
        ),
        ("crates/tine-store/src/store.rs", "purge_asset_trash"),
        (
            "crates/tine-store/src/store.rs",
            "remove_trash_entry_counted",
        ),
        ("crates/tine-store/src/transaction.rs", "apply"),
        ("crates/tine-store/src/transaction.rs", "commit_timed"),
        // Zip backups, app data only (docs/storage-contract.md, Graph
        // backups). Publication, partial cleanup, settling and retention run
        // under the namespace lock; creating `<id>.zips/` necessarily
        // precedes taking that lock (below). Crash matrix: a crash before the
        // no-replace rename leaves a `.partial-*.zip` that is never listed
        // and that the next lock holder removes after its name scan; after
        // it, a complete zip that is visible but settled only once the
        // zips -> backups -> app-data sync chain ran, which every
        // publication and every keeper settlement runs
        // (`a_crash_leaves_a_partial_or_a_complete_zip`,
        // `every_publication_settles_the_whole_chain`,
        // scripts/check-backup-syscalls.mjs).
        ("src-tauri/src/backup.rs", "publish_zip"),
        ("src-tauri/src/backup.rs", "remove_partials"),
        // Deletes only after a verified keeper zip is settled strongly (its
        // own publication, or `settle`); a crash or failed delete mid-way
        // leaves a subset the next locked maintenance, including an
        // unchanged launch, finishes
        // (`deletion_needs_a_verified_strongly_synced_keeper`,
        // `a_failed_prune_delete_is_retried_by_the_next_unchanged_launch`).
        ("src-tauri/src/backup.rs", "maintain_locked"),
        // Opens a published zip for writing only to `sync_all` it, then
        // syncs the directory chain; no content change.
        ("src-tauri/src/backup.rs", "settle"),
        // Empty, idempotent directory creation (create_dir_all, or a
        // create_dir whose AlreadyExists is accepted): a crash leaves the
        // directory or nothing, and the next caller recreates it. The
        // creation itself carries no content; content later written inside
        // depends on the ancestors reaching disk, which each owner's own
        // protocol settles (or does not claim), not this admission.
        // `<id>.zips/` before the backup namespace lock: the publication's
        // sync chain settles it; restore only reads under it.
        ("src-tauri/src/backup.rs", "write_snapshot"),
        ("src-tauri/src/backup/restore.rs", "under_namespace_lock"),
        // Graph-side directories made before an audited write into them.
        (
            "crates/tine-store/src/model.rs",
            "withdraw_file_to_conflict_if",
        ),
        ("crates/tine-store/src/publish.rs", "publish_site_at"),
        ("crates/tine-store/src/store.rs", "create_graph"),
        ("src-tauri/src/device_io.rs", "atomic_update_with_hooks"),
        ("src-tauri/src/device_io.rs", "copy_tree"),
        ("src-tauri/src/graph.rs", "default_graph_parent"),
        // Directory creation as a protocol step, not idempotent setup.
        // Exclusive reservation: `create_dir` of a fresh unique name claims a
        // publish stage or recovery root; AlreadyExists means taken, so the
        // owner picks another name. Crash matrix: at worst an empty, uniquely
        // named directory; what is published into it is governed by the
        // publish protocol's owners above (the recovery root and its
        // ancestors are synced before use).
        (
            "crates/tine-store/src/publish.rs",
            "reserve_publish_recovery",
        ),
        (
            "crates/tine-store/src/publish.rs",
            "reserve_publish_stage_at",
        ),
        // Restore's recovery root and missing destination parents: each
        // created component's parent is synced before a file is retired or
        // published inside (`reserve`, `real_parent` docs).
        ("crates/tine-store/src/restore.rs", "real_parent"),
        ("crates/tine-store/src/restore.rs", "reserve"),
        // Creates the persistent, empty lock file; never truncated or unlinked.
        ("src-tauri/src/file_lock.rs", "open_lock_file"),
        // Thumbnail cache cleanup is outside graph/private-durable state.
        ("src-tauri/src/commands.rs", "import_native_capture"),
        // Deletes a forgotten graph's launch checkpoint (ADR 0070), a
        // disposable cache in app data, never graph content. Crash matrix: a
        // crash or failed remove leaves the old file; re-adding that graph
        // validates it like any checkpoint (header, config key, stamps) and
        // rebuilds on any mismatch.
        ("src-tauri/src/graph.rs", "forget_launch_checkpoint"),
        // Not a filesystem call: `PageHost::rename`, the page host's rename
        // operation (STEP3 §7). Crash matrix: the host's operation custody
        // and drafts (`page_host`, `binding_rename_tests` Q4 crash traces).
        ("crates/tine-graph-features/src/pages.rs", "host_rename"),
        ("src-tauri/src/device_io.rs", "atomic_write"),
        ("src-tauri/src/device_io.rs", "atomic_write_new"),
        // Git (ADR 0078): every path is under `.git`, never the work tree.
        // The index follows git's own protocol: `index.lock` taken with
        // create_new (held = another git command's, never touched), a private
        // copy `tine-index-<uuid>.tmp` beside it, the new index written into
        // the lock, fsynced and renamed over `index`. Crash matrix: before the
        // rename the old index stands and a stale `index.lock` blocks git
        // until removed (Settings names it after 30 min); copies left by a
        // crash are removed by the next lock holder only
        // (`a_crash_left_index_copy_is_removed_only_under_the_lock`,
        // `a_missing_index_refuses_instead_of_committing_deletions`).
        ("src-tauri/src/owner/git_run.rs", "acquire"),
        ("src-tauri/src/owner/git_run.rs", "install"),
        ("src-tauri/src/owner/git_run.rs", "drop"),
        ("src-tauri/src/owner/git_run.rs", "remove_stale_copies"),
        // Repack (git's `repack -d` + `prune-packed`): a loose object is
        // deleted only once the new pack holding it, its idx and the pack
        // directory are fsynced; then empty fan-out directories go. Crash
        // matrix: a crash leaves loose duplicates of packed objects, which
        // the next repack packs again and deletes
        // (`repack_deletes_only_packed_loose_objects`).
        ("src-tauri/src/owner/git_autocommit.rs", "repack"),
        ("src-tauri/src/owner/git_autocommit.rs", "remove_object"),
    ]
    .into_iter()
    .collect();
    let new: Vec<_> = found
        .iter()
        .filter(|(file, owner, _)| !reviewed.contains(&(file.as_str(), owner.as_str())))
        .collect();
    assert!(new.is_empty(), "I-1: every content mutation needs a named audited owner and crash matrix; exemplar model.rs atomic_write_new; unreviewed: {new:#?}");
}

#[test]
fn census_detects_an_unreviewed_direct_write() {
    let _integration_guard = crate::integration_lock();
    let planted =
        syn::parse_file("fn rogue() { std::fs::write(\"page.md\", b\"lost\").unwrap(); }").unwrap();
    let mut visitor = Mutations::default();
    visitor.visit_file(&planted);
    assert!(
        visitor.hits.contains(&("rogue".into(), "fs::write".into())),
        "I-1: a new direct write must trigger owner review; exemplar model.rs atomic_write_new"
    );
}

#[test]
fn census_detects_an_unreviewed_directory_creation() {
    let _integration_guard = crate::integration_lock();
    let planted =
        syn::parse_file("fn rogue() { std::fs::create_dir_all(\"backups/x.zips\").unwrap(); }")
            .unwrap();
    let mut visitor = Mutations::default();
    visitor.visit_file(&planted);
    assert!(
        visitor
            .hits
            .contains(&("rogue".into(), "fs::create_dir_all".into())),
        "I-1: a new directory creation must trigger owner review; exemplar backup.rs write_snapshot"
    );
}

#[test]
fn census_detects_an_unreviewed_directory_creation_through_a_handle() {
    let _integration_guard = crate::integration_lock();
    let planted = syn::parse_file(
        "fn rogue(dir: &cap_std::fs::Dir) { dir.create_dir_all(\"backups/x.zips\").unwrap(); }",
    )
    .unwrap();
    let mut visitor = Mutations::default();
    visitor.visit_file(&planted);
    assert!(
        visitor
            .hits
            .contains(&("rogue".into(), "create_dir_all".into())),
        "I-1: a handle-relative directory creation must trigger owner review too; exemplar \
         backup.rs write_snapshot"
    );
}

/// N2 family (REVIEW-3b-P2b-2): a `cfg(not(test))` write is production and
/// needs a reviewed owner like any other.
#[test]
fn census_detects_a_not_test_direct_write() {
    let _integration_guard = crate::integration_lock();
    let planted = syn::parse_file(
        "#[cfg(not(test))] fn rogue() { std::fs::write(\"page.md\", b\"lost\").unwrap(); }",
    )
    .unwrap();
    let mut visitor = Mutations::default();
    visitor.visit_file(&planted);
    assert!(
        visitor.hits.contains(&("rogue".into(), "fs::write".into())),
        "cfg(not(test)) is production"
    );
}
