//! The writer call-site guard (STEP3 §7, R6, A-F1): every store write in a
//! production function is inside the write closure of a host reservation
//! (`retained::reserved`) or of the host's restore stop (`restore_hosted`), or
//! is in a function that runs only under one: every call site of it is in
//! such a closure or in a function that itself runs only under one (a least
//! fixed point anchored at real closures, so a reservation elsewhere in the
//! body, mutual recursion or no caller at all certifies nothing). A function
//! that writes no page file is exempt with its reason. Calls resolve by their
//! last path segment; a function the rule depends on that is named as a value
//! (a callback, an alias, a turbofish call) is rejected as unsupported. A new
//! writer call site fails here, naming the rule and an exemplar.

use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use syn::visit::Visit;

const RULE: &str = "STEP3 §7 (R6): a production page writer runs under a page-host reservation";
const EXEMPLAR: &str = "crates/tine-graph-features/src/conflicts.rs fold_pair";
/// Store calls that write graph files.
const WRITES: &[&str] = &["transaction", "restore"];
/// Calls that route a writer through the host.
const ROUTES: &[&str] = &["reserved", "restore_hosted"];

/// Writers of no page file, by `file::fn`, each with its reason.
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
        "crates/tine-graph-features/src/pages.rs::commit_home",
        "writes logseq/config.edn (the home page setting) after a host rename; no page",
    ),
];

#[derive(Default)]
struct Function {
    /// A store write anywhere in the body.
    writes: bool,
    /// A store write outside every route call's write closure (A-F1).
    unrouted_writes: bool,
    /// Every call, by its last path segment.
    calls: BTreeSet<String>,
    /// The calls outside every route call's write closure.
    unrouted_calls: BTreeSet<String>,
    /// Identifiers used as values: callbacks, aliases, variables, and
    /// turbofish calls the call scan does not follow.
    values: BTreeSet<String>,
    /// The body's tokens, spaced as `quote` prints them.
    body: String,
}

/// Records `tokens`' calls, writes and value names; `routed` when they are
/// inside a route call's last argument (its write closure).
fn calls(tokens: proc_macro2::TokenStream, routed: bool, out: &mut Function) {
    use proc_macro2::{Delimiter, Spacing, TokenTree};
    let tokens: Vec<_> = tokens.into_iter().collect();
    let punct = |i: usize| match tokens.get(i) {
        Some(TokenTree::Punct(p)) => Some((p.as_char(), p.spacing())),
        _ => None,
    };
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            TokenTree::Group(group) => calls(group.stream(), routed, out),
            TokenTree::Ident(name) => {
                let name = name.to_string();
                let method = i > 0 && punct(i - 1) == Some(('.', Spacing::Alone));
                let declared =
                    i > 0 && matches!(&tokens[i - 1], TokenTree::Ident(word) if word == "fn");
                match tokens.get(i + 1) {
                    Some(TokenTree::Group(args))
                        if args.delimiter() == Delimiter::Parenthesis && !declared =>
                    {
                        let write = method && WRITES.contains(&name.as_str());
                        out.writes |= write;
                        out.unrouted_writes |= write && !routed;
                        if !routed {
                            out.unrouted_calls.insert(name.clone());
                        }
                        if ROUTES.contains(&name.as_str()) {
                            // The last argument is the write closure.
                            let mut arguments = vec![proc_macro2::TokenStream::new()];
                            for token in args.stream() {
                                match &token {
                                    TokenTree::Punct(comma) if comma.as_char() == ',' => {
                                        arguments.push(proc_macro2::TokenStream::new())
                                    }
                                    _ => arguments.last_mut().unwrap().extend([token]),
                                }
                            }
                            arguments.retain(|argument| !argument.is_empty());
                            let last = arguments.pop().unwrap_or_default();
                            for argument in arguments {
                                calls(argument, routed, out);
                            }
                            calls(last, true, out);
                        } else {
                            calls(args.stream(), routed, out);
                        }
                        out.calls.insert(name);
                        i += 2;
                        continue;
                    }
                    // A path prefix (`module::`), a macro, a field or a
                    // binding's type; a turbofish call is a value: unfollowed.
                    _ if punct(i + 1) == Some((':', Spacing::Joint)) => {
                        if punct(i + 3) == Some(('<', Spacing::Alone)) {
                            out.values.insert(name);
                        }
                    }
                    _ if matches!(punct(i + 1), Some(('!' | ':', _))) => {}
                    _ if method || declared => {}
                    _ => {
                        out.values.insert(name);
                    }
                }
            }
            _ => {}
        }
        i += 1;
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
        calls(body.to_token_stream(), false, &mut found);
        entry.writes |= found.writes;
        entry.unrouted_writes |= found.unrouted_writes;
        entry.calls.extend(found.calls);
        entry.unrouted_calls.extend(found.unrouted_calls);
        entry.values.extend(found.values);
        entry.body.push_str(&body.to_token_stream().to_string());
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

/// Writers that are neither routed nor exempt, and value uses of a function
/// the rule depends on (A-F1).
fn unrouted(functions: &BTreeMap<String, Function>, exempt: &[(&str, &str)]) -> Vec<String> {
    fn name(key: &str) -> &str {
        key.rsplit("::").next().unwrap()
    }
    // Functions that run only under a reservation: the least fixed point of
    // "called somewhere, and every call outside a write closure is in such a
    // function". A function's own recursive call is not a call site.
    let mut under = BTreeSet::new();
    loop {
        let grown: Vec<&String> = functions
            .keys()
            .filter(|key| !under.contains(*key))
            .filter(|key| {
                let callers: Vec<(&String, &Function)> = functions
                    .iter()
                    .filter(|(caller, f)| caller != key && f.calls.contains(name(key)))
                    .collect();
                !callers.is_empty()
                    && callers.iter().all(|(caller, f)| {
                        !f.unrouted_calls.contains(name(key)) || under.contains(*caller)
                    })
            })
            .collect();
        if grown.is_empty() {
            break;
        }
        under.extend(grown);
    }
    let writers: Vec<&String> = functions
        .iter()
        .filter(|(key, f)| f.unrouted_writes && !exempt.iter().any(|(e, _)| e == key))
        .map(|(key, _)| key)
        .collect();
    // The functions whose call sites the rule reads: the writers, and every
    // function calling one of these outside a write closure.
    let mut depended: BTreeSet<&str> = writers.iter().map(|key| name(key)).collect();
    loop {
        let grown: Vec<&str> = functions
            .iter()
            .filter(|(key, f)| {
                !depended.contains(name(key))
                    && f.unrouted_calls
                        .iter()
                        .any(|call| depended.contains(call.as_str()))
            })
            .map(|(key, _)| name(key))
            .collect();
        if grown.is_empty() {
            break;
        }
        depended.extend(grown);
    }
    let mut found: Vec<String> = writers
        .into_iter()
        .filter(|key| !under.contains(*key))
        .cloned()
        .collect();
    for (key, f) in functions {
        for value in f.values.iter().filter(|v| depended.contains(v.as_str())) {
            found.push(format!("{key} names {value} as a value (unsupported)"));
        }
    }
    found
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
        "crates/tine-graph-features/src/conflicts.rs::fold_pair",
        "crates/tine-graph-features/src/conflicts.rs::trash_sync_conflict",
        "crates/tine-graph-features/src/conflicts.rs::resolve_vcs_marker_conflict",
        "crates/tine-graph-features/src/journals.rs::migrate_journal_filenames",
        "crates/tine-graph-features/src/journals.rs::trash_journal_file",
        "crates/tine-graph-features/src/pdf.rs::write_highlights",
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

/// A-F1 (REVIEW-3a3): the rule is scope-sensitive. A reservation elsewhere in
/// the body, one unreserved caller, mutual recursion, a callback, an alias or
/// a turbofish call certifies nothing; a qualified call inside a write closure
/// and a recursive helper under one do.
#[test]
fn a_planted_writer_beside_an_empty_reservation_fails_the_guard() {
    let planted = r#"
        pub fn empty(store: &Store) {
            reserved(None, Input::Flush, d, r, |_| Ok(None));
            store.transaction(None).commit();
        }
        fn shared(store: &Store) { store.transaction(None).commit(); }
        pub fn kept(store: &Store) { reserved(None, Input::Flush, d, r, |_| shared(store)); }
        pub fn leaked(store: &Store) { shared(store); }
        fn ping(store: &Store) { pong(store); store.transaction(None).commit(); }
        fn pong(store: &Store) { ping(store); }
        pub fn anchors_ping(store: &Store) {
            reserved(None, Input::Flush, d, r, |_| ping(store));
        }
        fn by_value(store: &Store) { store.transaction(None).commit(); }
        pub fn callback(store: &Store) { reserved(None, Input::Flush, d, r, by_value); }
        fn aliased(store: &Store) { store.transaction(None).commit(); }
        pub fn alias(store: &Store) {
            let write = aliased;
            reserved(None, Input::Flush, d, r, |_| aliased(store));
        }
        fn generic<T>(store: &Store) { store.transaction(None).commit(); }
        pub fn turbofish(store: &Store) {
            reserved(None, Input::Flush, d, r, |_| generic::<u8>(store));
        }
        fn nested(store: &Store, n: u8) {
            if n > 0 { nested(store, n - 1) }
            store.transaction(None).commit();
        }
        pub fn qualified(store: &Store) {
            crate::retained::reserved(
                None,
                Input::Flush,
                d,
                r,
                |_| crate::planted::nested(store, 2),
            );
        }
    "#;
    let mut functions = BTreeMap::new();
    scan("planted.rs", planted, &mut functions);
    assert_eq!(
        unrouted(&functions, &[]),
        [
            "planted.rs::by_value",
            "planted.rs::empty",
            "planted.rs::generic",
            "planted.rs::ping",
            "planted.rs::shared",
            "planted.rs::alias names aliased as a value (unsupported)",
            "planted.rs::callback names by_value as a value (unsupported)",
            "planted.rs::turbofish names generic as a value (unsupported)",
        ]
    );
}

/// OG-RULES Rule 8 for host operations (A-K1): each operation that writes
/// page content, and the edit kind it declares in the binding's per-page
/// kinds (the channel a submit's kinds take).
const OPERATION_KINDS: &[(&str, &str)] = &[
    ("delete", "DeletePage"),
    ("delete_checked", "DeletePage"),
    ("rename", "RenamePage"),
    ("rename_with", "RenamePage"),
];

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
                && function.body.contains(&format!("EditKind :: {kind}"));
            if runs && !declares {
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
        operations.contains(&"delete".to_owned()) && operations.contains(&"rename_with".to_owned()),
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
