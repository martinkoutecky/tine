//! The checkpoint's config key (ADR 0070) is exactly the config the build
//! and index path reads, and a config edited while closed is judged by it.
use super::*;
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};
use syn::visit::{self, Visit};

const RULE: &str = "ADR 0070 / I-21: `checkpoint::config_key` keys exactly the config fields \
     tine-store and every tine-core function given a `Config` read (exemplar: the \
     destructuring in `config_key`); bind a field `_` only if no build or index code reads it";

fn crate_dir(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            // Unit-test modules (`#[cfg(test)] #[path = "…_tests.rs"]`).
            && !path.to_string_lossy().ends_with("_tests.rs")
        {
            out.push(path);
        }
    }
}

fn parse(path: &Path) -> syn::File {
    syn::parse_file(&fs::read_to_string(path).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Code that is not in a shipped build: `#[test]`, `#[cfg(test)]`, and
/// `#[cfg(any(test, feature = "test-faults"))]`. A `not(…)` cfg is scanned.
fn test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if attr.path().is_ident("test") {
            return true;
        }
        if !attr.path().is_ident("cfg") {
            return false;
        }
        let tokens = attr.meta.to_token_stream().to_string();
        let words: Vec<&str> = tokens
            .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-'))
            .collect();
        (words.contains(&"test") || words.contains(&"test-faults")) && !words.contains(&"not")
    })
}

fn item_attrs(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Fn(i) => &i.attrs,
        syn::Item::Mod(i) => &i.attrs,
        syn::Item::Impl(i) => &i.attrs,
        syn::Item::Const(i) => &i.attrs,
        syn::Item::Static(i) => &i.attrs,
        syn::Item::Trait(i) => &i.attrs,
        syn::Item::Macro(i) => &i.attrs,
        syn::Item::Struct(i) => &i.attrs,
        syn::Item::Enum(i) => &i.attrs,
        syn::Item::Use(i) => &i.attrs,
        _ => &[],
    }
}

fn impl_item_attrs(item: &syn::ImplItem) -> &[syn::Attribute] {
    match item {
        syn::ImplItem::Fn(i) => &i.attrs,
        syn::ImplItem::Const(i) => &i.attrs,
        _ => &[],
    }
}

fn mentions(tokens: proc_macro2::TokenStream, ident: &str) -> bool {
    tokens.into_iter().any(|tree| match tree {
        proc_macro2::TokenTree::Ident(found) => found == ident,
        proc_macro2::TokenTree::Group(group) => mentions(group.stream(), ident),
        _ => false,
    })
}

/// Every `.name` a body reads (field or method), `Config { name, .. }`
/// patterns, and `.name` inside macro arguments (`format!`, `assert!`),
/// which `syn` leaves as tokens.
#[derive(Default)]
struct Reads {
    names: BTreeSet<String>,
}

impl Reads {
    fn tokens(&mut self, stream: proc_macro2::TokenStream) {
        let mut dot = false;
        for tree in stream {
            match tree {
                proc_macro2::TokenTree::Punct(p) => dot = p.as_char() == '.',
                proc_macro2::TokenTree::Ident(ident) => {
                    if dot {
                        self.names.insert(ident.to_string());
                    }
                    dot = false;
                }
                proc_macro2::TokenTree::Group(group) => {
                    self.tokens(group.stream());
                    dot = false;
                }
                proc_macro2::TokenTree::Literal(_) => dot = false,
            }
        }
    }
}

impl<'ast> Visit<'ast> for Reads {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        // `config_key` itself binds the key; it is not evidence of a read.
        let the_key = matches!(item, syn::Item::Fn(f) if f.sig.ident == "config_key");
        if !test_only(item_attrs(item)) && !the_key {
            visit::visit_item(self, item);
        }
    }
    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if !test_only(impl_item_attrs(item)) {
            visit::visit_impl_item(self, item);
        }
    }
    fn visit_expr_field(&mut self, expr: &'ast syn::ExprField) {
        if let syn::Member::Named(name) = &expr.member {
            self.names.insert(name.to_string());
        }
        visit::visit_expr_field(self, expr);
    }
    fn visit_expr_method_call(&mut self, expr: &'ast syn::ExprMethodCall) {
        self.names.insert(expr.method.to_string());
        visit::visit_expr_method_call(self, expr);
    }
    fn visit_pat_struct(&mut self, pat: &'ast syn::PatStruct) {
        if pat
            .path
            .segments
            .last()
            .is_some_and(|s| s.ident == "Config")
        {
            for field in &pat.fields {
                if let syn::Member::Named(name) = &field.member {
                    if !matches!(*field.pat, syn::Pat::Wild(_)) {
                        self.names.insert(name.to_string());
                    }
                }
            }
        }
        visit::visit_pat_struct(self, pat);
    }
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        self.tokens(mac.tokens.clone());
    }
}

/// tine-core: what every non-test function with a `Config` parameter reads
/// (`ParseConfig` and other derived values are built inside one), and what
/// each `impl Config` method reads of `self`. `GraphMeta::from_config` is
/// returned to the app as display metadata and never enters the generation
/// (pinned by `graph_meta_stays_out_of_the_generation` below).
#[derive(Default)]
struct CoreReads {
    in_impl: Option<String>,
    reads: Reads,
    config_methods: BTreeMap<String, BTreeSet<String>>,
}

impl CoreReads {
    fn function(&mut self, name: &str, sig: &syn::Signature, block: &syn::Block) {
        let impl_of = self.in_impl.clone().unwrap_or_default();
        if impl_of == "Config" && sig.receiver().is_some() {
            let mut reads = Reads::default();
            reads.visit_block(block);
            self.config_methods.insert(name.to_owned(), reads.names);
            return;
        }
        let takes_config = sig.inputs.iter().any(|input| match input {
            syn::FnArg::Typed(typed) => mentions(typed.ty.to_token_stream(), "Config"),
            syn::FnArg::Receiver(_) => false,
        });
        if takes_config && !(impl_of == "GraphMeta" && name == "from_config") {
            self.reads.visit_block(block);
        }
    }
}

impl<'ast> Visit<'ast> for CoreReads {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if !test_only(item_attrs(item)) {
            visit::visit_item(self, item);
        }
    }
    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let outer = self.in_impl.take();
        self.in_impl = match (&*item.self_ty, &item.trait_) {
            (syn::Type::Path(path), None) => path.path.segments.last().map(|s| s.ident.to_string()),
            _ => None,
        };
        visit::visit_item_impl(self, item);
        self.in_impl = outer;
    }
    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if !test_only(&item.attrs) {
            self.function(&item.sig.ident.to_string(), &item.sig, &item.block);
        }
    }
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if !test_only(&item.attrs) {
            self.function(&item.sig.ident.to_string(), &item.sig, &item.block);
        }
    }
}

/// The fields of `tine_core::config::Config`, from its definition.
fn config_fields() -> BTreeSet<String> {
    let file = parse(&crate_dir("../tine-core/src/config.rs"));
    for item in file.items {
        if let syn::Item::Struct(item) = item {
            if item.ident == "Config" {
                return item
                    .fields
                    .iter()
                    .map(|field| field.ident.as_ref().unwrap().to_string())
                    .collect();
            }
        }
    }
    panic!("no struct Config in tine-core/src/config.rs");
}

/// `config_key`'s pattern: (keyed, bound `_`).
fn key_classification() -> (BTreeSet<String>, BTreeSet<String>) {
    struct Find(Option<(BTreeSet<String>, BTreeSet<String>)>);
    impl<'ast> Visit<'ast> for Find {
        fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
            if item.sig.ident != "config_key" {
                return;
            }
            for stmt in &item.block.stmts {
                if let syn::Stmt::Local(local) = stmt {
                    if let syn::Pat::Struct(pat) = &local.pat {
                        let (mut keyed, mut unkeyed) = (BTreeSet::new(), BTreeSet::new());
                        for field in &pat.fields {
                            let syn::Member::Named(name) = &field.member else {
                                continue;
                            };
                            if matches!(*field.pat, syn::Pat::Wild(_)) {
                                unkeyed.insert(name.to_string());
                            } else {
                                keyed.insert(name.to_string());
                            }
                        }
                        assert!(pat.rest.is_none(), "{RULE}: no `..` in config_key");
                        self.0 = Some((keyed, unkeyed));
                    }
                }
            }
        }
    }
    let mut find = Find(None);
    find.visit_file(&parse(&crate_dir("src/store/checkpoint.rs")));
    find.0.expect("config_key destructures Config")
}

fn store_reads() -> BTreeSet<String> {
    let mut files = Vec::new();
    rust_files(&crate_dir("src"), &mut files);
    let mut reads = Reads::default();
    for path in files {
        reads.visit_file(&parse(&path));
    }
    reads.names
}

fn core_reads() -> (BTreeSet<String>, BTreeMap<String, BTreeSet<String>>) {
    let mut files = Vec::new();
    rust_files(&crate_dir("../tine-core/src"), &mut files);
    let mut core = CoreReads::default();
    for path in files {
        core.visit_file(&parse(&path));
    }
    (core.reads.names, core.config_methods)
}

#[test]
fn the_config_key_is_exactly_what_the_build_reads() {
    let fields = config_fields();
    let (keyed, unkeyed) = key_classification();
    let classified: BTreeSet<String> = keyed.union(&unkeyed).cloned().collect();
    assert_eq!(
        classified, fields,
        "{RULE}: every Config field is classified"
    );

    let (core, methods) = core_reads();
    let mut names: BTreeSet<String> = store_reads().union(&core).cloned().collect();
    // A `Config` method read anywhere reads what its body reads (to a fixpoint).
    loop {
        let implied: BTreeSet<String> = methods
            .iter()
            .filter(|(method, _)| names.contains(*method))
            .flat_map(|(_, reads)| reads.iter().cloned())
            .collect();
        let before = names.len();
        names.extend(implied);
        if names.len() == before {
            break;
        }
    }
    let read: BTreeSet<String> = names.intersection(&fields).cloned().collect();
    let unkeyed_but_read: Vec<_> = read.difference(&keyed).collect();
    let keyed_but_unread: Vec<_> = keyed.difference(&read).collect();
    assert!(
        unkeyed_but_read.is_empty(),
        "{RULE}: read by the build but not keyed: {unkeyed_but_read:?}"
    );
    assert!(
        keyed_but_unread.is_empty(),
        "{RULE}: keyed but read by no build code: {keyed_but_unread:?}"
    );
}

/// The `GraphMeta::from_config` exemption: in shipped code tine-store builds
/// `GraphMeta` only to return it from `Store::open` (`Graph::meta` is
/// test-only), and reads no `GraphMeta` back.
#[test]
fn graph_meta_stays_out_of_the_generation() {
    struct Users {
        current: Vec<String>,
        users: BTreeSet<String>,
        meta_calls: usize,
    }
    impl<'ast> Visit<'ast> for Users {
        fn visit_item(&mut self, item: &'ast syn::Item) {
            if !test_only(item_attrs(item)) && !matches!(item, syn::Item::Use(_)) {
                visit::visit_item(self, item);
            }
        }
        fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
            if !test_only(impl_item_attrs(item)) {
                visit::visit_impl_item(self, item);
            }
        }
        fn visit_signature(&mut self, sig: &'ast syn::Signature) {
            self.current.push(sig.ident.to_string());
            if mentions(sig.to_token_stream(), "GraphMeta") {
                self.users.insert(sig.ident.to_string());
            }
        }
        fn visit_block(&mut self, block: &'ast syn::Block) {
            if mentions(block.to_token_stream(), "GraphMeta") {
                if let Some(name) = self.current.last() {
                    self.users.insert(name.clone());
                }
            }
            visit::visit_block(self, block);
        }
        fn visit_expr_method_call(&mut self, expr: &'ast syn::ExprMethodCall) {
            if expr.method == "meta" && expr.args.is_empty() {
                self.meta_calls += 1;
            }
            visit::visit_expr_method_call(self, expr);
        }
    }
    let mut files = Vec::new();
    rust_files(&crate_dir("src"), &mut files);
    let mut users = Users {
        current: Vec::new(),
        users: BTreeSet::new(),
        meta_calls: 0,
    };
    for path in files {
        users.visit_file(&parse(&path));
    }
    assert_eq!(
        users.users,
        BTreeSet::from(["open".to_owned()]),
        "{RULE}: a new GraphMeta use in tine-store may carry display settings into the generation"
    );
    assert_eq!(users.meta_calls, 0, "{RULE}: tine-store reads no GraphMeta");
}
