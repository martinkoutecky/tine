//! The page-state writer census (M4, REVIEW-progress-plan R3): `Progress`
//! and `Book` follow the host by its change feed, so every production write
//! of a state they read happens in a function that keeps the feed: held
//! pages (`set_page` and `stop` mark), the draft census and its per-key
//! index (`index_vehicle`, `index_drafts`), the operation vehicles a page's
//! removal waits for (`live_ops`), the worker overlay (`worker`,
//! `application`, `before`), liveness and incarnation (a rebuild), the feed
//! itself, and the publication consumer's ownership (`owned` with
//! `orphans`). Parsed Rust, by token: a field access is a read, a write (an
//! assignment, a mutating method, `&mut`, an index followed by an
//! assignment) or an unsupported form (a method or a shape the census
//! cannot classify, `ref mut`, a `Host`/`Book` pattern, a `&mut` of an
//! expression that is not a place), and the last fails closed. A
//! parenthesized place keeps its context, and only a `cfg` predicate that
//! implies `test` makes an item test-only (REVIEW-3b-P2b-2 N2). Which
//! functions mark correctly is the reference tracker's (`progress.rs`
//! `Reference`); this census pins who may write at all.
//!
//! Scope: the census catches honest mistakes written in the house style, not
//! adversarial spellings. Known limit (REVIEW-3b-P2b-3 N2): a `Self` pattern
//! that binds a watched field (`let Self { pages, .. } = self`) escapes it.
//! The page-table privacy of the next storage step retires the limit.

use crate::test_cfg::is_test;
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
        &["mod.rs::set_page", "mod.rs::stop", "binding_spelling.rs::apply_spellings"],
        "held pages: set_page marks its key; stop clears them as it ends liveness, which \
         rebuilds Progress's sets; apply_spellings mutates only HostOp.pages planning metadata, \
         then delegates to PageHost::apply, never the host's page table",
    ),
    (
        "drafts",
        &["logical.rs::index_vehicle", "mod.rs::launch"],
        "the draft census: index_vehicle indexes and marks; launch replaces it, then index_drafts",
    ),
    (
        "durable",
        &[
            "logical.rs::index_vehicle",
            "logical.rs::reindex",
            "logical.rs::index_drafts",
            "production.rs::draft_sync",
            "production.rs::quarantine",
        ],
        "the per-key index; production.rs's are ProductionIo's own durable census, not the host's",
    ),
    (
        "vehicles_of",
        &["logical.rs::index_vehicle", "logical.rs::index_drafts"],
        "the reverse vehicle index, kept with `durable`",
    ),
    (
        "live_ops",
        &[
            "logical.rs::index_op",
            "logical.rs::reindex_owed",
            "logical.rs::index_drafts",
            "logical.rs::release_owed",
        ],
        "RENAME-COST L1's operation vehicles: index_op follows index_vehicle (which marks every \
         page either version names), reindex_owed marks every page of a vehicle whose owed set \
         empties or fills, index_drafts rebuilds with launch, release_owed (from stop) empties every owed set as \
         liveness ends",
    ),
    (
        "changed",
        &[
            "mod.rs::mark",
            "progress.rs::reconcile",
            "draft_worker.rs::apply_draft_terminal",
            "logical.rs::set_submitted",
        ],
        "the feed: mark adds, reconcile drains, an operation's caller marks its applied pages \
         urgent, a window Submit marks its page urgent (PERF-FIXES D1); R5 removes the \
         special rename binding's separate changed-referrer set",
    ),
    (
        "worker",
        &[
            "draft_worker.rs::begin_draft_retiring",
            "draft_worker.rs::retire_whole",
            "draft_worker.rs::rewrite_pair",
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
        &["draft_worker.rs::advance_draft", "draft_worker.rs::rewrite_step"],
        "the overlay ends when its application is consumed, which marks the worker's pages; a \
         settlement rewrite's step updates only its own bookkeeping (written drafts, failure), \
         and its swap of the operation in Order marks through repair_op",
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
    "is_disjoint",
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
    for (i, token) in tokens.iter().enumerate() {
        match token {
            TokenTree::Group(group) => {
                uses(group.stream(), watched, out);
                // A grouped place, `(self.pages).clear()`: what follows the
                // group uses its field as it would the bare place (N2).
                if is_place_group(&tokens, i) {
                    if let Some(field) = place_tail(group.stream(), watched) {
                        if let Some(found) = after_place(&tokens, &field, i + 1) {
                            out.insert((field, found));
                        }
                    }
                }
            }
            TokenTree::Ident(word)
                if word == "ref" && ident(&tokens, i + 1).as_deref() == Some("mut") =>
            {
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
            // escapes as a mutable alias. An operand that is not a place (a
            // block, `if`, `match`, ...) naming a watched field fails closed.
            TokenTree::Punct(p)
                if p.as_char() == '&' && ident(&tokens, i + 1).as_deref() == Some("mut") =>
            {
                let operand = &tokens[i + 2..];
                let expression = matches!(operand.first(), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace)
                    || matches!(operand.first(), Some(TokenTree::Ident(w)) if ["if", "match", "loop", "unsafe", "while", "async"].contains(&w.to_string().as_str()));
                if expression {
                    let end = operand
                        .iter()
                        .position(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == ';'))
                        .unwrap_or(operand.len());
                    for field in named(&operand[..end], watched) {
                        out.insert((
                            field,
                            Use::Unsupported(
                                "a `&mut` of an expression that is not a place".into(),
                            ),
                        ));
                    }
                } else {
                    for field in borrowed(operand, watched) {
                        out.insert((field, Use::Write));
                    }
                }
            }
            TokenTree::Ident(name)
                if i > 0
                    && punct(&tokens, i - 1) == Some(('.', Spacing::Alone))
                    && watched.contains(name.to_string().as_str()) =>
            {
                let field = name.to_string();
                if let Some(found) = after_place(&tokens, &field, i + 1) {
                    out.insert((field, found));
                }
            }
            _ => {}
        }
    }
}

fn punct(tokens: &[TokenTree], i: usize) -> Option<(char, Spacing)> {
    match tokens.get(i) {
        Some(TokenTree::Punct(p)) => Some((p.as_char(), p.spacing())),
        _ => None,
    }
}

fn ident(tokens: &[TokenTree], i: usize) -> Option<String> {
    match tokens.get(i) {
        Some(TokenTree::Ident(name)) => Some(name.to_string()),
        _ => None,
    }
}

fn parenthesized(token: Option<&TokenTree>) -> bool {
    matches!(token, Some(TokenTree::Group(g)) if matches!(g.delimiter(), Delimiter::Parenthesis | Delimiter::None))
}

/// The group at `i` is a parenthesized expression, not a call's arguments:
/// nothing callable (a name that is not a keyword, a closing `>` of
/// generics, another group) precedes it.
fn is_place_group(tokens: &[TokenTree], i: usize) -> bool {
    const KEYWORDS: &[&str] = &[
        "if", "match", "while", "return", "in", "let", "mut", "move", "else", "break",
    ];
    parenthesized(tokens.get(i))
        && match i.checked_sub(1).and_then(|j| tokens.get(j)) {
            Some(TokenTree::Ident(name)) => KEYWORDS.contains(&name.to_string().as_str()),
            Some(TokenTree::Group(_)) => false,
            Some(TokenTree::Punct(p)) => p.as_char() != '>',
            _ => true,
        }
}

/// The watched field a grouped place ends in: `(… .pages)`, nested
/// groups included.
fn place_tail(stream: TokenStream, watched: &BTreeSet<&str>) -> Option<String> {
    let tokens: Vec<_> = stream.into_iter().collect();
    match tokens.as_slice() {
        [.., TokenTree::Punct(dot), TokenTree::Ident(name)]
            if dot.as_char() == '.' && watched.contains(name.to_string().as_str()) =>
        {
            Some(name.to_string())
        }
        [.., TokenTree::Group(group)] if parenthesized(tokens.last()) => {
            place_tail(group.stream(), watched)
        }
        _ => None,
    }
}

/// How the tokens from `next` on use the watched place `field` ends: an
/// index element is read unless assigned or given to a method the census
/// cannot classify.
fn after_place(tokens: &[TokenTree], field: &str, mut next: usize) -> Option<Use> {
    let call = |i: usize| matches!(tokens.get(i), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis);
    // An assignment `=` (not `==`, `=>`, `<=`, `>=`, `!=`) at `i`.
    let assigns = |i: usize| {
        punct(tokens, i) == Some(('=', Spacing::Alone))
            || (punct(tokens, i) == Some(('=', Spacing::Joint))
                && punct(tokens, i + 1) != Some(('=', Spacing::Alone))
                && punct(tokens, i + 1) != Some(('>', Spacing::Alone)))
    };
    if matches!(tokens.get(next), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Bracket)
    {
        next += 1;
    }
    if assigns(next) {
        Some(Use::Write)
    } else if punct(tokens, next) == Some(('.', Spacing::Alone)) {
        match ident(tokens, next + 1) {
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
        punct(tokens, next),
        Some(('+' | '-' | '*' | '|' | '&' | '^', Spacing::Joint))
    ) && punct(tokens, next + 1) == Some(('=', Spacing::Alone))
    {
        Some(Use::Write)
    } else {
        None
    }
}

/// The watched fields a `&mut` operand borrows: those on its place path,
/// through derefs, indexes and parenthesized places. A call on the path
/// ends the place: the fields before it are its receiver, which the method
/// rule classifies.
fn borrowed(tokens: &[TokenTree], watched: &BTreeSet<&str>) -> Vec<String> {
    let mut fields = Vec::new();
    for (j, token) in tokens.iter().enumerate() {
        match token {
            TokenTree::Ident(name) => {
                if punct(tokens, j.wrapping_sub(1)) == Some(('.', Spacing::Alone))
                    && watched.contains(name.to_string().as_str())
                {
                    fields.push(name.to_string());
                }
            }
            TokenTree::Punct(p) if matches!(p.as_char(), '.' | ':' | '*') => {}
            TokenTree::Literal(_)
                if punct(tokens, j.wrapping_sub(1)) == Some(('.', Spacing::Alone)) => {}
            TokenTree::Group(g) if g.delimiter() == Delimiter::Bracket => {}
            TokenTree::Group(g) if parenthesized(Some(token)) => {
                let callee = matches!(
                    j.checked_sub(1).and_then(|k| tokens.get(k)),
                    Some(TokenTree::Ident(_) | TokenTree::Group(_))
                );
                if callee {
                    fields.clear();
                } else {
                    fields.extend(borrowed(
                        &g.stream().into_iter().collect::<Vec<_>>(),
                        watched,
                    ));
                }
            }
            _ => break,
        }
    }
    fields
}

/// Every watched field named anywhere in `tokens`, groups included.
fn named(tokens: &[TokenTree], watched: &BTreeSet<&str>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (j, token) in tokens.iter().enumerate() {
        match token {
            TokenTree::Ident(name)
                if punct(tokens, j.wrapping_sub(1)) == Some(('.', Spacing::Alone))
                    && watched.contains(name.to_string().as_str()) =>
            {
                out.insert(name.to_string());
            }
            TokenTree::Group(g) => {
                out.extend(named(&g.stream().into_iter().collect::<Vec<_>>(), watched))
            }
            _ => {}
        }
    }
    out
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
/// page writer that does not mark, a page writer that does not index.
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
    if body("logical.rs::index_vehicle")
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
        // REVIEW-3b-P2b-2 N2: a parenthesized place keeps its mutation
        // context, an unclassified mutable escape fails closed, and a
        // production-only cfg is production.
        ("order.rs", "fn take_disk", "fn paren_alias_mutant (& mut self) { let pages = & mut (self . pages) ; pages . clear () ; } fn take_disk"),
        ("order.rs", "fn take_disk", "#[cfg(not(test))] fn not_test_mutant (& mut self) { self . pages . clear () ; } fn take_disk"),
        ("order.rs", "fn take_disk", "#[cfg(any(test, unix))] fn any_test_mutant (& mut self) { self . pages . clear () ; } fn take_disk"),
        ("order.rs", "fn take_disk", "fn grouped_method_mutant (& mut self) { (self . pages) . clear () ; } fn take_disk"),
        ("order.rs", "fn take_disk", "fn grouped_assign_mutant (& mut self) { (self . pages) = Default :: default () ; } fn take_disk"),
        ("order.rs", "fn take_disk", "fn deref_alias_mutant (& mut self) { let p = & mut * (self . pages) ; } fn take_disk"),
        ("order.rs", "fn take_disk", "fn tuple_alias_mutant (& mut self) { let p = & mut (self) . pages ; } fn take_disk"),
        ("order.rs", "fn take_disk", "fn block_escape_mutant (& mut self) { let p = & mut { self . pages } ; } fn take_disk"),
        ("order.rs", "fn take_disk", "fn match_escape_mutant (& mut self) { let p = & mut match 0 { _ => self . pages } ; } fn take_disk"),
        ("order.rs", "fn take_disk", "fn ref_mutant (& mut self) { if let Some (ref mut x) = self . job { } } fn take_disk"),
        ("order.rs", "fn take_disk", "fn pattern_mutant (h : & mut Host < F >) { let Host { pages , .. } = h ; } fn take_disk"),
        ("binding.rs", "fn record", "fn owned_mutant (& mut self) { self . owned . insert (String :: new ()) ; } fn record"),
        ("draft_worker.rs", "fn begin_draft_retiring", "fn overlay_mutant (& mut self) { self . worker . as_mut () ; } fn begin_draft_retiring"),
        ("mod.rs", "self.mark(key);\n        let previous", "let previous"),
        ("mod.rs", "self.index_drafts();\n        self.vehicle_debt", "self.vehicle_debt"),
        ("mod.rs", "self.alive = false;\n        self.pages.clear();", "self.pages.clear();"),
        ("logical.rs", "self.mark(&record.page);\n            if let Some(names)", "if let Some(names)"),
        ("logical.rs", "self.mark(&record.page);\n            let names", "let names"),
    ];
    let mut missed = Vec::new();
    for (file, from, to) in mutants {
        let mut files = files.clone();
        let source = files.get_mut(file).unwrap();
        assert!(source.contains(from), "mutant anchor {from} in {file}");
        *source = source.replacen(from, to, 1);
        let (found, calls) = census(&files);
        if violations(&found, &calls).is_empty() {
            missed.push(format!("{to} in {file}"));
        }
    }
    assert!(
        missed.is_empty(),
        "the census misses the mutants:\n{}",
        missed.join("\n")
    );
}

/// One answerer of "does this paired operation still owe a settlement"
/// (rcd-impl-2d): `Order::holds`, asked live. Production may call it only
/// where the answer is used at once, and no `live_ops` entry may keep a copy
/// (an `Order` phase change would then have to refresh it).
const HOLDS_READERS: &[(&str, &str)] = &[
    (
        "logical.rs::waits",
        "whole retirement, candidacy (`op_waits`, `pair_retains`) and the reference ask it",
    ),
    (
        "logical.rs::reindex_owed",
        "whether an owed-set change alone turns the vehicle's waiting on or off",
    ),
];

/// `LiveOp`'s fields: what only the census and the pages answer.
const LIVE_OP_FIELDS: &[&str] = &["versions", "owed"];

fn holds_violations(files: &BTreeMap<String, String>) -> Vec<String> {
    let mut out = Vec::new();
    let (_, calls) = census(files);
    for (function, body) in &calls {
        if body.contains("order . holds (") && !HOLDS_READERS.iter().any(|(f, _)| f == function) {
            out.push(format!("{function} reads `Order::holds`"));
        }
    }
    let logical = syn::parse_file(&files["logical.rs"]).expect("Rust source must parse");
    let fields: Vec<String> = (logical.items.iter())
        .find_map(|item| match item {
            syn::Item::Struct(s) if s.ident == "LiveOp" => Some(&s.fields),
            _ => None,
        })
        .expect("logical.rs declares LiveOp")
        .iter()
        .map(|f| f.ident.as_ref().map_or(String::new(), ToString::to_string))
        .collect();
    if fields != LIVE_OP_FIELDS {
        out.push(format!(
            "LiveOp has fields {fields:?}, not {LIVE_OP_FIELDS:?}"
        ));
    }
    out
}

#[test]
fn no_operation_vehicle_caches_whether_its_order_holds() {
    let files = production_files(&page_host_dir());
    let (_, calls) = census(&files);
    for (reader, why) in HOLDS_READERS {
        assert!(
            calls
                .get(*reader)
                .is_some_and(|b| b.contains("order . holds (")),
            "the holds reader {reader} ({why}) no longer reads it; drop it"
        );
    }
    let violations = holds_violations(&files);
    assert!(violations.is_empty(), "{}", violations.join("\n"));
    // Its mutants: the cached field back, and a new reader that keeps it.
    let mutants = [
        (
            "pub(super) owed: BTreeSet<PageKey>,\n}",
            "pub(super) owed: BTreeSet<PageKey>,\n    pub(super) held: bool,\n}",
        ),
        (
            "    pub(super) fn repair_op(",
            "    fn held_mutant(&mut self, name: &str) -> bool { self.order.holds(name) }\n    pub(super) fn repair_op(",
        ),
    ];
    for (from, to) in mutants {
        let mut files = files.clone();
        let source = files.get_mut("logical.rs").unwrap();
        assert!(source.contains(from), "mutant anchor {from}");
        *source = source.replacen(from, to, 1);
        assert!(!holds_violations(&files).is_empty(), "missed mutant {to}");
    }
}

/// One answerer of "which pair's removal does newer text on this source
/// supersede now" (rcd-impl-2e B1): `Order::superseded` alone names the
/// phase set (Armed, or Held before the source moved); `supersedes` (is a
/// durable decision still owed) and `disarming`/`disarm` (which pair the
/// published text disarms or retains) ask it.
const SUPERSEDED_PHASES: &str = "PairPhase :: Armed | PairPhase :: Held { removed : false }";
const SUPERSEDED_ANSWERER: &str = "order.rs::superseded";

/// One answerer of "does the source hold its admitted text" (rcd-impl-2e
/// B2, the model's `sourcePresent`): `order::source_present` alone compares
/// a page's bytes with a pair's preimage; `anchored` (with the certified
/// receiver) and `cancel_phase` build on it for launch, Use Disk and the
/// newer text's publication.
const PREIMAGE_ANSWERER: &str = "order.rs::source_present";

/// One answerer of "does this source durably hold its pair's original
/// text" (fix-launch-custody): `Host::source_original` alone asks the byte
/// match (`source_present`) and the file's durability witness
/// (`source_witnessed`); every custody decision asks it: launch's decided
/// phases, Use Disk, and a paired source's publication.
const ORIGINAL_ANSWERER: &str = "save.rs::source_original";
const ORIGINAL_DECIDERS: [&str; 3] = [
    "logical.rs::adopt_pairing",
    "order.rs::discard",
    "save.rs::advance_save_locked",
];

/// A `==`/`!=` whose operand ends in `preimage` (`x . preimage`, `* p .
/// preimage`) in a body's token text.
fn compares_preimage(body: &str) -> bool {
    // Token text glues a group's delimiters to its edge tokens.
    let t: Vec<&str> = (body.split_whitespace())
        .map(|x| x.trim_matches(|c| "()[]{},;".contains(c)))
        .collect();
    let right = |t: &[&str]| {
        let mut i = t.iter().take_while(|x| matches!(**x, "*" | "&")).count();
        let mut last = None;
        while i < t.len() {
            last = Some(t[i]);
            if t.get(i + 1) != Some(&".") {
                break;
            }
            i += 2;
        }
        last == Some("preimage")
    };
    (0..t.len()).any(|i| {
        matches!(t[i], "==" | "!=") && (t[..i].last() == Some(&"preimage") || right(&t[i + 1..]))
    })
}

fn answerer_violations(files: &BTreeMap<String, String>) -> Vec<String> {
    let (_, calls) = census(files);
    let mut out = Vec::new();
    for (function, body) in &calls {
        if body.contains(SUPERSEDED_PHASES) && function != SUPERSEDED_ANSWERER {
            out.push(format!("{function} names the superseded phase set"));
        }
        if compares_preimage(body) && function != PREIMAGE_ANSWERER {
            out.push(format!("{function} compares bytes with a preimage"));
        }
        let flat = squashed(body);
        if (flat.contains("source_present(") || flat.contains(".source_witnessed("))
            && function != ORIGINAL_ANSWERER
        {
            out.push(format!("{function} judges a source's original itself"));
        }
    }
    for decider in ORIGINAL_DECIDERS {
        let asks = calls.get(decider).map(|b| squashed(b));
        if !asks.is_some_and(|b| b.contains("Self::source_original(")) {
            out.push(format!("{decider} no longer asks `source_original`"));
        }
    }
    out
}

#[test]
fn one_function_answers_each_superseding_question() {
    let files = production_files(&page_host_dir());
    let violations = answerer_violations(&files);
    assert!(violations.is_empty(), "{}", violations.join("\n"));
    let (_, calls) = census(&files);
    assert!(calls[SUPERSEDED_ANSWERER].contains(SUPERSEDED_PHASES));
    assert!(compares_preimage(&calls[PREIMAGE_ANSWERER]));
    let original = squashed(&calls[ORIGINAL_ANSWERER]);
    assert!(original.contains("source_present(") && original.contains(".source_witnessed("));
    // Its mutants: an inline preimage match in the answerer, the base's
    // byte-only anchoring at publication and in a restoration, a second
    // copy of the phase set.
    let mutants = [
        (
            "save.rs",
            "!order::source_present(preimage, bytes)",
            "!(bytes.is_some() && bytes == preimage)",
        ),
        (
            "save.rs",
            "Self::source_original(&mut self.fs, &key, &preimage, &job.bytes)",
            "if order::source_present(&preimage, &job.bytes) { order::Original::Durable } else { order::Original::No }",
        ),
        (
            "save.rs",
            "Self::source_original(fs, &key, preimage, &job.bytes)",
            "if order::source_present(preimage, &job.bytes) { order::Original::Durable } else { order::Original::No }",
        ),
        (
            "order.rs",
            "    /// The superseded pair (`superseded`)",
            "    fn armed_mutant(p: &Pair) -> bool {\n        matches!(p.phase, PairPhase::Armed | PairPhase::Held { removed: false })\n    }\n    /// The superseded pair (`superseded`)",
        ),
    ];
    for (file, from, to) in mutants {
        let mut files = files.clone();
        let source = files.get_mut(file).unwrap();
        assert!(source.contains(from), "mutant anchor {from}");
        *source = source.replacen(from, to, 1);
        assert!(
            !answerer_violations(&files).is_empty(),
            "missed mutant {to}"
        );
    }
}

/// One answerer each for two questions the slice reviews found copied
/// (rcd-complete, F7 and F5): "does this paired operation still owe a pair"
/// is `PairedOp::owes` (`touches` asks it), and "mark the sources whose
/// settlement-rewrite candidacy may have changed" is
/// `mark_resolved_sources` (`repair_op` and `reindex_owed` call it).
const OWES_ANSWERER: &str = "order.rs::owes";
const OWES_TEST: &str = ".any(|p|p.phase.unresolved())";
const RESOLVED_MARKER: &str = "logical.rs::mark_resolved_sources";
const RESOLVED_SELECTION: &str = ".filter(|p|!p.phase.unresolved()).map(|p|p.source.clone())";

/// A body's token text without whitespace, so a pattern matches however
/// the token printer spaces it.
fn squashed(body: &str) -> String {
    body.split_whitespace().collect()
}

fn owner_violations(files: &BTreeMap<String, String>) -> Vec<String> {
    let (_, calls) = census(files);
    let mut out = Vec::new();
    for (function, body) in &calls {
        let body = squashed(body);
        if body.contains(OWES_TEST) && function != OWES_ANSWERER {
            out.push(format!("{function} inlines `PairedOp::owes`"));
        }
        if body.contains(RESOLVED_SELECTION) && function != RESOLVED_MARKER {
            out.push(format!("{function} selects resolved sources itself"));
        }
    }
    let calls_of = |function: &str| calls.get(function).map(|b| squashed(b));
    if !calls_of("order.rs::touches").is_some_and(|b| b.contains(".owes()")) {
        out.push("order.rs::touches no longer asks `owes`".into());
    }
    for caller in ["logical.rs::repair_op", "logical.rs::reindex_owed"] {
        if !calls_of(caller).is_some_and(|b| b.contains("mark_resolved_sources(")) {
            out.push(format!("{caller} no longer calls `mark_resolved_sources`"));
        }
    }
    out
}

#[test]
fn one_function_owns_owing_and_the_resolved_marking() {
    let files = production_files(&page_host_dir());
    let violations = owner_violations(&files);
    assert!(violations.is_empty(), "{}", violations.join("\n"));
    let (_, calls) = census(&files);
    assert!(squashed(&calls[OWES_ANSWERER]).contains(OWES_TEST));
    assert!(squashed(&calls[RESOLVED_MARKER]).contains(RESOLVED_SELECTION));
    // Its mutants: the base's inline test in `touches`, and the base's
    // second copy of the marking in `reindex_owed`.
    let mutants = [
        (
            "order.rs",
            "op.pages.contains(key) && op.owes()",
            "op.pages.contains(key) && op.pairs.iter().any(|p| p.phase.unresolved())",
        ),
        (
            "logical.rs",
            "                self.mark_resolved_sources(&name);",
            "                let resolved: Vec<PageKey> = (self.order.paired(&name).into_iter())\n                    .flat_map(|op| op.pairs.iter())\n                    .filter(|p| !p.phase.unresolved())\n                    .map(|p| p.source.clone())\n                    .collect();\n                for source in resolved {\n                    self.mark(&source);\n                }",
        ),
    ];
    for (file, from, to) in mutants {
        let mut files = files.clone();
        let source = files.get_mut(file).unwrap();
        assert!(source.contains(from), "mutant anchor {from}");
        *source = source.replacen(from, to, 1);
        assert!(!owner_violations(&files).is_empty(), "missed mutant {to}");
    }
}
