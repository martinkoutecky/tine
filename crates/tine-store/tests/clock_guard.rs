//! The crates of the browser build read time only through `tine_core::clock`
//! (browser core slice 2, D-20261122-WEX): `std::time::Instant::now` and
//! `SystemTime::now` panic on `wasm32-unknown-unknown`, so one std clock read
//! left in code the browser runs is an outage there that no native test sees.
//! Walks each crate's module tree from `lib.rs` (every file a `#[path]` or
//! `cfg_attr(..., path)` may choose), skipping only items whose cfg is false
//! in the browser build, and names each std clock path, `use` or rename left,
//! macro arguments included. Not covered: a crate-root rename (`use std as
//! s`) and clock reads in dependencies.
use quote::ToTokens;
use std::path::{Path, PathBuf};
use syn::visit::{self, Visit};

const CRATES: [&str; 7] = [
    "lsdoc-wasm",
    "tine-core",
    "tine-outline",
    "tine-search",
    "tine-guide",
    "tine-store",
    "tine-graph-features",
];

/// A cfg predicate's value in the browser build (wasm32, not test, no test
/// features): `None` when it depends on what this guard does not know
/// (another feature, `debug_assertions`), which keeps the item in the walk.
fn browser_cfg(meta: &syn::Meta) -> Option<bool> {
    let all = |list: &syn::MetaList, any: bool| -> Option<bool> {
        let items = list
            .parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            )
            .ok()?;
        let values: Vec<_> = items.iter().map(browser_cfg).collect();
        if values.contains(&Some(any)) {
            Some(any)
        } else if values.iter().all(|value| *value == Some(!any)) {
            Some(!any)
        } else {
            None
        }
    };
    match meta {
        syn::Meta::Path(path) if path.is_ident("test") => Some(false),
        syn::Meta::Path(path) if path.is_ident("unix") || path.is_ident("windows") => Some(false),
        syn::Meta::Path(_) => None,
        syn::Meta::List(list) if list.path.is_ident("any") => all(list, true),
        syn::Meta::List(list) if list.path.is_ident("all") => all(list, false),
        syn::Meta::List(list) if list.path.is_ident("not") => {
            browser_cfg(&list.parse_args::<syn::Meta>().ok()?).map(|value| !value)
        }
        syn::Meta::List(_) => None,
        syn::Meta::NameValue(pair) => {
            let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(value),
                ..
            }) = &pair.value
            else {
                return None;
            };
            let value = value.value();
            let key = pair.path.get_ident()?.to_string();
            match key.as_str() {
                "target_arch" => Some(value == "wasm32"),
                "target_os" => Some(value == "unknown"),
                "target_family" => Some(value == "wasm"),
                "feature" if matches!(value.as_str(), "test-faults" | "test-counters") => {
                    Some(false)
                }
                _ => None,
            }
        }
    }
}

/// Off the browser build: an item with a `cfg` that is false there.
fn skipped(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .parse_args::<syn::Meta>()
                .ok()
                .and_then(|meta| browser_cfg(&meta))
                == Some(false)
    })
}

struct Uses<'a> {
    file: &'a Path,
    found: Vec<String>,
    mods: Vec<(String, Vec<String>)>,
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
                // Every file the module may be: `#[path]`, and each
                // `cfg_attr(..., path = ...)` whatever its condition.
                let path_of = |meta: &syn::Meta| match meta {
                    syn::Meta::NameValue(syn::MetaNameValue {
                        path,
                        value:
                            syn::Expr::Lit(syn::ExprLit {
                                lit: syn::Lit::Str(text),
                                ..
                            }),
                        ..
                    }) if path.is_ident("path") => Some(text.value()),
                    _ => None,
                };
                // A `cfg_attr`'s attributes after its condition, nested ones too.
                fn attr_paths(
                    meta: &syn::Meta,
                    path_of: &dyn Fn(&syn::Meta) -> Option<String>,
                    out: &mut Vec<String>,
                ) {
                    match meta {
                        syn::Meta::List(list) if list.path.is_ident("cfg_attr") => {
                            let metas = list.parse_args_with(
                                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
                            );
                            for inner in metas.into_iter().flatten().skip(1) {
                                attr_paths(&inner, path_of, out);
                            }
                        }
                        _ => out.extend(path_of(meta)),
                    }
                }
                let mut paths = Vec::new();
                for attr in &module.attrs {
                    attr_paths(&attr.meta, &path_of, &mut paths);
                }
                self.mods.push((module.ident.to_string(), paths));
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
            let module = name == "std::time" || name == "std::time::self";
            if module || name == "std::time::Instant" || name == "std::time::*" {
                self.flag(&format!("use {name}"));
            }
        }
        // A renamed `std::time` or item of it hides later reads from the path
        // check (`use std::time as t; t::SystemTime::now()`).
        fn renames(tree: &syn::UseTree, prefix: &mut Vec<String>, out: &mut Vec<String>) {
            match tree {
                syn::UseTree::Path(path) => {
                    prefix.push(path.ident.to_string());
                    renames(&path.tree, prefix, out);
                    prefix.pop();
                }
                syn::UseTree::Rename(name) => {
                    let full = [prefix.join("::"), name.ident.to_string()].join("::");
                    if full == "std::time" || full.starts_with("std::time::") {
                        out.push(format!("use {full} as {}", name.rename));
                    }
                }
                syn::UseTree::Group(group) => {
                    for item in &group.items {
                        renames(item, prefix, out);
                    }
                }
                syn::UseTree::Name(_) | syn::UseTree::Glob(_) => {}
            }
        }
        let mut renamed = Vec::new();
        renames(tree, &mut Vec::new(), &mut renamed);
        for rename in renamed {
            self.flag(&rename);
        }
    }
    /// Macro arguments are tokens, not syntax: their identifiers in order
    /// carry the same two paths (`std time Instant`, `SystemTime now`).
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        fn idents(tokens: proc_macro2::TokenStream, out: &mut Vec<String>) {
            for token in tokens {
                match token {
                    proc_macro2::TokenTree::Ident(ident) => out.push(ident.to_string()),
                    proc_macro2::TokenTree::Group(group) => idents(group.stream(), out),
                    _ => {}
                }
            }
        }
        let mut names = Vec::new();
        idents(mac.tokens.clone(), &mut names);
        if names.windows(3).any(|w| w == ["std", "time", "Instant"])
            || names.windows(2).any(|w| w == ["SystemTime", "now"])
        {
            self.flag(&format!("{}!(...)", segments(&mac.path).join("::")));
        }
        visit::visit_macro(self, mac);
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

/// The files module `name` declared in `file` may be: each `path`, relative
/// to the declaring file's directory as rustc resolves it for a non-mod.rs
/// file declaring an out-of-line module with an explicit path, and the
/// default file when one exists (a `cfg_attr` path leaves it in use elsewhere).
fn module_files(file: &Path, name: &str, paths: &[String]) -> Vec<PathBuf> {
    let dir = file.parent().unwrap();
    let mut out: Vec<_> = paths.iter().map(|path| dir.join(path)).collect();
    let stem = file.file_stem().unwrap().to_str().unwrap();
    let base = if matches!(stem, "lib" | "mod" | "main") {
        dir.to_path_buf()
    } else {
        dir.join(stem)
    };
    out.extend(
        [
            base.join(format!("{name}.rs")),
            base.join(name).join("mod.rs"),
        ]
        .into_iter()
        .find(|candidate| candidate.exists()),
    );
    out
}

/// The std clock reads in the module tree from `lib`, as `file: what`, and
/// how many files the walk read.
fn scan(lib: PathBuf, shown_from: &Path) -> (Vec<String>, usize) {
    let mut found = Vec::new();
    let mut files = 0;
    let mut pending = vec![lib];
    let mut seen = std::collections::BTreeSet::new();
    while let Some(file) = pending.pop() {
        if !seen.insert(file.clone()) {
            continue;
        }
        files += 1;
        let text = std::fs::read_to_string(&file).unwrap();
        let syntax = syn::parse_file(&text).unwrap();
        let shown = file.strip_prefix(shown_from).unwrap_or(&file).to_path_buf();
        let mut uses = Uses {
            file: &shown,
            found: Vec::new(),
            mods: Vec::new(),
        };
        uses.visit_file(&syntax);
        if !shown.ends_with("tine-core/src/clock.rs") {
            found.extend(uses.found);
        }
        for (module, paths) in uses.mods {
            pending.extend(module_files(&file, &module, &paths));
        }
    }
    (found, files)
}

#[test]
fn the_browser_crates_read_time_only_through_the_clock() {
    let _guard = crate::integration_lock();
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let (mut found, mut files) = (Vec::new(), 0);
    for name in CRATES {
        let (more, read) = scan(crates.join(name).join("src/lib.rs"), &crates);
        found.extend(more);
        files += read;
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

/// Each case the guard must name, planted in a module tree of its own: a read
/// in macro arguments, a renamed `std::time`, a module chosen by `cfg_attr`,
/// and an item whose cfg mixes test with wasm32. Native-only and test-only
/// code stays out.
#[test]
fn the_clock_guard_names_each_planted_read() {
    let temp = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| std::fs::write(temp.path().join(name), text).unwrap();
    write(
        "lib.rs",
        r#"
fn in_macro() { let _ = format!("{:?}", std::time::SystemTime::now()); }
use std::time as renamed;
use std::time::{self as also_renamed};
use std::time;
#[cfg_attr(target_arch = "wasm32", path = "browser.rs")]
mod chosen;
#[cfg_attr(unix, cfg_attr(target_arch = "wasm32", path = "nested.rs"))]
mod nested;
#[cfg(any(test, target_arch = "wasm32"))]
fn mixed() { let _ = std::time::Instant::now(); }
#[cfg(not(target_arch = "wasm32"))]
fn native() { let _ = std::time::Instant::now(); }
#[cfg(all(test, target_arch = "wasm32"))]
fn test_only() { let _ = std::time::SystemTime::now(); }
"#,
    );
    write(
        "browser.rs",
        "pub fn f() { let _ = std::time::SystemTime::now(); }\n",
    );
    write(
        "nested.rs",
        "pub fn g() { let _ = std::time::SystemTime::now(); }\n",
    );
    let (mut found, files) = scan(temp.path().join("lib.rs"), temp.path());
    found.sort();
    assert_eq!(files, 3);
    assert_eq!(
        found,
        [
            "browser.rs: std::time::SystemTime::now",
            "lib.rs: format!(...)",
            "lib.rs: std::time::Instant::now",
            // `use std::time;` and `use std::time as renamed;` both name the module.
            "lib.rs: use std::time",
            "lib.rs: use std::time",
            "lib.rs: use std::time as renamed",
            "lib.rs: use std::time::self",
            "lib.rs: use std::time::self as also_renamed",
            "nested.rs: std::time::SystemTime::now",
        ]
    );
}
