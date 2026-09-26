use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use syn::visit::{self, Visit};

#[derive(Default)]
struct Mutations {
    owner: String,
    hits: BTreeSet<(String, String)>,
}

fn test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("test")
            || (attr.path().is_ident("cfg") && quote::quote!(#attr).to_string().contains("test"))
    })
}

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
                    ))
                    || (parent == "OpenOptions" && method == "new")
                    || (parent == "libc" && matches!(method, "syscall" | "renameatx_np"))
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
            "rename" | "remove_file" | "remove_dir" | "remove_dir_all" | "hard_link" | "create_new"
        ) {
            self.hits.insert((self.owner.clone(), method));
        }
        visit::visit_expr_method_call(self, node);
    }
}

#[test]
fn every_content_mutation_has_a_reviewed_owner() {
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
        "src-tauri/src/commands.rs",
        "src-tauri/src/graph.rs",
        "src-tauri/src/state.rs",
        "src-tauri/src/watcher.rs",
        "src-tauri/src/device_io.rs",
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
        ("crates/tine-store/src/store.rs", "purge_asset_trash"),
        (
            "crates/tine-store/src/store.rs",
            "remove_trash_entry_counted",
        ),
        ("crates/tine-store/src/transaction.rs", "apply"),
        ("crates/tine-store/src/transaction.rs", "commit"),
        ("src-tauri/src/backup.rs", "cleanup_partial_backups"),
        ("src-tauri/src/backup.rs", "drop"),
        ("src-tauri/src/backup.rs", "prune_backups"),
        ("src-tauri/src/backup.rs", "publish_snapshot"),
        ("src-tauri/src/backup.rs", "sync_dir"),
        ("src-tauri/src/backup.rs", "write_manifest"),
        ("src-tauri/src/backup.rs", "write_payload"),
        // Thumbnail cache cleanup is outside graph/private-durable state.
        ("src-tauri/src/commands.rs", "import_native_capture"),
        ("src-tauri/src/device_io.rs", "atomic_write"),
        ("src-tauri/src/device_io.rs", "atomic_write_new"),
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
    let planted =
        syn::parse_file("fn rogue() { std::fs::write(\"page.md\", b\"lost\").unwrap(); }").unwrap();
    let mut visitor = Mutations::default();
    visitor.visit_file(&planted);
    assert!(
        visitor.hits.contains(&("rogue".into(), "fs::write".into())),
        "I-1: a new direct write must trigger owner review; exemplar model.rs atomic_write_new"
    );
}
