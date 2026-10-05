//! og batch 1 arrival ratchet (`tine-agents/og/batches/01-arrival.md`).
//!
//! Every public item of tine-store is either on the target surface
//! (`SURFACE.txt`, counted against arrival budget A) or listed in
//! `SHALLOW.txt` — the v0.6.5 `Graph` surface still waiting to be rewired,
//! moved to a client, or justified. `SHALLOW.txt` may only shrink:
//! - a public item in neither file fails (new surface must be argued);
//! - a `SHALLOW.txt` entry that no longer exists fails (delete it);
//! - the entry count must equal the `# ceiling:` line, so a shrink is recorded
//!   by lowering the ceiling, and growth is a visible edit of that number.
//! Arrived = `SHALLOW.txt` is empty.
//!
//! `TINE_SHALLOW_PRINT=1 cargo test -p tine-store --test shallow_ratchet`
//! prints the current listing.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use syn::{Attribute, ImplItem, Item, Visibility};

fn is_pub(v: &Visibility) -> bool {
    matches!(v, Visibility::Public(_))
}

/// Evaluates a `cfg` predicate for a production (non-test) build: `test` and
/// `feature = "test-faults"` are false, every other predicate is unknown.
/// `None` = unknown, which counts as compiled in (conservative).
fn cfg_value(meta: &syn::Meta) -> Option<bool> {
    match meta {
        syn::Meta::Path(p) if p.is_ident("test") => Some(false),
        syn::Meta::NameValue(nv) if nv.path.is_ident("feature") => {
            use quote::ToTokens;
            let value = nv.value.to_token_stream().to_string();
            (value.trim_matches('"') == "test-faults").then_some(false)
        }
        syn::Meta::List(l) => {
            let args: Vec<syn::Meta> = l
                .parse_args_with(
                    syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
                )
                .map(|p| p.into_iter().collect())
                .unwrap_or_default();
            let vals: Vec<Option<bool>> = args.iter().map(cfg_value).collect();
            if l.path.is_ident("not") {
                vals.first().copied().flatten().map(|v| !v)
            } else if l.path.is_ident("all") {
                if vals.contains(&Some(false)) {
                    Some(false)
                } else if vals.iter().all(|v| *v == Some(true)) {
                    Some(true)
                } else {
                    None
                }
            } else if l.path.is_ident("any") {
                if vals.contains(&Some(true)) {
                    Some(true)
                } else if vals.iter().all(|v| *v == Some(false)) {
                    Some(false)
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

/// True when the attributes compile the item out of a production build
/// (`#[cfg(test)]`, `#[cfg(feature = "test-faults")]`, and combinations).
/// `#[cfg(not(test))]` is production code and is counted.
fn is_cfg_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && a.parse_args::<syn::Meta>()
                .map(|m| cfg_value(&m) == Some(false))
                .unwrap_or(false)
    })
}

fn type_name(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(p) => p
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default(),
        other => quote_type(other),
    }
}

fn quote_type(ty: &syn::Type) -> String {
    use quote::ToTokens;
    ty.to_token_stream().to_string()
}

fn idents(tokens: impl quote::ToTokens) -> Vec<String> {
    tokens
        .to_token_stream()
        .to_string()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A free function (or a method of another type) whose parameters take a
/// `Store` or `Transaction` is an operation of the store: it is counted with
/// Store + Transaction methods (honest-ratchet rule 1, H2).
fn takes_store(sig: &syn::Signature) -> bool {
    sig.inputs.iter().any(|input| match input {
        syn::FnArg::Typed(t) => idents(&t.ty)
            .iter()
            .any(|t| t == "Store" || t == "Transaction"),
        syn::FnArg::Receiver(_) => false,
    })
}

/// One parsed module of the crate, with where it sits in the module tree.
struct Module {
    /// Logical name used in SURFACE.txt keys: the module path with every
    /// non-`pub` file module folded into its parent (a private module directly
    /// under the crate root keeps its own name).
    key: String,
    /// Every module from the crate root down to this one is `pub`.
    public_chain: bool,
    items: Vec<Item>,
}

/// Walks the whole module tree from `lib.rs`, following every `mod x;`
/// (whatever its visibility, `#[path]` included). Modules compiled out of a
/// production build are recorded in `visited` but not returned.
fn walk(src: &Path) -> (Vec<Module>, BTreeSet<PathBuf>) {
    fn visit(
        file: &Path,
        dir: &Path,
        key: &str,
        public_chain: bool,
        excluded: bool,
        out: &mut Vec<Module>,
        visited: &mut BTreeSet<PathBuf>,
    ) {
        visited.insert(file.to_path_buf());
        let text =
            std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        let parsed = syn::parse_file(&text).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        inline(
            &parsed.items,
            file,
            dir,
            key,
            public_chain,
            excluded,
            out,
            visited,
        );
    }
    #[allow(clippy::too_many_arguments)]
    fn inline(
        items: &[Item],
        file: &Path,
        dir: &Path,
        key: &str,
        public_chain: bool,
        excluded: bool,
        out: &mut Vec<Module>,
        visited: &mut BTreeSet<PathBuf>,
    ) {
        for item in items {
            let Item::Mod(m) = item else { continue };
            let child_excluded = excluded || is_cfg_test(&m.attrs);
            let child_public = public_chain && is_pub(&m.vis);
            let name = m.ident.to_string();
            let child_key = if is_pub(&m.vis) || key == "crate" {
                if key == "crate" {
                    name.clone()
                } else {
                    format!("{key}::{name}")
                }
            } else {
                key.to_owned()
            };
            if let Some((_, inner)) = &m.content {
                inline(
                    inner,
                    file,
                    &dir.join(&name),
                    &child_key,
                    child_public,
                    child_excluded,
                    out,
                    visited,
                );
                continue;
            }
            let path_attr = m.attrs.iter().find_map(|a| {
                if !a.path().is_ident("path") {
                    return None;
                }
                let syn::Meta::NameValue(nv) = &a.meta else {
                    return None;
                };
                let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(s),
                    ..
                }) = &nv.value
                else {
                    return None;
                };
                Some(s.value())
            });
            let (child_file, child_dir) = match path_attr {
                Some(rel) => {
                    let f = file.parent().unwrap().join(rel);
                    let d = f.with_extension("");
                    (f, d)
                }
                None => {
                    let flat = dir.join(format!("{name}.rs"));
                    if flat.exists() {
                        (flat, dir.join(&name))
                    } else {
                        (dir.join(&name).join("mod.rs"), dir.join(&name))
                    }
                }
            };
            visit(
                &child_file,
                &child_dir,
                &child_key,
                child_public,
                child_excluded,
                out,
                visited,
            );
        }
        if !excluded {
            out.push(Module {
                key: key.to_owned(),
                public_chain,
                items: items.to_vec(),
            });
        }
    }
    let mut out = Vec::new();
    let mut visited = BTreeSet::new();
    visit(
        &src.join("lib.rs"),
        src,
        "crate",
        true,
        false,
        &mut out,
        &mut visited,
    );
    (out, visited)
}

/// Names re-exported by a `pub use` anywhere in a public module chain.
fn reexported_names(modules: &[Module]) -> BTreeSet<String> {
    fn names(tree: &syn::UseTree, out: &mut BTreeSet<String>) {
        match tree {
            syn::UseTree::Path(p) => names(&p.tree, out),
            syn::UseTree::Name(n) => {
                out.insert(n.ident.to_string());
            }
            syn::UseTree::Rename(r) => {
                out.insert(r.ident.to_string());
            }
            syn::UseTree::Glob(_) => {}
            syn::UseTree::Group(g) => g.items.iter().for_each(|t| names(t, out)),
        }
    }
    let mut out = BTreeSet::new();
    for m in modules.iter().filter(|m| m.public_chain) {
        for item in &m.items {
            if let Item::Use(u) = item {
                if is_pub(&u.vis) && !is_cfg_test(&u.attrs) {
                    names(&u.tree, &mut out);
                }
            }
        }
    }
    out
}

fn item_ident(item: &Item) -> Option<(String, &Visibility, &[Attribute])> {
    Some(match item {
        Item::Fn(i) => (i.sig.ident.to_string(), &i.vis, &i.attrs),
        Item::Struct(i) => (i.ident.to_string(), &i.vis, &i.attrs),
        Item::Enum(i) => (i.ident.to_string(), &i.vis, &i.attrs),
        Item::Type(i) => (i.ident.to_string(), &i.vis, &i.attrs),
        Item::Const(i) => (i.ident.to_string(), &i.vis, &i.attrs),
        Item::Static(i) => (i.ident.to_string(), &i.vis, &i.attrs),
        Item::Trait(i) => (i.ident.to_string(), &i.vis, &i.attrs),
        Item::Union(i) => (i.ident.to_string(), &i.vis, &i.attrs),
        _ => return None,
    })
}

/// The crate's reachable public surface: items, which of them are store
/// operations, and which carry an OS path in their signature.
struct Surface {
    items: BTreeSet<String>,
    /// Keys (without the `fn ` prefix) of every operation: methods of `Store`
    /// and `Transaction`, plus every reachable function taking either.
    ops: BTreeSet<String>,
    /// Keys of path-carrying signatures (rule 1 allow-list).
    paths: BTreeSet<String>,
    visited: BTreeSet<PathBuf>,
}

/// Identifiers that make a signature carry an OS path: `Path`/`PathBuf` and
/// the `OsStr`/`OsString` spellings of a path (H4).
fn carries_path(tokens: impl quote::ToTokens) -> bool {
    idents(tokens)
        .iter()
        .any(|part| matches!(part.as_str(), "Path" | "PathBuf" | "OsStr" | "OsString"))
}

fn surface(src: &Path) -> Surface {
    let (modules, visited) = walk(src);
    let reexported = reexported_names(&modules);
    // A free item is reachable when its whole module chain is public, or when
    // a public `pub use` names it (it is counted where it is defined, H3).
    let reachable = |m: &Module, name: &str| m.public_chain || reexported.contains(name);
    let mut visible_types = BTreeSet::new();
    for m in &modules {
        for item in &m.items {
            let Some((name, vis, attrs)) = item_ident(item) else {
                continue;
            };
            if matches!(
                item,
                Item::Struct(_) | Item::Enum(_) | Item::Trait(_) | Item::Type(_) | Item::Union(_)
            ) && is_pub(vis)
                && !is_cfg_test(attrs)
                && reachable(m, &name)
            {
                visible_types.insert(name);
            }
        }
    }
    // The crate also re-exports tine-core identities.
    for core in ["FileId", "PageId"] {
        visible_types.insert(core.to_owned());
    }
    let mut s = Surface {
        items: BTreeSet::new(),
        ops: BTreeSet::new(),
        paths: BTreeSet::new(),
        visited,
    };
    for m in &modules {
        let module = m.key.as_str();
        for item in &m.items {
            match item {
                Item::Impl(i) if i.trait_.is_none() && !is_cfg_test(&i.attrs) => {
                    let ty = type_name(&i.self_ty);
                    if !visible_types.contains(&ty) {
                        continue;
                    }
                    for it in &i.items {
                        match it {
                            ImplItem::Fn(f) if is_pub(&f.vis) && !is_cfg_test(&f.attrs) => {
                                let key = format!("{module}::{ty}::{}", f.sig.ident);
                                if ty == "Store" || ty == "Transaction" || takes_store(&f.sig) {
                                    s.ops.insert(key.clone());
                                }
                                if carries_path(&f.sig) {
                                    s.paths.insert(key.clone());
                                }
                                s.items.insert(format!("fn {key}"));
                            }
                            ImplItem::Const(c) if is_pub(&c.vis) && !is_cfg_test(&c.attrs) => {
                                let key = format!("{module}::{ty}::{}", c.ident);
                                if carries_path(&c.ty) {
                                    s.paths.insert(key.clone());
                                }
                                s.items.insert(format!("const {key}"));
                            }
                            _ => {}
                        }
                    }
                }
                Item::Use(i) if m.public_chain && is_pub(&i.vis) && !is_cfg_test(&i.attrs) => {
                    use quote::ToTokens;
                    let tree = i.tree.to_token_stream().to_string().replace(' ', "");
                    s.items.insert(format!("use {module}::{tree}"));
                }
                Item::Macro(i) => {
                    let exported = i.attrs.iter().any(|a| a.path().is_ident("macro_export"));
                    if let (Some(id), true, false) = (&i.ident, exported, is_cfg_test(&i.attrs)) {
                        s.items.insert(format!("macro {id}"));
                    }
                }
                _ => {
                    let Some((name, vis, attrs)) = item_ident(item) else {
                        continue;
                    };
                    if !is_pub(vis) || is_cfg_test(attrs) || !reachable(m, &name) {
                        continue;
                    }
                    let key = format!("{module}::{name}");
                    let kind = match item {
                        Item::Fn(f) => {
                            if takes_store(&f.sig) {
                                s.ops.insert(key.clone());
                            }
                            if carries_path(&f.sig) {
                                s.paths.insert(key.clone());
                            }
                            "fn"
                        }
                        Item::Struct(st) => {
                            for (index, f) in st.fields.iter().enumerate() {
                                if !is_pub(&f.vis) {
                                    continue;
                                }
                                let field = f
                                    .ident
                                    .as_ref()
                                    .map_or_else(|| index.to_string(), ToString::to_string);
                                if f.ident.is_some() {
                                    s.items.insert(format!("field {key}.{field}"));
                                }
                                if carries_path(&f.ty) {
                                    s.paths.insert(format!("{key}.{field}"));
                                }
                            }
                            "struct"
                        }
                        Item::Enum(e) => {
                            for v in &e.variants {
                                if v.fields.iter().any(|f| carries_path(&f.ty)) {
                                    s.paths.insert(format!("{key}::{}", v.ident));
                                }
                            }
                            "enum"
                        }
                        Item::Type(t) => {
                            if carries_path(&t.ty) {
                                s.paths.insert(key.clone());
                            }
                            "type"
                        }
                        Item::Const(c) => {
                            if carries_path(&c.ty) {
                                s.paths.insert(key.clone());
                            }
                            "const"
                        }
                        Item::Static(c) => {
                            if carries_path(&c.ty) {
                                s.paths.insert(key.clone());
                            }
                            "static"
                        }
                        Item::Trait(t) => {
                            for member in &t.items {
                                if let syn::TraitItem::Fn(method) = member {
                                    if carries_path(&method.sig) {
                                        s.paths.insert(format!("{key}::{}", method.sig.ident));
                                    }
                                }
                            }
                            "trait"
                        }
                        Item::Union(u) => {
                            for field in &u.fields.named {
                                if is_pub(&field.vis) && carries_path(&field.ty) {
                                    s.paths
                                        .insert(format!("{key}.{}", field.ident.as_ref().unwrap()));
                                }
                            }
                            "union"
                        }
                        _ => continue,
                    };
                    s.items.insert(format!("{kind} {key}"));
                }
            }
        }
    }
    s
}

fn public_items(src: &Path) -> BTreeSet<String> {
    surface(src).items
}

/// Honest-ratchet rule 1 (H1): the scan reaches every `.rs` file of the
/// library, so no file can hold uncounted surface. Binaries under `src/bin`
/// are separate crate roots and carry no library surface.
#[test]
fn the_surface_scan_reaches_every_source_file() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let visited: BTreeSet<PathBuf> = surface(&src)
        .visited
        .iter()
        .filter_map(|p| p.canonicalize().ok())
        .collect();
    fn all_rs(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n != "bin") {
                    all_rs(&p, out);
                }
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p.canonicalize().unwrap());
            }
        }
    }
    let mut files = Vec::new();
    all_rs(&src, &mut files);
    let missed: Vec<_> = files.iter().filter(|f| !visited.contains(*f)).collect();
    assert!(
        missed.is_empty(),
        "tine-store honest ratchet: these source files are not reached from lib.rs by the surface scan, \
         so their public items would go uncounted. Teach `walk` the module declaration:\n{missed:#?}"
    );
}

fn read_list(path: &Path) -> (Option<usize>, Vec<String>) {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut ceiling = None;
    let mut entries = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(n) = line.strip_prefix("# ceiling:") {
            ceiling = Some(n.trim().parse().expect("ceiling is a number"));
        } else if !line.is_empty() && !line.starts_with('#') {
            entries.push(line.to_string());
        }
    }
    (ceiling, entries)
}

#[test]
fn shallow_surface_only_shrinks() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let public = public_items(&root.join("src"));
    if std::env::var_os("TINE_SHALLOW_PRINT").is_some() {
        println!("# ceiling: {}", public.len());
        for p in &public {
            println!("{p}");
        }
    }
    let (_, surface) = read_list(&root.join("SURFACE.txt"));
    let (ceiling, shallow) = read_list(&root.join("SHALLOW.txt"));
    let surface: BTreeSet<String> = surface.into_iter().collect();
    let shallow_set: BTreeSet<String> = shallow.iter().cloned().collect();
    assert_eq!(
        shallow.len(),
        shallow_set.len(),
        "SHALLOW.txt has duplicate entries"
    );

    let unlisted: Vec<&String> = public
        .iter()
        .filter(|p| !surface.contains(*p) && !shallow_set.contains(*p))
        .collect();
    assert!(
        unlisted.is_empty(),
        "new public tine-store items outside the target surface. Either don't make them pub, \
         or add them to SURFACE.txt (they count against 01-arrival.md budget A). \
         SHALLOW.txt may not grow:\n{unlisted:#?}"
    );
    let stale: Vec<&String> = shallow_set
        .iter()
        .filter(|s| !public.contains(*s))
        .collect();
    assert!(
        stale.is_empty(),
        "SHALLOW.txt entries no longer public — delete them and lower `# ceiling:`:\n{stale:#?}"
    );
    let stale_surface: Vec<&String> = surface.iter().filter(|s| !public.contains(*s)).collect();
    assert!(
        stale_surface.is_empty(),
        "SURFACE.txt entries no longer public:\n{stale_surface:#?}"
    );
    let overlap: Vec<&String> = surface.intersection(&shallow_set).collect();
    assert!(
        overlap.is_empty(),
        "listed in both SURFACE.txt and SHALLOW.txt:\n{overlap:#?}"
    );
    assert_eq!(
        Some(shallow.len()),
        ceiling,
        "SHALLOW.txt has {} entries; set `# ceiling: {}` (it only ever goes down)",
        shallow.len(),
        shallow.len()
    );
}

#[test]
fn arrival_numeric_budgets() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let surface = surface(&root.join("src"));
    let items = &surface.items;
    // Honest count (rule 1): every Store/Transaction method, plus every
    // reachable function that takes a Store or Transaction, wherever it lives.
    let operations = surface.ops.len();
    let questions = items
        .iter()
        .filter(|item| item.starts_with("fn ") && item.contains("::WholeGraph::"))
        .count();
    let types = items
        .iter()
        .filter(|item| {
            ["struct ", "enum ", "trait ", "type ", "union "]
                .iter()
                .any(|prefix| item.starts_with(prefix))
        })
        .count();
    if std::env::var_os("TINE_SHALLOW_PRINT").is_some() {
        println!("operations {operations} questions {questions} types {types}");
        for op in &surface.ops {
            println!("op {op}");
        }
    }
    assert!(operations <= OPS, "tine-store Rule 1: Store + Transaction has {operations} operations (methods plus functions taking a Store/Transaction), budget {OPS}; imitate crates/tine-store/SURFACE.txt");
    assert!(questions <= QUESTIONS, "tine-store Rule 4: WholeGraph has {questions} public methods, budget {QUESTIONS}; imitate crates/tine-store/SURFACE.txt");
    assert!(
        types <= TYPES,
        "tine-store Rule 1: {types} public types, budget {TYPES}; imitate crates/tine-store/SURFACE.txt"
    );
}

// Honest baseline (og-surface step 1, 2026-10-05): the earlier scan missed
// files outside a hand list (H1), functions taking `&Store` (H2) and types
// re-exported from private modules (H3).
const OPS: usize = 45;
const QUESTIONS: usize = 26;
const TYPES: usize = 58;

#[test]
fn public_paths_are_only_inputs_and_handoffs() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let actual = surface(&root).paths;
    // Each path is a user-selected OS path input or a path handed to the OS or
    // user for opening, serving, or recovery. Internal graph identities use
    // FileId/PageId. New signatures require a reason even if SURFACE accepts them.
    let allowed = [
        (
            "directory_durability::sync_directory_entry",
            "app-data directory outside any graph (settings, backup) handed to the OS for sync",
        ),
        (
            "store::Store::create_graph",
            "user-chosen parent input; created root to user",
        ),
        (
            "store::Store::canonical_root",
            "user-chosen root input; canonical root to caller for binding",
        ),
        (
            "store::Store::inspect",
            "user-chosen root input; inspection hand-off",
        ),
        ("store::Store::open", "user-chosen root input"),
        (
            "link_identity::Store::read_link_identity_at",
            "known graph root input, read-only identity lookup before open",
        ),
        (
            "publish::PublishDest::External",
            "user-chosen parent folder input",
        ),
        (
            "store::Store::path_for_os_handoff",
            "validated file path to OS",
        ),
        (
            "store::ConfigState.asset_trash_location",
            "trash location in user-facing error",
        ),
        (
            "store::GraphAccessInspection::approves_external_assets",
            "user-approved device input for comparison",
        ),
        (
            "store::GraphAccessInspection.root",
            "canonical root to user/binding",
        ),
        (
            "store::GraphAccessInspection.external_assets",
            "external target to user for consent",
        ),
        (
            "store::OpenOptions.approved_external_assets",
            "user-approved external target input",
        ),
        (
            "store::OpenOptions.launch_checkpoint",
            "host-chosen app-data file location (OS hand-off)",
        ),
        (
            "store::OpenError::NotAFolder",
            "failed user root path to user",
        ),
        (
            "store::OpenError::Unresolvable",
            "failed user root path to user",
        ),
        (
            "store::OpenError::ExternalAssetsUnapproved",
            "external target to user for consent",
        ),
        ("store::OpenError::CreateFailed", "failed user path to user"),
        ("publish::PublishReceipt.site", "published site to user/OS"),
        (
            "publish::PublishReceipt.previous_kept",
            "recovery site to user",
        ),
        (
            "publish::PublishFailed.previous_kept",
            "recovery site to user",
        ),
        (
            "restore::RestoreReport.recovery",
            "recovery locations to user",
        ),
    ];
    let allowed: BTreeSet<String> = allowed
        .iter()
        .map(|(item, reason)| {
            assert!(!reason.is_empty());
            (*item).to_owned()
        })
        .collect();
    assert_eq!(actual, allowed, "tine-store Rule 1: a public Path/PathBuf signature is only an OS hand-off; graph identities use FileId/PageId. Exemplar: Store::path_for_os_handoff. Update this allow-list only with an OS hand-off reason");
}
