//! I-1 scope ratchet. Syntactic presence is evidence of review, not a call graph
//! or a proof of check ordering. See docs/storage-model/ASSUMPTIONS.md.
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use syn::visit::{self, Visit};

pub(super) fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub(super) fn files(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            result.extend(files(&path, extension));
        } else if path.extension().is_some_and(|ext| ext == extension) {
            result.push(path);
        }
    }
    result.sort();
    result
}

// Like test_cfg, but this census deliberately also excludes test-faults-only
// items. any(test-faults, production) and not(test) must still be scanned.
fn excluded(attrs: &[syn::Attribute]) -> bool {
    fn implies(meta: &syn::Meta) -> bool {
        match meta {
            syn::Meta::Path(path) => path.is_ident("test"),
            syn::Meta::NameValue(value) if value.path.is_ident("feature") => {
                matches!(&value.value, syn::Expr::Lit(lit)
                    if matches!(&lit.lit, syn::Lit::Str(s) if s.value() == "test-faults"))
            }
            syn::Meta::List(list) if list.path.is_ident("all") || list.path.is_ident("any") => {
                let args = list
                    .parse_args_with(
                        syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
                    )
                    .unwrap();
                if list.path.is_ident("all") {
                    args.iter().any(implies)
                } else {
                    !args.is_empty() && args.iter().all(implies)
                }
            }
            _ => false,
        }
    }
    crate::test_cfg::is_test(attrs)
        || attrs.iter().any(|attr| {
            attr.path().is_ident("cfg") && attr.parse_args::<syn::Meta>().is_ok_and(|m| implies(&m))
        })
}

#[derive(Default, Debug)]
struct Owner {
    body: String,
    operations: BTreeMap<String, usize>,
    host_method: bool,
}

#[derive(Default)]
struct Mutations {
    git_metadata: bool,
    owner: String,
    implementation: String,
    host_impl: bool,
    owners: BTreeMap<String, Owner>,
}

impl Mutations {
    fn hit(&mut self, operation: String) {
        assert!(
            !self.owner.is_empty(),
            "mutation outside a function: {operation}"
        );
        *self
            .owners
            .entry(self.owner.clone())
            .or_default()
            .operations
            .entry(operation)
            .or_default() += 1;
    }
}

impl<'ast> Visit<'ast> for Mutations {
    fn visit_item(&mut self, node: &'ast syn::Item) {
        // Cover cfg on impls as well as modules/functions.
        let attrs = match node {
            syn::Item::Mod(n) => &n.attrs,
            syn::Item::Fn(n) => &n.attrs,
            syn::Item::Impl(n) => &n.attrs,
            _ => return visit::visit_item(self, node),
        };
        if !excluded(attrs) {
            visit::visit_item(self, node);
        }
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        let implementation = match node.self_ty.as_ref() {
            syn::Type::Path(path) => path.path.segments.last().unwrap().ident.to_string(),
            ty => ty.to_token_stream().to_string().replace(' ', ""),
        };
        let old = std::mem::replace(&mut self.implementation, implementation);
        let host = std::mem::replace(
            &mut self.host_impl,
            self.implementation == "ProductionIo"
                && node
                    .trait_
                    .as_ref()
                    .is_some_and(|(_, p, _)| p.is_ident("HostIo")),
        );
        visit::visit_item_impl(self, node);
        self.implementation = old;
        self.host_impl = host;
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if excluded(&node.attrs) {
            return;
        }
        let old = std::mem::replace(&mut self.owner, node.sig.ident.to_string());
        self.owners
            .entry(self.owner.clone())
            .or_default()
            .body
            .push_str(&format!(
                "{} {}",
                node.sig.to_token_stream(),
                node.block.to_token_stream()
            ));
        visit::visit_item_fn(self, node);
        self.owner = old;
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        if excluded(&node.attrs) {
            return;
        }
        let old = std::mem::replace(
            &mut self.owner,
            format!("{}::{}", self.implementation, node.sig.ident),
        );
        let owner = self.owners.entry(self.owner.clone()).or_default();
        owner.body.push_str(&format!(
            "{} {}",
            node.sig.to_token_stream(),
            node.block.to_token_stream()
        ));
        owner.host_method = self.host_impl;
        visit::visit_impl_item_fn(self, node);
        self.owner = old;
    }

    fn visit_expr(&mut self, node: &'ast syn::Expr) {
        // cfg can also remove a statement/block inside a production function.
        if let syn::Expr::Block(block) = node {
            if excluded(&block.attrs) {
                return;
            }
        }
        visit::visit_expr(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = node.func.as_ref() {
            let names: Vec<_> = path
                .path
                .segments
                .iter()
                .map(|p| p.ident.to_string())
                .collect();
            let method = names.last().unwrap().as_str();
            let parent = names.iter().rev().nth(1).map(String::as_str).unwrap_or("");
            let raw = (matches!(parent, "fs" | "File" | "Dir")
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
                        | "hard_link"
                        | "set_permissions"
                ))
                || (parent == "OpenOptions" && method == "new")
                || (parent == "libc"
                    && matches!(
                        method,
                        "syscall" | "renameatx_np" | "renameat" | "rename" | "unlinkat" | "mkdirat"
                    ))
                || matches!(
                    method,
                    "MoveFileExW" | "ReplaceFileW" | "DeleteFileW" | "CreateDirectoryW"
                );
            let own = matches!(
                method,
                "move_file_noreplace"
                    | "rename_noreplace_dir"
                    | "rename_replace"
                    | "create_dir_all_durable"
                    | "create_dir_all_with_sync"
                    | "atomic_write"
                    | "atomic_write_new"
                    | "atomic_write_private"
                    | "atomic_write_with_check"
                    | "atomic_copy_new"
                    | "atomic_copy_file_new"
                    | "sync_existing"
                    | "sync_directory_entry"
                    | "sync_directory_witness"
                    | "sync_private_directory"
                    | "temp"
                    | "remove_present"
                    | "sync_move_dirs"
                    | "create_graph_dir"
                    | "create_graph_dir_all"
                    | "remove_graph_file"
                    | "remove_graph_dir"
                    | "remove_publish_stage"
                    | "rename_publish_entry"
                    | "remove_restore_temp"
                    | "rename_spelling_replace"
            ) || (parent == "PreparedWrite"
                && matches!(method, "with_hooks" | "publish"))
                || (parent == "DirectoryCreation" && method == "new");
            if raw || own {
                self.hit(if parent.is_empty() {
                    method.into()
                } else {
                    format!("{parent}::{method}")
                });
            }
        }
        visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let method = node.method.to_string();
        if (self.git_metadata
            && matches!(
                method.as_str(),
                "blob"
                    | "blob_writer"
                    | "commit"
                    | "write_tree_to"
                    | "reference"
                    | "reference_matching"
                    | "write"
            ))
            || (method == "write" && !node.args.is_empty())
            || matches!(
                method.as_str(),
                "write_all"
                    | "write_vectored"
                    | "set_len"
                    | "sync_all"
                    | "sync_data"
                    | "rename"
                    | "remove_file"
                    | "remove_dir"
                    | "remove_dir_all"
                    | "hard_link"
                    | "create"
                    | "create_new"
                    | "create_dir"
                    | "create_dir_all"
                    | "publish"
                    | "withdraw_file_to_conflict_if"
                    | "graph_directory"
                    | "graph_sync"
            )
        {
            self.hit(format!(".{method}"));
        }
        visit::visit_expr_method_call(self, node);
    }
}

// References are exact file#owner names; tests must be real #[test] functions,
// not a comment, a substring, or a production function with the same spelling.
pub(super) fn reference_exists(reference: &str, test: bool) -> bool {
    let Some((file, name)) = reference.split_once('#') else {
        return false;
    };
    let Ok(source) = fs::read_to_string(repo().join(file)) else {
        return false;
    };
    let Ok(syntax) = syn::parse_file(&source) else {
        return false;
    };
    if !test {
        let mut scan = Mutations::default();
        scan.visit_file(&syntax);
        return scan.owners.contains_key(name);
    }
    struct Tests(BTreeSet<String>);
    impl<'ast> Visit<'ast> for Tests {
        fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
            if node
                .attrs
                .iter()
                .any(|a| a.path().segments.last().is_some_and(|s| s.ident == "test"))
            {
                self.0.insert(node.sig.ident.to_string());
            }
            visit::visit_item_fn(self, node);
        }
    }
    let mut tests = Tests(BTreeSet::new());
    tests.visit_file(&syntax);
    tests.0.contains(name)
}

type Row = BTreeMap<String, String>;

// Deliberately small TOML subset: [[site]], one JSON-compatible quoted string
// or boolean per line. Reject unknown/duplicate keys rather than silently drift.
fn ledger(source: &str) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    for line in source
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        if line == "[[site]]" {
            rows.push(Row::new());
            continue;
        }
        let (key, value) = line.split_once('=').expect("ledger key = value");
        let key = key.trim();
        assert!(
            [
                "file",
                "fn",
                "domain",
                "guard",
                "test",
                "oracle",
                "note",
                "mutations"
            ]
            .contains(&key),
            "unknown ledger key {key}"
        );
        let value = value.trim();
        let value = if key == "oracle" {
            assert!(matches!(value, "true" | "false"));
            value.into()
        } else {
            serde_json::from_str::<String>(value).expect("quoted ledger value")
        };
        assert!(
            rows.last_mut()
                .expect("[[site]] first")
                .insert(key.into(), value)
                .is_none(),
            "duplicate key {key}"
        );
    }
    for row in &rows {
        for key in [
            "file",
            "fn",
            "domain",
            "guard",
            "test",
            "oracle",
            "note",
            "mutations",
        ] {
            assert!(
                row.contains_key(key),
                "ledger row {row:?}: missing {key}; supply every reviewed field"
            );
        }
    }
    rows
}

fn ledger_path(relative: &Path) -> String {
    // Ledger keys use repository slashes, including the Swift scan below.
    // read_dir appends native separators even to a slash-spelled scan root.
    relative.to_str().unwrap().replace('\\', "/")
}

fn inventory() -> BTreeMap<(String, String), Owner> {
    let root = repo();
    let mut result = BTreeMap::new();
    for dir in [
        "crates/tine-store/src",
        "crates/tine-graph-features/src",
        "src-tauri/src",
    ] {
        let sources = files(&root.join(dir), "rs");
        let mut test_modules = BTreeSet::new();
        // Out-of-line #[cfg(test)] mod declarations are classified with syn,
        // too: their files do not necessarily carry a file-level cfg.
        loop {
            let before = test_modules.len();
            for file in &sources {
                let syntax = syn::parse_file(&fs::read_to_string(file).unwrap()).unwrap();
                let parent = file.parent().unwrap();
                let stem = file.file_stem().unwrap().to_str().unwrap();
                let base = if matches!(stem, "lib" | "main" | "mod") {
                    parent.to_path_buf()
                } else {
                    parent.join(stem)
                };
                fn modules(
                    items: &[syn::Item],
                    base: &Path,
                    parent: &Path,
                    hidden: bool,
                    out: &mut BTreeSet<PathBuf>,
                ) {
                    for item in items {
                        if let syn::Item::Mod(module) = item {
                            let hidden = hidden || excluded(&module.attrs);
                            if let Some((_, items)) = &module.content {
                                modules(
                                    items,
                                    &base.join(module.ident.to_string()),
                                    parent,
                                    hidden,
                                    out,
                                );
                            } else if hidden {
                                let explicit = module.attrs.iter().find_map(|a| {
                                    if a.path().is_ident("path") {
                                        if let syn::Meta::NameValue(v) = &a.meta {
                                            if let syn::Expr::Lit(l) = &v.value {
                                                if let syn::Lit::Str(s) = &l.lit {
                                                    return Some(parent.join(s.value()));
                                                }
                                            }
                                        }
                                    }
                                    None
                                });
                                if let Some(path) = explicit {
                                    out.insert(path);
                                } else {
                                    out.insert(base.join(format!("{}.rs", module.ident)));
                                    out.insert(base.join(module.ident.to_string()));
                                }
                            }
                        }
                    }
                }
                let hidden = excluded(&syntax.attrs)
                    || test_modules.iter().any(|path| file.starts_with(path));
                modules(&syntax.items, &base, parent, hidden, &mut test_modules);
                if excluded(&syntax.attrs) {
                    test_modules.insert(file.clone());
                }
            }
            if test_modules.len() == before {
                break;
            }
        }
        for file in sources {
            if test_modules.iter().any(|path| file.starts_with(path)) {
                continue;
            }
            let source = fs::read_to_string(&file).unwrap();
            let syntax =
                syn::parse_file(&source).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            let mut scan = Mutations {
                git_metadata: file.ends_with("owner/git_run.rs")
                    || file.ends_with("owner/git_autocommit.rs"),
                ..Mutations::default()
            };
            scan.visit_file(&syntax);
            for (name, owner) in scan.owners {
                if !owner.operations.is_empty() {
                    result.insert(
                        (ledger_path(file.strip_prefix(&root).unwrap()), name),
                        owner,
                    );
                }
            }
        }
    }
    // Swift has no syn AST. Scope each regex match to its nearest func (the
    // agreed fallback); scan every Swift file, including new plugin directories.
    let function = regex::Regex::new(r"\bfunc\s+(\w+)").unwrap();
    let mutation =
        regex::Regex::new(r"\bcreateDirectory\b|\bcreateFile\b|\bwrite\s*\(\s*to:").unwrap();
    for file in files(&root.join("src-tauri"), "swift") {
        let source = fs::read_to_string(&file).unwrap();
        let functions: Vec<_> = function.captures_iter(&source).collect();
        for (i, capture) in functions.iter().enumerate() {
            let start = capture.get(0).unwrap().start();
            let end = functions
                .get(i + 1)
                .map(|c| c.get(0).unwrap().start())
                .unwrap_or(source.len());
            let body = &source[start..end];
            let mut owner = Owner {
                body: body.into(),
                ..Owner::default()
            };
            for hit in mutation.find_iter(body) {
                *owner.operations.entry(hit.as_str().into()).or_default() += 1;
            }
            if !owner.operations.is_empty() {
                result.insert(
                    (
                        ledger_path(file.strip_prefix(&root).unwrap()),
                        capture[1].into(),
                    ),
                    owner,
                );
            }
        }
    }
    result
}

fn operations(owner: &Owner) -> String {
    owner
        .operations
        .iter()
        .map(|(name, count)| format!("{name}={count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn scope_errors(found: &BTreeMap<(String, String), Owner>, rows: &[Row]) -> Vec<String> {
    let reviewed: BTreeSet<_> = rows
        .iter()
        .map(|r| (r["file"].clone(), r["fn"].clone()))
        .collect();
    let mut errors = Vec::new();
    for row in rows {
        let key = (row["file"].clone(), row["fn"].clone());
        match found.get(&key) {
            None => errors.push(format!(
                "stale row {key:?}: remove or review its replacement"
            )),
            Some(owner) if row["mutations"] != operations(owner) => errors.push(format!(
                "changed mutations at {key:?}: {}",
                operations(owner)
            )),
            _ => {}
        }
    }
    for (key, owner) in found {
        if !reviewed.contains(key) {
            errors.push(format!(
                "new mutation site outside the ledger: {key:?}: {}",
                operations(owner)
            ));
        }
    }
    errors
}

#[test]
fn every_content_mutation_has_a_reviewed_owner() {
    let _integration_guard = crate::integration_lock();
    let found = inventory();
    let rows = ledger(include_str!("write_sites.toml"));
    let mut reviewed = BTreeSet::new();
    let errors = scope_errors(&found, &rows);
    for row in &rows {
        let key = (row["file"].clone(), row["fn"].clone());
        assert!(
            reviewed.insert(key.clone()),
            "duplicate ledger row: {key:?}"
        );
        let Some(owner) = found.get(&key) else {
            continue;
        };
        let domain = row["domain"].as_str();
        assert!(
            [
                "graph",
                "app",
                "scratch",
                "dotgit",
                "container",
                "create-root",
                "primitive"
            ]
            .contains(&domain),
            "{key:?}: invalid domain"
        );
        assert!(
            !row["note"].is_empty(),
            "{key:?}: explain classification and crash scope"
        );
        if matches!(domain, "graph" | "dotgit") {
            let guard = &row["guard"];
            if let Some(id) = guard.strip_prefix("unguarded:") {
                let assumptions =
                    fs::read_to_string(repo().join("docs/storage-model/ASSUMPTIONS.md")).unwrap();
                assert!(
                    assumptions
                        .lines()
                        .any(|l| l.starts_with(&format!("| {id} |")) && l.contains("unguarded")),
                    "{key:?}: unguarded finding needs an assumption row"
                );
            } else {
                let Some((file, name)) = guard.split_once('#') else {
                    panic!("{key:?}: guard must name file#owner or unguarded:A-ID");
                };
                let source = fs::read_to_string(repo().join(file)).unwrap();
                let mut scan = Mutations::default();
                scan.visit_file(&syn::parse_file(&source).unwrap());
                let body = &scan
                    .owners
                    .get(name)
                    .unwrap_or_else(|| panic!("{key:?}: missing guard {guard}"))
                    .body;
                assert!(
                    body.contains(". path (")
                        || [
                            "check_path",
                            "spelled_path",
                            "ensure_write_target",
                            "ensure_asset_write_target",
                            "checked_write_target",
                            "Dir"
                        ]
                        .iter()
                        .any(|token| body
                            .split(|c: char| !c.is_alphanumeric() && c != '_')
                            .any(|word| word == *token)),
                    "{key:?}: {guard} has no containment/handle token"
                );
            }
        }
        if domain == "graph" || !row["test"].is_empty() {
            assert!(
                reference_exists(&row["test"], true),
                "{key:?}: missing test {}",
                row["test"]
            );
        }
        if domain == "create-root" {
            assert_eq!(
                key,
                (
                    "crates/tine-store/src/store.rs".into(),
                    "Store::create_graph".into()
                )
            );
        }
        if domain == "container" {
            assert_eq!(
                key.0,
                "src-tauri/ios-folder-picker-native/ios/Sources/GraphFolderPickerPlugin.swift"
            );
        }
        // HostIo is the phase seam in io.rs, including launch/finish methods
        // which have no fault-injectable Phase variant. Helpers remain false.
        assert_eq!(
            row["oracle"] == "true",
            owner.host_method,
            "{key:?}: oracle must agree with ProductionIo's HostIo seam"
        );
        if owner.host_method {
            let syntax = syn::parse_file(
                &fs::read_to_string(repo().join("crates/tine-store/src/page_host/io.rs")).unwrap(),
            )
            .unwrap();
            let name = key.1.strip_prefix("ProductionIo::").unwrap();
            assert!(syntax.items.iter().any(|item| matches!(item, syn::Item::Trait(t) if t.ident == "HostIo" && t.items.iter().any(|i| matches!(i, syn::TraitItem::Fn(f) if f.sig.ident == name)))), "{key:?}: no HostIo phase method");
        }
    }
    assert!(
        errors.is_empty(),
        "I-1: review write_sites.toml; {}",
        errors.join("\n")
    );
}

#[test]
fn ledger_paths_are_portable_without_hiding_stale_or_changed_sites() {
    let _integration_guard = crate::integration_lock();
    // read_dir can append native separators to a slash-spelled scan root.
    // Exercise both Rust and Swift spellings even on a Unix test host.
    for (portable, native) in [
        (
            "crates/tine-store/src/page_host/production.rs",
            r"crates/tine-store/src\page_host\production.rs",
        ),
        (
            "src-tauri/ios-app/Shared/ShareInbox.swift",
            r"src-tauri\ios-app\Shared\ShareInbox.swift",
        ),
    ] {
        for path in [portable, native] {
            let file = ledger_path(Path::new(path));
            assert_eq!(file, portable);
            let found = BTreeMap::from([(
                (file, "writer".into()),
                Owner {
                    operations: BTreeMap::from([("fs::write".into(), 1)]),
                    ..Owner::default()
                },
            )]);
            let row = Row::from([
                ("file".into(), portable.into()),
                ("fn".into(), "writer".into()),
                ("mutations".into(), "fs::write=1".into()),
            ]);
            assert!(scope_errors(&found, &[row.clone()]).is_empty());
            assert!(scope_errors(&found, &[])[0].contains("new mutation site outside the ledger"));
            let mut stale = row.clone();
            stale.insert("fn".into(), "removed_writer".into());
            assert!(scope_errors(&found, &[stale])[0].contains("stale row"));
            let mut changed = row;
            changed.insert("mutations".into(), "fs::write=2".into());
            assert!(scope_errors(&found, &[changed])[0].contains("changed mutations"));
        }
    }
}

#[test]
fn ledger_rejects_missing_stale_and_changed_sites_and_dangling_references() {
    let _integration_guard = crate::integration_lock();
    let mut scan = Mutations::default();
    scan.visit_file(&syn::parse_file("fn rogue() { std::fs::write(\"a\", b\"b\"); }").unwrap());
    let found = scan
        .owners
        .into_iter()
        .map(|(name, owner)| (("new.rs".into(), name), owner))
        .collect();
    assert!(scope_errors(&found, &[])[0].contains("new mutation site outside the ledger"));
    let mut row = Row::from([
        ("file".into(), "new.rs".into()),
        ("fn".into(), "gone".into()),
        ("mutations".into(), "fs::write=1".into()),
    ]);
    assert!(scope_errors(&found, &[row.clone()])[0].contains("stale row"));
    row.insert("fn".into(), "rogue".into());
    row.insert("mutations".into(), "fs::write=2".into());
    assert!(scope_errors(&found, &[row])[0].contains("changed mutations"));
    assert!(!reference_exists(
        "crates/tine-store/src/model.rs#nonexistent_guard",
        false
    ));
    assert!(
        !reference_exists("crates/tine-store/src/model.rs#atomic_write_new", true),
        "a production function is not test evidence"
    );
}

#[test]
fn census_detects_raw_handle_and_crate_primitives_and_respects_cfg() {
    let _integration_guard = crate::integration_lock();
    let planted = syn::parse_file(
        r#"
        #[cfg(not(test))] fn rogue() { std::fs::write("page.md", b"lost"); }
        fn wrappers(dir: &cap_std::fs::Dir) {
            dir.create("page.md"); dir.rename("a", dir, "b"); dir.remove_file("a");
            PreparedWrite::with_hooks(); DirectoryCreation::new(); move_file_noreplace();
            rename_noreplace_dir(); rename_replace(); create_dir_all_durable();
            create_dir_all_with_sync(); atomic_copy_new(); atomic_copy_file_new();
            graph.withdraw_file_to_conflict_if(); prepared.publish();
            create_graph_dir(); create_graph_dir_all(); remove_graph_file(); remove_graph_dir();
            remove_publish_stage(); rename_publish_entry(); remove_restore_temp(); rename_spelling_replace();
        }
        #[cfg(test)] mod tests { fn hidden() { std::fs::write("a", b"b"); } }
        #[cfg(feature = "test-faults")] impl Hidden { fn hidden() { std::fs::write("a", b"b"); } }
        #[cfg(any(feature = "test-faults", target_os = "linux"))]
        fn shipped() { std::fs::create_dir_all("a"); }
    "#,
    )
    .unwrap();
    let mut scan = Mutations::default();
    scan.visit_file(&planted);
    assert_eq!(scan.owners["rogue"].operations["fs::write"], 1);
    assert_eq!(scan.owners["wrappers"].operations.len(), 22);
    assert_eq!(scan.owners["shipped"].operations["fs::create_dir_all"], 1);
    assert!(!scan.owners.keys().any(|name| name.contains("hidden")));
}

#[test]
fn oracle_rows_cover_the_mutating_phase_list_and_exceptions_are_pinned() {
    let _integration_guard = crate::integration_lock();
    let syntax = syn::parse_file(
        &fs::read_to_string(repo().join("crates/tine-store/src/page_host/io.rs")).unwrap(),
    )
    .unwrap();
    let phases = syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Enum(e) if e.ident == "Phase" => Some(&e.variants),
            _ => None,
        })
        .expect("HostIo Phase enum");
    let rows = ledger(include_str!("write_sites.toml"));
    let snake = regex::Regex::new(r"([a-z])([A-Z])").unwrap();
    for phase in phases {
        let name = phase.ident.to_string();
        if name == "Read" {
            continue;
        } // the sole non-mutating fault phase
        let method = if name == "CustodyList" {
            "custody_markers".into()
        } else {
            snake.replace_all(&name, "${1}_${2}").to_lowercase()
        };
        assert!(
            rows.iter().any(
                |row| row["file"] == "crates/tine-store/src/page_host/production.rs"
                    && row["fn"] == format!("ProductionIo::{method}")
                    && row["oracle"] == "true"
            ),
            "Phase::{name}: missing oracle mutation owner {method}"
        );
    }
    let roots: Vec<_> = rows
        .iter()
        .filter(|r| r["domain"] == "create-root")
        .map(|r| (r["file"].as_str(), r["fn"].as_str()))
        .collect();
    assert_eq!(
        roots,
        [("crates/tine-store/src/store.rs", "Store::create_graph")]
    );
    let containers: BTreeSet<_> = rows
        .iter()
        .filter(|r| r["domain"] == "container")
        .map(|r| r["file"].as_str())
        .collect();
    assert_eq!(
        containers,
        BTreeSet::from([
            "src-tauri/ios-folder-picker-native/ios/Sources/GraphFolderPickerPlugin.swift"
        ])
    );
}
