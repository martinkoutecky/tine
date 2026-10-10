//! The writer call-site guard (STEP3 §7, R6): every production function that
//! writes graph files through the store runs as a retained writer under a
//! host reservation (`retained::reserved`), or around the host's restore stop
//! (`restore_hosted`), or is called only from functions that do; or it writes
//! no page file and is exempt with its reason. A new writer call site fails
//! here, naming the rule and an exemplar.

use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use syn::visit::Visit;

const RULE: &str = "STEP3 §7 (R6): a production page writer runs under a page-host reservation";
const EXEMPLAR: &str = "crates/tine-graph-features/src/conflicts.rs fold_pair";
/// Store calls that write graph files.
const WRITES: &[&str] = &["transaction", "save_pages", "restore"];
/// Calls that route a writer through the host.
const ROUTES: &[&str] = &["reserved", "restore_hosted"];

/// Writers of no page file, or old-engine paths lane 3b deletes, by
/// `file::fn`, each with its reason.
const EXEMPT: &[(&str, &str)] = &[
    (
        "crates/tine-graph-features/src/assets.rs::create_unique",
        "creates a new asset file; no page",
    ),
    (
        "crates/tine-graph-features/src/assets.rs::trash_asset",
        "trashes an asset file and rewrites no page (the census's asset trash rewrites a page)",
    ),
    (
        "crates/tine-graph-features/src/pdf.rs::write_pdf_area_image",
        "writes a PDF crop image asset; no page",
    ),
    (
        "crates/tine-graph-features/src/pdf.rs::rollback_pdf_area_file",
        "trashes a PDF crop image asset; no page",
    ),
    (
        "crates/tine-graph-features/src/config.rs::update",
        "writes config.edn; no page",
    ),
    (
        "crates/tine-graph-features/src/custom_css.rs::ensure_custom_css",
        "writes custom.css; no page",
    ),
    (
        "crates/tine-graph-features/src/guide.rs::create_if_absent",
        "creates absent Guide files only; a host page with no file meets it as an external create",
    ),
    (
        "crates/tine-graph-features/src/pages.rs::save_pages",
        "the old engine's save path; lane 3b deletes it (STEP3: no production save_pages)",
    ),
];

#[derive(Default)]
struct Function {
    writes: bool,
    calls: BTreeSet<String>,
}

fn calls(tokens: proc_macro2::TokenStream, out: &mut Function) {
    use proc_macro2::{Delimiter, TokenTree};
    let tokens: Vec<_> = tokens.into_iter().collect();
    for (i, token) in tokens.iter().enumerate() {
        match token {
            TokenTree::Group(group) => calls(group.stream(), out),
            TokenTree::Ident(name) => {
                let Some(TokenTree::Group(args)) = tokens.get(i + 1) else {
                    continue;
                };
                if args.delimiter() != Delimiter::Parenthesis {
                    continue;
                }
                let name = name.to_string();
                let method = matches!(tokens.get(i.wrapping_sub(1)),
                    Some(TokenTree::Punct(dot)) if dot.as_char() == '.');
                out.writes |= method && WRITES.contains(&name.as_str());
                out.calls.insert(name);
            }
            _ => {}
        }
    }
}

fn is_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path()
            .segments
            .last()
            .is_some_and(|s| s.ident == "test")
            || (attr.path().is_ident("cfg")
                && attr.meta.to_token_stream().to_string().contains("test"))
    })
}

struct Scan<'a> {
    file: &'a str,
    in_test: bool,
    functions: &'a mut BTreeMap<String, Function>,
}

impl Scan<'_> {
    fn record(&mut self, name: &syn::Ident, attrs: &[syn::Attribute], body: &syn::Block) {
        if self.in_test || is_test(attrs) {
            return;
        }
        let entry = self
            .functions
            .entry(format!("{}::{name}", self.file))
            .or_default();
        let mut found = Function::default();
        calls(body.to_token_stream(), &mut found);
        entry.writes |= found.writes;
        entry.calls.extend(found.calls);
    }
}

impl<'ast> Visit<'ast> for Scan<'_> {
    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        let previous = self.in_test;
        self.in_test |= is_test(&module.attrs);
        syn::visit::visit_item_mod(self, module);
        self.in_test = previous;
    }
    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        self.record(&function.sig.ident, &function.attrs, &function.block);
    }
    fn visit_impl_item_fn(&mut self, function: &'ast syn::ImplItemFn) {
        self.record(&function.sig.ident, &function.attrs, &function.block);
    }
}

fn scan(file: &str, source: &str, functions: &mut BTreeMap<String, Function>) {
    let parsed = syn::parse_file(source).expect("Rust source must parse");
    Scan {
        file,
        in_test: false,
        functions,
    }
    .visit_file(&parsed);
}

/// Writers that are neither routed nor exempt.
fn unrouted(functions: &BTreeMap<String, Function>, exempt: &[(&str, &str)]) -> Vec<String> {
    fn name(key: &str) -> &str {
        key.rsplit("::").next().unwrap()
    }
    fn routed(
        key: &str,
        functions: &BTreeMap<String, Function>,
        seen: &mut BTreeSet<String>,
    ) -> bool {
        if !seen.insert(key.to_owned()) {
            return true;
        }
        if functions[key]
            .calls
            .iter()
            .any(|call| ROUTES.contains(&call.as_str()))
        {
            return true;
        }
        let callers: Vec<&String> = functions
            .iter()
            .filter(|(caller, f)| *caller != key && f.calls.contains(name(key)))
            .map(|(caller, _)| caller)
            .collect();
        !callers.is_empty() && callers.iter().all(|caller| routed(caller, functions, seen))
    }
    functions
        .iter()
        .filter(|(key, f)| f.writes && !exempt.iter().any(|(e, _)| e == key))
        .filter(|(key, _)| !routed(key, functions, &mut BTreeSet::new()))
        .map(|(key, _)| key.clone())
        .collect()
}

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn every_production_page_writer_runs_under_a_host_reservation() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in ["crates/tine-graph-features/src", "src-tauri/src"] {
        rust_files(&root.join(dir), &mut files);
    }
    let mut functions = BTreeMap::new();
    for path in files {
        let file = path
            .strip_prefix(&root)
            .unwrap()
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        if file.ends_with("_tests.rs") {
            continue;
        }
        scan(
            &file,
            &std::fs::read_to_string(&path).unwrap(),
            &mut functions,
        );
    }
    for (key, reason) in EXEMPT {
        assert!(
            functions.get(*key).is_some_and(|f| f.writes),
            "{RULE}: the exemption {key} ({reason}) names no writer; drop it"
        );
    }
    // The census (STEP3 §7) reaches the host.
    for writer in [
        "crates/tine-graph-features/src/pages.rs::rename_page_after_inventory",
        "crates/tine-graph-features/src/pages.rs::host_rename",
        "crates/tine-graph-features/src/pages.rs::merge_pages",
        "crates/tine-graph-features/src/pages.rs::rename_file_to_page",
        "crates/tine-graph-features/src/pages.rs::delete_page_expected",
        "crates/tine-graph-features/src/conflicts.rs::fold_pair",
        "crates/tine-graph-features/src/conflicts.rs::trash_sync_conflict",
        "crates/tine-graph-features/src/conflicts.rs::resolve_vcs_marker_conflict",
        "crates/tine-graph-features/src/journals.rs::migrate_journal_filenames",
        "crates/tine-graph-features/src/journals.rs::trash_journal_file",
        "crates/tine-graph-features/src/pdf.rs::write_highlights",
        "crates/tine-graph-features/src/live_conflict.rs::resolve_live_conflict",
        "src-tauri/src/backup/restore.rs::restore_backup",
    ] {
        assert!(
            functions
                .get(writer)
                .is_some_and(|f| f.calls.iter().any(|call| ROUTES.contains(&call.as_str()))),
            "{RULE}: census writer {writer} no longer reaches the host; exemplar {EXEMPLAR}"
        );
    }
    let found = unrouted(&functions, EXEMPT);
    assert!(
        found.is_empty(),
        "{RULE}: {} write through the store outside a reservation. Call \
         `retained::reserved` with the pages they touch (exemplar {EXEMPLAR}), or \
         exempt one in EXEMPT with its reason only if it writes no page file",
        found.join(", ")
    );
}

#[test]
fn a_planted_writer_outside_a_reservation_fails_the_guard() {
    let planted = r#"
        fn helper(store: &Store) { store.transaction(None).commit(); }
        pub fn routed(store: &Store) { reserved(None, Input::Flush, d, r, |_| helper(store)); }
        pub fn bare(store: &Store) { let tx = store.transaction(None); }
        #[cfg(test)] mod tests { fn t(store: &Store) { store.transaction(None); } }
    "#;
    let mut functions = BTreeMap::new();
    scan("planted.rs", planted, &mut functions);
    assert_eq!(unrouted(&functions, &[]), ["planted.rs::bare"]);
    let reached = r#"
        fn helper(store: &Store) { store.transaction(None).commit(); }
        pub fn bare(store: &Store) { helper(store); }
    "#;
    let mut functions = BTreeMap::new();
    scan("planted.rs", reached, &mut functions);
    assert_eq!(unrouted(&functions, &[]), ["planted.rs::helper"]);
}
