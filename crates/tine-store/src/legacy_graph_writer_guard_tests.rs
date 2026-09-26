use quote::ToTokens;
use std::path::{Path, PathBuf};
use syn::visit::Visit;

const RULE: &str = "tests write through Store; one composition";
const LEGACY_WRITERS: &[&str] = &[
    "save_page",
    "force_save_page",
    "rename_page",
    "rename_page_expected",
    "delete_page",
    "delete_page_expected",
    "merge_pages",
    "rename_file_to_page",
    "save_asset",
    "import_asset",
    "import_asset_file",
    "write_highlights",
    "write_pdf_area_image",
    "write_pdf_view_state",
    "open_pdf",
    "trash_asset",
    "sync_file",
    "forget_file",
    "cache_upsert",
    "warm_parsed_pages",
    "warm_cache_cancellable",
    "invalidate_cache",
    "sync_file_internal",
    "forget_file_internal",
    "transaction_publish_page",
    "create_markdown_page_if_absent",
    "migrate_journal_filenames",
];

// These specific tests exercise Graph methods that still have production
// callers: warm_cache_cancellable in store.rs/watch.rs; invalidate_cache in
// restore.rs/transaction.rs; sync_file_internal and forget_file_internal in
// watch.rs; transaction_publish_page in transaction.rs. All other test writes
// must go through Store. Transaction::save_page is a production write boundary.
const ALLOWED_GRAPH_UNIT_CALLS: &[(&str, &str, &str)] = &[
    (
        "crates/tine-store/src/gh221_malformed_html_tests.rs",
        "gh221_malformed_html_fragment_indexes_without_panic",
        "warm_cache_cancellable",
    ),
    (
        "crates/tine-store/src/issue137_investigation_tests.rs",
        "issue137_current_contract_snapshot_uses_real_parser_and_engine",
        "invalidate_cache",
    ),
    (
        "crates/tine-store/src/issue137_investigation_tests.rs",
        "issue232_cache_rebuild_preserves_block_identity",
        "invalidate_cache",
    ),
    (
        "crates/tine-store/src/model.rs",
        "search_cache_isolates_one_page_projection_panic",
        "invalidate_cache",
    ),
    (
        "crates/tine-store/src/model.rs",
        "search_cache_isolates_one_page_projection_panic",
        "warm_cache_cancellable",
    ),
    (
        "crates/tine-store/src/model.rs",
        "find_entry_cache_avoids_per_lookup_list_md_fanout",
        "warm_cache_cancellable",
    ),
    (
        "crates/tine-store/src/model.rs",
        "parsed_doc_cache_index_avoids_warm_open_linear_scans",
        "warm_cache_cancellable",
    ),
    (
        "crates/tine-store/src/model.rs",
        "find_entry_cache_invalidated_by_cold_sync_file",
        "sync_file_internal",
    ),
    (
        "crates/tine-store/src/model.rs",
        "with_pages_snapshot_does_not_block_cache_upsert",
        "warm_cache_cancellable",
    ),
    (
        "crates/tine-store/src/model.rs",
        "with_pages_snapshot_does_not_block_cache_upsert",
        "transaction_publish_page",
    ),
    (
        "crates/tine-store/src/model.rs",
        "with_pages_snapshot_survives_concurrent_upsert",
        "warm_cache_cancellable",
    ),
    (
        "crates/tine-store/src/model.rs",
        "with_pages_snapshot_survives_concurrent_upsert",
        "transaction_publish_page",
    ),
    (
        "crates/tine-store/src/model.rs",
        "future_journals_are_feed_only_excluded_but_keep_raw_identity",
        "warm_cache_cancellable",
    ),
    (
        "crates/tine-store/src/model.rs",
        "readonly_org_unchanged_does_not_reconcile",
        "warm_cache_cancellable",
    ),
    (
        "crates/tine-store/src/model.rs",
        "readonly_org_unchanged_does_not_reconcile",
        "sync_file_internal",
    ),
    (
        "crates/tine-store/src/model.rs",
        "nfd_alias_resolves_and_canonical_equivalent_alias_cannot_shadow_real_page",
        "warm_cache_cancellable",
    ),
    (
        "crates/tine-store/src/model.rs",
        "warmed_duplicate_name_cache_keeps_physical_owners_distinct",
        "warm_cache_cancellable",
    ),
    (
        "crates/tine-store/src/model.rs",
        "warmed_duplicate_name_cache_keeps_physical_owners_distinct",
        "sync_file_internal",
    ),
    (
        "crates/tine-store/src/model.rs",
        "forget_file_evicts_only_the_deleted_duplicate_path",
        "warm_cache_cancellable",
    ),
    (
        "crates/tine-store/src/model.rs",
        "forget_file_evicts_only_the_deleted_duplicate_path",
        "forget_file_internal",
    ),
];
fn is_transaction_save(tokens: &[proc_macro2::TokenTree], dot: usize) -> bool {
    matches!(tokens.get(dot.wrapping_sub(1)), Some(proc_macro2::TokenTree::Ident(id)) if id == "tx")
}

fn calls(tokens: proc_macro2::TokenStream, found: &mut Vec<String>) {
    use proc_macro2::{Delimiter, TokenTree};
    let tokens: Vec<_> = tokens.into_iter().collect();
    for (i, token) in tokens.iter().enumerate() {
        if let TokenTree::Group(group) = token {
            calls(group.stream(), found);
        }
        if !matches!(token, TokenTree::Punct(dot) if dot.as_char() == '.') {
            continue;
        }
        let (Some(TokenTree::Ident(method)), Some(TokenTree::Group(args))) =
            (tokens.get(i + 1), tokens.get(i + 2))
        else {
            continue;
        };
        if args.delimiter() != Delimiter::Parenthesis {
            continue;
        }
        let name = method.to_string();
        if LEGACY_WRITERS.contains(&name.as_str())
            && !(name == "save_page" && is_transaction_save(&tokens, i))
        {
            found.push(name);
        }
    }
}

fn is_test_attr(attr: &syn::Attribute) -> bool {
    attr.path()
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "test")
}

struct TestCalls {
    source_name: String,
    in_test_module: bool,
    found: Vec<String>,
}

impl TestCalls {
    fn record(&mut self, test: &str, tokens: proc_macro2::TokenStream) {
        let mut found = Vec::new();
        calls(tokens, &mut found);
        self.found.extend(found.into_iter().filter_map(|method| {
            (!ALLOWED_GRAPH_UNIT_CALLS.contains(&(
                self.source_name.as_str(),
                test,
                method.as_str(),
            )))
            .then(|| format!("{test}: {method}"))
        }));
    }
}

impl<'ast> Visit<'ast> for TestCalls {
    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        let previous = self.in_test_module;
        self.in_test_module |= module.ident == "tests"
            || module.attrs.iter().any(|attr| {
                attr.path().is_ident("cfg")
                    && attr.meta.to_token_stream().to_string().contains("test")
            });
        syn::visit::visit_item_mod(self, module);
        self.in_test_module = previous;
    }

    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        if self.in_test_module || function.attrs.iter().any(is_test_attr) {
            self.record(
                &function.sig.ident.to_string(),
                function.block.to_token_stream(),
            );
        }
        syn::visit::visit_item_fn(self, function);
    }

    fn visit_impl_item_fn(&mut self, function: &'ast syn::ImplItemFn) {
        if self.in_test_module || function.attrs.iter().any(is_test_attr) {
            self.record(
                &function.sig.ident.to_string(),
                function.block.to_token_stream(),
            );
        }
        syn::visit::visit_impl_item_fn(self, function);
    }
}

fn violations(source: &str, source_name: &str, test_file: bool) -> Vec<String> {
    let parsed = syn::parse_file(source).expect("Rust source must parse");
    let mut visitor = TestCalls {
        source_name: source_name.to_owned(),
        in_test_module: test_file,
        found: Vec::new(),
    };
    visitor.visit_file(&parsed);
    visitor.found
}

fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("source directory must be readable") {
        let path = entry.expect("source entry must be readable").path();
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
}

#[test]
fn test_writers_use_store_or_a_production_graph_boundary() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for relative in [
        "crates/tine-store/src",
        "crates/tine-graph-features/src",
        "src-tauri/src",
    ] {
        rust_files(&root.join(relative), &mut files);
    }
    let mut found = Vec::new();
    for file in files {
        let source = std::fs::read_to_string(&file).expect("source must be readable");
        let test_file = file
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with("_tests.rs"));
        found.extend(
            violations(
                &source,
                &file.strip_prefix(&root).unwrap().to_string_lossy(),
                test_file,
            )
            .into_iter()
            .map(|call| format!("{}: {call}", file.display())),
        );
    }
    assert!(found.is_empty(), "{RULE}: {}", found.join(", "));
}

#[test]
fn planted_legacy_writer_fails_the_guard() {
    let planted = r#"
        #[cfg(test)] mod tests {
            #[test] fn violation() {
                assert!(g.save_page(&page, None).is_ok());
            }
        }
    "#;
    assert_eq!(
        violations(planted, "planted.rs", false),
        ["violation: save_page"]
    );
}
