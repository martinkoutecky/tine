//! The page-state writer census (M4, REVIEW-progress-plan R3): `Progress`
//! and `Book` follow the host by its change feed, so every production write
//! of a state they read happens in a function that keeps the feed: held
//! pages (`set_page` and `stop` mark), the draft census and its per-key
//! index (`index_vehicle`, `index_drafts`), the worker overlay (`worker`,
//! `application`, `before`), liveness and incarnation (a rebuild), the feed
//! itself, and the publication consumer's ownership (`owned` with
//! `orphans`). Parsed Rust, by token: a field access is a read, a write (an
//! assignment, a mutating method, `&mut`, an index followed by an
//! assignment) or an unsupported form (a method or a shape the census
//! cannot classify, `ref mut`, a `Host`/`Book` pattern), and the last fails
//! closed. Which functions mark correctly is the reference tracker's
//! (`progress.rs` `Reference`); this census pins who may write at all.

use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use syn::visit::Visit;

const RULE: &str = "M4 (REVIEW-progress-plan R3): a page-host state Progress or Book reads is \
                    written only where the change feed is kept";
const EXEMPLAR: &str = "page_host/mod.rs set_page (marks the key before the write)";

/// Watched fields, the functions (`file::fn`) that may write them, and why.
const WRITERS: &[(&str, &[&str], &str)] = &[
    (
        "pages",
        &["mod.rs::set_page", "mod.rs::stop"],
        "held pages: set_page marks its key; stop clears them as it ends liveness, which \
         rebuilds Progress's sets",
    ),
    (
        "drafts",
        &["mod.rs::index_vehicle", "mod.rs::launch"],
        "the draft census: index_vehicle indexes and marks; launch replaces it, then index_drafts",
    ),
    (
        "durable",
        &[
            "mod.rs::index_vehicle",
            "mod.rs::reindex",
            "mod.rs::index_drafts",
            "production.rs::draft_sync",
            "production.rs::quarantine",
        ],
        "the per-key index; production.rs's are ProductionIo's own durable census, not the host's",
    ),
    (
        "vehicles_of",
        &["mod.rs::index_vehicle", "mod.rs::index_drafts"],
        "the reverse vehicle index, kept with `durable`",
    ),
    (
        "changed",
        &[
            "mod.rs::mark",
            "progress.rs::reconcile",
            "binding_retained.rs::rename",
            "draft_worker.rs::apply_draft_terminal",
        ],
        "the feed: mark adds, reconcile drains, an operation's caller marks its applied pages \
         urgent; binding_retained's is a rename record's own set",
    ),
    (
        "worker",
        &[
            "draft_worker.rs::begin_draft",
            "draft_worker.rs::install_operation",
            "draft_worker.rs::advance_draft",
            "mod.rs::stop",
            "mod.rs::tidy_drafts",
        ],
        "an installed overlay exposes the live drafts it replaced (no change); advance_draft marks \
         when it consumes the application, stop marks every worker page",
    ),
    (
        "application",
        &["draft_worker.rs::advance_draft"],
        "the overlay ends when its application is consumed, which marks the worker's pages",
    ),
    ("before", &[], "fixed at installation"),
    (
        "alive",
        &["mod.rs::stop", "mod.rs::launch"],
        "a liveness change rebuilds Progress's sets",
    ),
    (
        "incarnation",
        &["mod.rs::launch"],
        "a new incarnation rebuilds Progress's policy",
    ),
    (
        "owned",
        &["binding.rs::own", "binding.rs::collect"],
        "ownership: each writer keeps `orphans = owned - held`",
    ),
    (
        "orphans",
        &["binding.rs::own", "binding.rs::collect"],
        "kept with `owned` at every ownership writer",
    ),
];

/// Methods that only read a watched field (or an element of it).
const READS: &[&str] = &[
    "get",
    "contains_key",
    "contains",
    "keys",
    "values",
    "iter",
    "len",
    "is_empty",
    "range",
    "first",
    "last",
    "first_key_value",
    "last_key_value",
    "clone",
    "cloned",
    "as_ref",
    "as_deref",
    "is_some",
    "is_none",
    "is_some_and",
    "is_none_or",
    "is_subset",
    "clean",
    "union",
    "difference",
    "checked_add",
    "join",
    "display",
];

/// Methods that write a watched field.
const WRITES: &[&str] = &[
    "insert",
    "remove",
    "clear",
    "extend",
    "retain",
    "get_mut",
    "entry",
    "push",
    "push_back",
    "pop_front",
    "pop_first",
    "take",
    "replace",
    "append",
    "drain",
    "iter_mut",
    "values_mut",
    "as_mut",
    "split_off",
];

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Use {
    Write,
    Unsupported(String),
}

/// Every write and unsupported form of a watched field in `tokens`, by
/// field.
fn uses(tokens: TokenStream, watched: &BTreeSet<&str>, out: &mut BTreeSet<(String, Use)>) {
    let tokens: Vec<_> = tokens.into_iter().collect();
    let punct = |i: usize| match tokens.get(i) {
        Some(TokenTree::Punct(p)) => Some((p.as_char(), p.spacing())),
        _ => None,
    };
    let ident = |i: usize| match tokens.get(i) {
        Some(TokenTree::Ident(name)) => Some(name.to_string()),
        _ => None,
    };
    let call = |i: usize| matches!(tokens.get(i), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis);
    // An assignment `=` (not `==`, `=>`, `<=`, `>=`, `!=`) at `i`.
    let assigns = |i: usize| {
        punct(i) == Some(('=', Spacing::Alone))
            || (punct(i) == Some(('=', Spacing::Joint))
                && punct(i + 1) != Some(('=', Spacing::Alone))
                && punct(i + 1) != Some(('>', Spacing::Alone)))
    };
    for (i, token) in tokens.iter().enumerate() {
        match token {
            TokenTree::Group(group) => uses(group.stream(), watched, out),
            TokenTree::Ident(word) if word == "ref" && ident(i + 1).as_deref() == Some("mut") => {
                out.insert((
                    "ref mut".into(),
                    Use::Unsupported("a `ref mut` binding".into()),
                ));
            }
            TokenTree::Ident(word)
                if (word == "Host" || word == "Book")
                    && matches!(tokens.get(i + 1), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace) =>
            {
                out.insert((
                    word.to_string(),
                    Use::Unsupported(format!("a `{word} {{ .. }}` shape")),
                ));
            }
            // `&mut` and the place it borrows: a watched field on its path
            // escapes as a mutable alias.
            TokenTree::Punct(p) if p.as_char() == '&' && ident(i + 1).as_deref() == Some("mut") => {
                let mut j = i + 2;
                while let Some(next) = tokens.get(j) {
                    match next {
                        TokenTree::Ident(name) => {
                            if punct(j - 1) == Some(('.', Spacing::Alone))
                                && watched.contains(name.to_string().as_str())
                            {
                                out.insert((name.to_string(), Use::Write));
                            }
                        }
                        TokenTree::Punct(p) if matches!(p.as_char(), '.' | ':') => {}
                        TokenTree::Group(g) if g.delimiter() == Delimiter::Bracket => {}
                        _ => break,
                    }
                    j += 1;
                }
            }
            TokenTree::Ident(name)
                if i > 0
                    && punct(i - 1) == Some(('.', Spacing::Alone))
                    && watched.contains(name.to_string().as_str()) =>
            {
                let field = name.to_string();
                // The element of an index `[..]` is read unless assigned or
                // given to a method the census cannot classify.
                let mut next = i + 1;
                if matches!(tokens.get(next), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Bracket)
                {
                    next += 1;
                }
                let found = if assigns(next) {
                    Some(Use::Write)
                } else if punct(next) == Some(('.', Spacing::Alone)) {
                    match ident(next + 1) {
                        Some(method) if call(next + 2) => {
                            if WRITES.contains(&method.as_str()) {
                                Some(Use::Write)
                            } else if READS.contains(&method.as_str()) {
                                None
                            } else {
                                Some(Use::Unsupported(format!("`.{field}.{method}(..)`")))
                            }
                        }
                        // A field of it: written when assigned.
                        Some(_) if assigns(next + 2) => Some(Use::Write),
                        Some(_) => None,
                        None => Some(Use::Unsupported(format!(
                            "`.{field}.` followed by a non-name"
                        ))),
                    }
                } else if matches!(
                    punct(next),
                    Some(('+' | '-' | '*' | '|' | '&' | '^', Spacing::Joint))
                ) && punct(next + 1) == Some(('=', Spacing::Alone))
                {
                    Some(Use::Write)
                } else {
                    None
                };
                if let Some(found) = found {
                    out.insert((field, found));
                }
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
    watched: &'a BTreeSet<&'a str>,
    /// (`file::fn`, field) -> uses.
    found: &'a mut BTreeMap<(String, String), BTreeSet<Use>>,
    /// Functions by `file::fn`, with the calls in their body.
    calls: &'a mut BTreeMap<String, String>,
}

impl Scan<'_> {
    fn record(&mut self, name: &syn::Ident, attrs: &[syn::Attribute], body: &syn::Block) {
        if self.in_test || is_test(attrs) {
            return;
        }
        let key = format!("{}::{name}", self.file);
        let mut uses_ = BTreeSet::new();
        uses(body.to_token_stream(), self.watched, &mut uses_);
        for (field, found) in uses_ {
            self.found
                .entry((key.clone(), field))
                .or_default()
                .insert(found);
        }
        self.calls
            .entry(key)
            .or_default()
            .push_str(&body.to_token_stream().to_string());
    }
}

impl<'ast> Visit<'ast> for Scan<'_> {
    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        let previous = self.in_test;
        self.in_test |= is_test(&module.attrs);
        syn::visit::visit_item_mod(self, module);
        self.in_test = previous;
    }
    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let previous = self.in_test;
        self.in_test |= is_test(&item.attrs);
        syn::visit::visit_item_impl(self, item);
        self.in_test = previous;
    }
    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        self.record(&function.sig.ident, &function.attrs, &function.block);
    }
    fn visit_impl_item_fn(&mut self, function: &'ast syn::ImplItemFn) {
        self.record(&function.sig.ident, &function.attrs, &function.block);
    }
}

/// The page host's production files: `mod.rs` and every module it (or a
/// production module) declares outside `cfg(test)`, `#[path]` included.
fn production_files(dir: &Path) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    let mut pending = vec!["mod.rs".to_string()];
    while let Some(file) = pending.pop() {
        let source = std::fs::read_to_string(dir.join(&file))
            .unwrap_or_else(|e| panic!("{RULE}: module file {file}: {e}"));
        let parsed = syn::parse_file(&source).expect("Rust source must parse");
        for item in &parsed.items {
            let syn::Item::Mod(module) = item else {
                continue;
            };
            if module.content.is_some() || is_test(&module.attrs) {
                continue;
            }
            let path = module
                .attrs
                .iter()
                .find(|a| a.path().is_ident("path"))
                .map(|a| {
                    let syn::Meta::NameValue(value) = &a.meta else {
                        panic!("{RULE}: unsupported #[path] form");
                    };
                    let syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(path),
                        ..
                    }) = &value.value
                    else {
                        panic!("{RULE}: unsupported #[path] form");
                    };
                    path.value()
                });
            let path = path.unwrap_or_else(|| format!("{}.rs", module.ident));
            if dir.join(&path).exists() && !files.contains_key(&path) {
                pending.push(path);
            }
        }
        files.insert(file, source);
    }
    files
}

type Census = BTreeMap<(String, String), BTreeSet<Use>>;

fn census(files: &BTreeMap<String, String>) -> (Census, BTreeMap<String, String>) {
    let watched: BTreeSet<&str> = WRITERS.iter().map(|(field, ..)| *field).collect();
    let mut found = BTreeMap::new();
    let mut calls = BTreeMap::new();
    for (file, source) in files {
        let parsed = syn::parse_file(source).expect("Rust source must parse");
        Scan {
            file,
            in_test: false,
            watched: &watched,
            found: &mut found,
            calls: &mut calls,
        }
        .visit_file(&parsed);
    }
    (found, calls)
}

/// Violations: writes outside the allowed functions, unsupported forms, a
/// page writer that does not mark, a census writer that does not index.
fn violations(found: &Census, calls: &BTreeMap<String, String>) -> Vec<String> {
    let mut out = Vec::new();
    for ((function, field), uses) in found {
        for found in uses {
            match found {
                Use::Unsupported(form) => {
                    out.push(format!("{function}: {form} on `{field}` is unsupported"))
                }
                Use::Write => {
                    let allowed = WRITERS
                        .iter()
                        .find(|(name, ..)| name == field)
                        .is_some_and(|(_, writers, _)| writers.contains(&function.as_str()));
                    if !allowed {
                        out.push(format!("{function} writes `{field}`"));
                    }
                }
            }
        }
    }
    let body = |function: &str| calls.get(function).map_or("", String::as_str);
    if !body("mod.rs::set_page").contains("mark (") {
        out.push("set_page must mark the key it changes".into());
    }
    if !body("mod.rs::stop").contains("self . alive = false") {
        out.push("stop must end liveness, which rebuilds Progress's sets".into());
    }
    if !body("mod.rs::launch").contains("index_drafts ()") {
        out.push("launch must rebuild the per-key index after replacing the census".into());
    }
    if !body("draft_worker.rs::advance_draft").contains("mark (") {
        out.push("advance_draft must mark the overlay's pages when it ends".into());
    }
    // Every durable index change marks its pages itself, so Progress never
    // depends on an overlay end or an event happening to cover it.
    if body("mod.rs::index_vehicle")
        .matches("self . mark (")
        .count()
        < 2
    {
        out.push("index_vehicle must mark every page either version of the vehicle names".into());
    }
    out
}

fn page_host_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/page_host")
}

#[test]
fn page_state_progress_reads_is_written_only_where_the_feed_is_kept() {
    let files = production_files(&page_host_dir());
    assert!(files.contains_key("binding_launch.rs") && files.contains_key("progress.rs"));
    assert!(!files.keys().any(|f| f.contains("test")), "{files:?}");
    let (found, calls) = census(&files);
    for (field, writers, reason) in WRITERS {
        for writer in *writers {
            assert!(
                found
                    .get(&(writer.to_string(), field.to_string()))
                    .is_some_and(|u| u.contains(&Use::Write)),
                "{RULE}: the allowance {writer} for `{field}` ({reason}) names no writer; drop it"
            );
        }
    }
    let violations = violations(&found, &calls);
    assert!(
        violations.is_empty(),
        "{RULE}; exemplar {EXEMPLAR}. Route the write through a function that keeps the feed, or \
         add it to WRITERS with its reason:\n{}",
        violations.join("\n")
    );
}

/// The census fails on each bypass: a write in another function, a mutable
/// alias, an index assignment, an unclassified method, `ref mut`, a `Host`
/// pattern, and a dropped mark or index rebuild.
#[test]
fn the_page_state_census_catches_its_bypass_mutants() {
    let files = production_files(&page_host_dir());
    let mutants = [
        ("order.rs", "fn take_disk", "fn take_disk_mutant (& mut self) { self . pages . get_mut (\"a\") ; } fn take_disk"),
        ("order.rs", "fn take_disk", "fn alias_mutant (& mut self) { let p = & mut self . pages ; } fn take_disk"),
        ("order.rs", "fn take_disk", "fn index_mutant (& mut self) { self . pages [\"a\"] . conflict = true ; } fn take_disk"),
        ("order.rs", "fn take_disk", "fn unknown_mutant (& mut self) { self . pages . frobnicate () ; } fn take_disk"),
        ("order.rs", "fn take_disk", "fn ref_mutant (& mut self) { if let Some (ref mut x) = self . job { } } fn take_disk"),
        ("order.rs", "fn take_disk", "fn pattern_mutant (h : & mut Host < F >) { let Host { pages , .. } = h ; } fn take_disk"),
        ("binding.rs", "fn record", "fn owned_mutant (& mut self) { self . owned . insert (String :: new ()) ; } fn record"),
        ("draft_worker.rs", "fn begin_draft", "fn overlay_mutant (& mut self) { self . worker . as_mut () ; } fn begin_draft"),
        ("mod.rs", "self.mark(key);\n        let previous", "let previous"),
        ("mod.rs", "self.index_drafts();\n        self.vehicle_debt", "self.vehicle_debt"),
        ("mod.rs", "self.alive = false;\n        self.pages.clear();", "self.pages.clear();"),
        ("mod.rs", "self.mark(&record.page);\n            if let Some(names)", "if let Some(names)"),
        ("mod.rs", "self.mark(&record.page);\n            let names", "let names"),
    ];
    for (file, from, to) in mutants {
        let mut files = files.clone();
        let source = files.get_mut(file).unwrap();
        assert!(source.contains(from), "mutant anchor {from} in {file}");
        *source = source.replacen(from, to, 1);
        let (found, calls) = census(&files);
        assert!(
            !violations(&found, &calls).is_empty(),
            "the census misses the mutant {to} in {file}"
        );
    }
}
