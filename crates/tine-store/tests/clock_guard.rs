//! The crates of the browser build read time only through `tine_core::clock`
//! (browser core slice 2, D-20261122-WEX): `std::time::Instant::now` and
//! `SystemTime::now` panic on `wasm32-unknown-unknown`, so one std clock read
//! left in code the browser runs is an outage there that no native test sees.
//! Walks each crate's module tree from `lib.rs`, skipping test-only and
//! native-only (`not(target_arch = "wasm32")`) items, and names every std
//! clock use left.
use quote::ToTokens;
use std::path::{Path, PathBuf};
use syn::visit::{self, Visit};

const CRATES: [&str; 6] = [
    "tine-core",
    "tine-outline",
    "tine-search",
    "tine-guide",
    "tine-store",
    "tine-graph-features",
];

/// Off the browser build: test-only code, test fault hooks, native-only code.
fn skipped(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("cfg") {
            return false;
        }
        let text = attr.meta.to_token_stream().to_string().replace(' ', "");
        let test = text == "cfg(test)"
            || text.starts_with("cfg(any(test")
            || text.starts_with("cfg(all(test")
            || text.contains("feature=\"test-faults\"") && !text.contains("not(feature")
            || text.contains("feature=\"test-counters\"");
        test || text.contains("not(target_arch=\"wasm32\")") && !text.starts_with("cfg(not(not")
    })
}

struct Uses<'a> {
    file: &'a Path,
    found: Vec<String>,
    mods: Vec<(String, Option<String>)>,
}

impl Uses<'_> {
    fn flag(&mut self, what: &str) {
        self.found.push(format!("{}: {what}", self.file.display()));
    }
}

fn segments(path: &syn::Path) -> Vec<String> {
    path.segments.iter().map(|s| s.ident.to_string()).collect()
}

impl<'ast> Visit<'ast> for Uses<'_> {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        let attrs: &[syn::Attribute] = match item {
            syn::Item::Fn(i) => &i.attrs,
            syn::Item::Impl(i) => &i.attrs,
            syn::Item::Mod(i) => &i.attrs,
            syn::Item::Use(i) => &i.attrs,
            syn::Item::Const(i) => &i.attrs,
            syn::Item::Static(i) => &i.attrs,
            syn::Item::Struct(i) => &i.attrs,
            syn::Item::Enum(i) => &i.attrs,
            syn::Item::Trait(i) => &i.attrs,
            syn::Item::Type(i) => &i.attrs,
            syn::Item::Macro(i) => &i.attrs,
            _ => &[],
        };
        if skipped(attrs) {
            return;
        }
        if let syn::Item::Mod(module) = item {
            if module.content.is_none() {
                let path = module.attrs.iter().find_map(|attr| {
                    let syn::Meta::NameValue(value) = &attr.meta else {
                        return None;
                    };
                    if !value.path.is_ident("path") {
                        return None;
                    }
                    let syn::Expr::Lit(lit) = &value.value else {
                        return None;
                    };
                    let syn::Lit::Str(text) = &lit.lit else {
                        return None;
                    };
                    Some(text.value())
                });
                self.mods.push((module.ident.to_string(), path));
            }
        }
        visit::visit_item(self, item);
    }
    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        let attrs: &[syn::Attribute] = match item {
            syn::ImplItem::Fn(i) => &i.attrs,
            syn::ImplItem::Const(i) => &i.attrs,
            _ => &[],
        };
        if !skipped(attrs) {
            visit::visit_impl_item(self, item);
        }
    }
    fn visit_stmt(&mut self, stmt: &'ast syn::Stmt) {
        let attrs: &[syn::Attribute] = match stmt {
            syn::Stmt::Local(local) => &local.attrs,
            syn::Stmt::Expr(syn::Expr::Block(block), _) => &block.attrs,
            syn::Stmt::Expr(syn::Expr::Call(call), _) => &call.attrs,
            syn::Stmt::Expr(syn::Expr::MethodCall(call), _) => &call.attrs,
            syn::Stmt::Macro(mac) => &mac.attrs,
            _ => &[],
        };
        if !skipped(attrs) {
            visit::visit_stmt(self, stmt);
        }
    }
    fn visit_expr_block(&mut self, expr: &'ast syn::ExprBlock) {
        if !skipped(&expr.attrs) {
            visit::visit_expr_block(self, expr);
        }
    }
    fn visit_use_tree(&mut self, tree: &'ast syn::UseTree) {
        fn walk(tree: &syn::UseTree, prefix: &mut Vec<String>, out: &mut Vec<String>) {
            match tree {
                syn::UseTree::Path(path) => {
                    prefix.push(path.ident.to_string());
                    walk(&path.tree, prefix, out);
                    prefix.pop();
                }
                syn::UseTree::Name(name) => {
                    let mut full = prefix.clone();
                    full.push(name.ident.to_string());
                    out.push(full.join("::"));
                }
                syn::UseTree::Rename(name) => {
                    let mut full = prefix.clone();
                    full.push(name.ident.to_string());
                    out.push(full.join("::"));
                }
                syn::UseTree::Group(group) => {
                    for item in &group.items {
                        walk(item, prefix, out);
                    }
                }
                syn::UseTree::Glob(_) => out.push(format!("{}::*", prefix.join("::"))),
            }
        }
        let mut names = Vec::new();
        walk(tree, &mut Vec::new(), &mut names);
        for name in names {
            if name == "std::time::Instant" || name == "std::time::*" {
                self.flag(&format!("use {name}"));
            }
        }
    }
    fn visit_path(&mut self, path: &'ast syn::Path) {
        let names = segments(path);
        let std_instant = names.len() >= 3 && names[..3] == ["std", "time", "Instant"];
        let system_now = names.windows(2).any(|pair| pair == ["SystemTime", "now"]);
        if std_instant || system_now {
            self.flag(&names.join("::"));
        }
        visit::visit_path(self, path);
    }
}

/// The file of module `name` declared in `file` (`#[path]` relative to the
/// declaring file's directory, as rustc resolves it for a non-mod.rs file
/// declaring an out-of-line module with an explicit path).
fn module_file(file: &Path, name: &str, path: Option<&str>) -> Option<PathBuf> {
    let dir = file.parent().unwrap();
    if let Some(path) = path {
        return Some(dir.join(path));
    }
    let stem = file.file_stem().unwrap().to_str().unwrap();
    let base = if matches!(stem, "lib" | "mod" | "main") {
        dir.to_path_buf()
    } else {
        dir.join(stem)
    };
    [
        base.join(format!("{name}.rs")),
        base.join(name).join("mod.rs"),
    ]
    .into_iter()
    .find(|candidate| candidate.exists())
}

#[test]
fn the_browser_crates_read_time_only_through_the_clock() {
    let _guard = crate::integration_lock();
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut found = Vec::new();
    let mut files = 0;
    for name in CRATES {
        let mut pending = vec![crates.join(name).join("src/lib.rs")];
        while let Some(file) = pending.pop() {
            files += 1;
            let text = std::fs::read_to_string(&file).unwrap();
            let syntax = syn::parse_file(&text).unwrap();
            let shown = file.strip_prefix(&crates).unwrap_or(&file).to_path_buf();
            let mut uses = Uses {
                file: &shown,
                found: Vec::new(),
                mods: Vec::new(),
            };
            uses.visit_file(&syntax);
            if !shown.ends_with("tine-core/src/clock.rs") {
                found.extend(uses.found);
            }
            for (module, path) in uses.mods {
                pending.extend(module_file(&file, &module, path.as_deref()));
            }
        }
    }
    assert!(
        files > 100,
        "the walk reached the crates' modules ({files} files)"
    );
    assert!(
        found.is_empty(),
        "std clock reads in code the browser build runs; use tine_core::clock::{{Instant, system_now}}:\n{}",
        found.join("\n")
    );
}
