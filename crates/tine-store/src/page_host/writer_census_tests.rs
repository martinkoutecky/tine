//! Every host operation records its edit kinds.
use crate::test_cfg::is_test;
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use syn::visit::Visit;

#[derive(Default)]
struct Function {
    body: String,
    calls: BTreeSet<String>,
}
fn scan(file: &str, source: &str, out: &mut BTreeMap<String, Function>) {
    struct Scan<'a> {
        file: &'a str,
        out: &'a mut BTreeMap<String, Function>,
    }
    impl<'ast> Visit<'ast> for Scan<'_> {
        fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
            if !is_test(&item.attrs) {
                syn::visit::visit_item_mod(self, item);
            }
        }
        fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
            if is_test(&item.attrs) {
                return;
            }
            let body = item.block.to_token_stream().to_string();
            let calls = body.split_whitespace().map(str::to_owned).collect();
            self.out.insert(
                format!("{}::{}", self.file, item.sig.ident),
                Function { body, calls },
            );
        }
        fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
            if is_test(&item.attrs) {
                return;
            }
            let body = item.block.to_token_stream().to_string();
            let calls = body.split_whitespace().map(str::to_owned).collect();
            self.out.insert(
                format!("{}::{}", self.file, item.sig.ident),
                Function { body, calls },
            );
        }
    }
    Scan { file, out }.visit_file(&syn::parse_file(source).unwrap());
}

/// OG-RULES Rule 8 for host operations (A-K1): each operation that writes
/// page content, and the edit kind it declares in the binding's per-page
/// kinds (the channel a submit's kinds take).
const OPERATION_KINDS: &[(&str, &str)] = &[
    ("delete", "DeletePage"),
    ("delete_checked", "DeletePage"),
    ("rename", "RenamePage"),
    ("rename_with", "RenamePage"),
    // A feature's operation records the kinds it names; `PageHost::apply`
    // refuses an operation that names none (binding_apply.rs).
    ("apply", OWN_KINDS),
];
/// The operation's own kinds (`HostOp::kinds`) rather than one fixed kind.
const OWN_KINDS: &str = "*";

/// The page host's production module tree: the files declared without
/// `#[cfg(test)]`, from `mod.rs` down.
fn host_runtime_files(dir: &Path) -> Vec<String> {
    let mut files = vec!["mod.rs".to_owned()];
    let mut next = 0;
    while next < files.len() {
        let source = std::fs::read_to_string(dir.join(&files[next])).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        for (n, line) in lines.iter().enumerate() {
            let Some(name) = line.strip_prefix("mod ").and_then(|l| l.strip_suffix(';')) else {
                continue;
            };
            let attrs: Vec<&str> = lines[..n]
                .iter()
                .rev()
                .take_while(|attr| attr.starts_with("#["))
                .copied()
                .collect();
            if attrs.contains(&"#[cfg(test)]") {
                continue;
            }
            let file = attrs
                .iter()
                .find_map(|attr| attr.strip_prefix("#[path = \"")?.strip_suffix("\"]"))
                .map_or(format!("{name}.rs"), str::to_owned);
            assert!(
                dir.join(&file).exists(),
                "page_host module {name}: no {file}"
            );
            files.push(file);
        }
        next += 1;
    }
    files
}

/// The host's operations: `operations.rs`'s non-test visible methods.
fn host_operations(source: &str) -> Vec<String> {
    let parsed = syn::parse_file(source).expect("operations.rs parses");
    let mut found = Vec::new();
    for item in parsed.items {
        let syn::Item::Impl(block) = item else {
            continue;
        };
        for item in block.items {
            if let syn::ImplItem::Fn(function) = item {
                let visible = !matches!(function.vis, syn::Visibility::Inherited);
                if visible && !is_test(&function.attrs) {
                    found.push(function.sig.ident.to_string());
                }
            }
        }
    }
    found
}

/// Production functions that run a host operation without recording its
/// edit kind, and operations with no kind at all.
fn undeclared(functions: &BTreeMap<String, Function>, operations: &[String]) -> Vec<String> {
    let mut found: Vec<String> = operations
        .iter()
        .filter(|op| !OPERATION_KINDS.iter().any(|(name, _)| name == op))
        .map(|op| format!("operation {op} has no edit kind"))
        .collect();
    for (key, function) in functions {
        for (op, kind) in OPERATION_KINDS {
            let runs = function.body.contains(&format!(". {op} ("));
            let declares = function.calls.contains("took")
                && match *kind {
                    OWN_KINDS => function.body.contains("in & kinds"),
                    kind => function.body.contains(&format!("EditKind :: {kind}")),
                };
            // This binding wrapper forwards the same op (including kinds)
            // to PageHost::apply; it never calls inner Host::apply or changes
            // the kinds. The inner admission remains the one kind owner.
            let forwards = *op == "apply"
                && key == "binding_spelling.rs::apply_spellings"
                && function.body.contains("self . apply (op)")
                && !function.body.contains(". kinds");
            if runs && !declares && !forwards {
                found.push(format!("{key} runs {op} without took(…, EditKind::{kind})"));
            }
        }
    }
    found
}

#[test]
fn every_host_operation_records_its_edit_kind() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/page_host");
    let operations = host_operations(&std::fs::read_to_string(dir.join("operations.rs")).unwrap());
    assert!(
        operations.contains(&"delete_checked".to_owned())
            && operations.contains(&"rename_with".to_owned()),
        "the host operation scan found {operations:?}"
    );
    let mut functions = BTreeMap::new();
    let files = host_runtime_files(&dir);
    assert!(
        files.contains(&"binding_retained.rs".to_owned()),
        "{files:?}"
    );
    // The operations' own file composes them; the binding runs them.
    for file in files.iter().filter(|file| *file != "operations.rs") {
        let source = std::fs::read_to_string(dir.join(file)).unwrap();
        scan(file, &source, &mut functions);
    }
    let found = undeclared(&functions, &operations);
    assert!(
        found.is_empty(),
        "OG-RULES Rule 8 (A-K1): a host operation that writes page content declares \
         its edit kind in the binding's per-page kinds: {found:?}. Exemplar \
         page_host/binding.rs PageHost::delete (took(…, EditKind::DeletePage))"
    );
}

#[test]
fn a_planted_host_operation_without_its_kind_fails_the_census() {
    let planted = r#"
        fn bare(host: &mut Host) { host.delete(&key); }
        fn kinded(state: &mut State) {
            state.host.rename_with(&a, &b, &refs, policy);
            state.book.took(written, EditKind::RenamePage);
        }
        fn wrong_kind(state: &mut State) {
            state.host.delete(&key);
            state.book.took([key], EditKind::RenamePage);
        }
        #[cfg(test)] mod tests { fn t(h: &mut Host) { h.delete(&k); } }
    "#;
    let mut functions = BTreeMap::new();
    scan("planted.rs", planted, &mut functions);
    let operations = vec!["delete".to_owned(), "rename_with".to_owned()];
    assert_eq!(
        undeclared(&functions, &operations),
        [
            "planted.rs::bare runs delete without took(…, EditKind::DeletePage)",
            "planted.rs::wrong_kind runs delete without took(…, EditKind::DeletePage)",
        ]
    );
    let new_operation = vec!["truncate".to_owned()];
    assert_eq!(
        undeclared(&BTreeMap::new(), &new_operation),
        ["operation truncate has no edit kind"]
    );
}
