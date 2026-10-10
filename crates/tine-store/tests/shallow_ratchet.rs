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
//! The target surface is budgeted per concept (og-surface review §6): rule 2
//! sections and ceilings, rule 4 production reachability, rule 5 no cache or
//! readiness state; the operation total is derived and must match
//! `docs/storage-contract.md`. SURFACE.txt's header states the rules.
//!
//! `TINE_SHALLOW_PRINT=1 cargo test -p tine-store --test shallow_ratchet`
//! prints the current listing.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use syn::{Attribute, ImplItem, Item, Visibility};

fn is_pub(v: &Visibility) -> bool {
    matches!(v, Visibility::Public(_))
}

thread_local! {
    /// Evaluate `feature = "test-faults"` as enabled: the scan then sees the
    /// test-oracle surface too (rule 4).
    static FAULTS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Evaluates a `cfg` predicate for a production (non-test) build: `test` and
/// `feature = "test-faults"` are false (the latter true under [`FAULTS`]),
/// every other predicate is unknown.
/// `None` = unknown, which counts as compiled in (conservative).
fn cfg_value(meta: &syn::Meta) -> Option<bool> {
    match meta {
        syn::Meta::Path(p) if p.is_ident("test") => Some(false),
        syn::Meta::NameValue(nv) if nv.path.is_ident("feature") => {
            use quote::ToTokens;
            let value = nv.value.to_token_stream().to_string();
            (value.trim_matches('"') == "test-faults").then(|| FAULTS.with(|f| f.get()))
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

/// True when the attributes compile the item out of the build being scanned
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
    walk_from(&src.join("lib.rs"), src)
}

/// [`walk`] from any crate root file whose `mod x;` children live in `dir`.
fn walk_from(root: &Path, dir: &Path) -> (Vec<Module>, BTreeSet<PathBuf>) {
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
    visit(root, dir, "crate", true, false, &mut out, &mut visited);
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
    /// Rule 5 evidence per item: its doc comment, and whether a function
    /// answers `Option<bool>`.
    docs: std::collections::BTreeMap<String, String>,
    option_bool: BTreeSet<String>,
    /// Identifiers of each function signature and field type (rule 5).
    signatures: std::collections::BTreeMap<String, Vec<String>>,
    visited: BTreeSet<PathBuf>,
}

/// The `///` text of an item.
fn doc_text(attrs: &[Attribute]) -> String {
    use quote::ToTokens;
    attrs
        .iter()
        .filter(|a| a.path().is_ident("doc"))
        .filter_map(|a| match &a.meta {
            syn::Meta::NameValue(nv) => Some(nv.value.to_token_stream().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `-> Option<bool>`: an answer whose `None` means "unknown" (rule 5).
fn answers_option_bool(sig: &syn::Signature) -> bool {
    let syn::ReturnType::Type(_, ty) = &sig.output else {
        return false;
    };
    quote_type(ty).replace(' ', "") == "Option<bool>"
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
        docs: Default::default(),
        option_bool: BTreeSet::new(),
        signatures: Default::default(),
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
                                if answers_option_bool(&f.sig) {
                                    s.option_bool.insert(format!("fn {key}"));
                                }
                                s.docs.insert(format!("fn {key}"), doc_text(&f.attrs));
                                s.signatures.insert(format!("fn {key}"), idents(&f.sig));
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
                            if answers_option_bool(&f.sig) {
                                s.option_bool.insert(format!("fn {key}"));
                            }
                            s.signatures.insert(format!("fn {key}"), idents(&f.sig));
                            "fn"
                        }
                        Item::Struct(st) => {
                            for (index, f) in st.fields.iter().enumerate() {
                                if !is_pub(&f.vis) || is_cfg_test(&f.attrs) {
                                    continue;
                                }
                                let field = f
                                    .ident
                                    .as_ref()
                                    .map_or_else(|| index.to_string(), ToString::to_string);
                                if f.ident.is_some() {
                                    s.items.insert(format!("field {key}.{field}"));
                                    s.docs
                                        .insert(format!("field {key}.{field}"), doc_text(&f.attrs));
                                    s.signatures
                                        .insert(format!("field {key}.{field}"), idents(&f.ty));
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
                    s.docs.insert(format!("{kind} {key}"), doc_text(attrs));
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

/// The surface of a build with `test-faults` enabled.
fn surface_with_faults(src: &Path) -> Surface {
    FAULTS.with(|f| f.set(true));
    let s = surface(src);
    FAULTS.with(|f| f.set(false));
    s
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

/// The concepts the surface is budgeted by (og-surface review §6 rule 2). A
/// new concept needs Martin; it is added here, visibly, not in SURFACE.txt.
const CONCEPTS: &[&str] = &[
    "Lifecycle",
    "Freshness",
    "Identity",
    "Reads",
    "Config",
    "Save",
    "Transaction steps",
    "Trash admin",
    "Diagnostics",
    "Listing",
    "Publication",
    "Restore",
    "WholeGraph: Names",
    "WholeGraph: References",
    "WholeGraph: Block refs",
    "WholeGraph: Search/Query",
    "WholeGraph: View meta",
    // Approved by Martin 2026-10-10 (surface concept): STEP3 §7 retained writers and restore.
    "Page host",
];

/// Bookkeeping, not a concept: the `pub use` lines that re-export items
/// counted in their concepts.
const REEXPORTS: &str = "Re-exports";

struct Section {
    name: String,
    ceiling: usize,
    entries: Vec<String>,
}

/// SURFACE.txt: `## Concept (ceiling: N)` sections of entries. A
/// `# test-oracle: <reason>` comment marks the entry after it (rule 4).
struct SurfaceFile {
    sections: Vec<Section>,
    /// Entries before the first section header.
    unsectioned: Vec<String>,
    oracles: BTreeSet<String>,
}

impl SurfaceFile {
    fn read() -> SurfaceFile {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("SURFACE.txt");
        let text = std::fs::read_to_string(&path).unwrap();
        let mut file = SurfaceFile {
            sections: Vec::new(),
            unsectioned: Vec::new(),
            oracles: BTreeSet::new(),
        };
        let mut oracle = false;
        for line in text.lines().map(str::trim) {
            if let Some(header) = line.strip_prefix("## ") {
                let (name, rest) = header.split_once(" (ceiling: ").unwrap_or_else(|| {
                    panic!("SURFACE.txt: `## Concept (ceiling: N)`, got `{line}`")
                });
                let ceiling = rest
                    .strip_suffix(')')
                    .and_then(|n| n.parse().ok())
                    .unwrap_or_else(|| panic!("SURFACE.txt: bad ceiling in `{line}`"));
                file.sections.push(Section {
                    name: name.to_owned(),
                    ceiling,
                    entries: Vec::new(),
                });
            } else if let Some(reason) = line.strip_prefix("# test-oracle:") {
                assert!(
                    !reason.trim().is_empty(),
                    "SURFACE.txt: `# test-oracle:` needs a reason"
                );
                oracle = true;
            } else if !line.is_empty() && !line.starts_with('#') {
                if oracle {
                    file.oracles.insert(line.to_owned());
                    oracle = false;
                }
                match file.sections.last_mut() {
                    Some(section) => section.entries.push(line.to_owned()),
                    None => file.unsectioned.push(line.to_owned()),
                }
            }
        }
        assert!(
            !oracle,
            "SURFACE.txt ends with a dangling `# test-oracle:` tag"
        );
        file
    }

    fn entries(&self) -> impl Iterator<Item = &String> {
        self.sections.iter().flat_map(|s| &s.entries)
    }
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
    let file = SurfaceFile::read();
    let listed: Vec<&String> = file.entries().chain(&file.unsectioned).collect();
    let surface: BTreeSet<String> = listed.iter().map(|s| (*s).clone()).collect();
    assert_eq!(
        listed.len(),
        surface.len(),
        "SURFACE.txt has duplicate entries"
    );
    let (ceiling, shallow) = read_list(&root.join("SHALLOW.txt"));
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
         or add them to their concept in SURFACE.txt (raising its ceiling needs a reason). \
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
    let stale_surface: Vec<&String> = surface
        .iter()
        .filter(|s| !public.contains(*s) && !file.oracles.contains(*s))
        .collect();
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

/// Rule 2: each concept's entry count is its ceiling. A cut lowers the
/// ceiling; growth is a visible edit of it, whose commit names the existing
/// item of the concept that cannot answer the need with a parameter.
#[test]
fn surface_concepts_stay_within_their_ceilings() {
    let file = SurfaceFile::read();
    assert!(
        file.unsectioned.is_empty(),
        "tine-store rule 2: SURFACE.txt entries outside any `## Concept (ceiling: N)` section; \
         put each in its concept:\n{:#?}",
        file.unsectioned
    );
    let mut seen = BTreeSet::new();
    for section in &file.sections {
        assert!(
            seen.insert(section.name.clone()),
            "SURFACE.txt: section `{}` appears twice",
            section.name
        );
        assert!(
            CONCEPTS.contains(&section.name.as_str()) || section.name == REEXPORTS,
            "tine-store rule 2: `{}` is not a surface concept. A new concept needs Martin; \
             known concepts: {CONCEPTS:?}",
            section.name
        );
        let uses = section
            .entries
            .iter()
            .filter(|e| e.starts_with("use "))
            .count();
        if section.name == REEXPORTS {
            assert_eq!(
                uses,
                section.entries.len(),
                "SURFACE.txt `{REEXPORTS}` holds only `use` lines"
            );
        } else {
            assert_eq!(
                uses, 0,
                "SURFACE.txt: `use` lines belong in `{REEXPORTS}`, not `{}`",
                section.name
            );
        }
        assert!(
            section.entries.len() <= section.ceiling,
            "tine-store rule 2: concept `{}` has {} items, ceiling {}. Answer the need with a \
             parameter on an existing item of the concept (rule 3: performance work adds no \
             surface; exemplars `WholeGraph::inventory(InventoryScope)`, `Store::refresh(Depth)`), \
             or raise the ceiling in a commit that names the item that cannot and why",
            section.name,
            section.entries.len(),
            section.ceiling
        );
        assert_eq!(
            section.entries.len(),
            section.ceiling,
            "tine-store rule 2: concept `{}` shrank to {} items; lower its ceiling to match \
             (a cut is recorded, never left as slack for the next addition)",
            section.name,
            section.entries.len()
        );
    }
}

/// The derived totals (operations, WholeGraph questions, types) and the
/// contract's stated operation count, which must agree.
#[test]
fn derived_totals_match_the_storage_contract() {
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
    let contract = std::fs::read_to_string(root.join("../../docs/storage-contract.md")).unwrap();
    let flat = contract.split_whitespace().collect::<Vec<_>>().join(" ");
    let stated = flat
        .split_once("The public operation surface is ")
        .and_then(|(_, rest)| rest.split(' ').next())
        .and_then(|n| n.parse::<usize>().ok())
        .expect("docs/storage-contract.md states `The public operation surface is N operations`");
    assert_eq!(
        stated, operations,
        "docs/storage-contract.md says the store has {stated} operations; the scan counts \
         {operations} (Store + Transaction methods plus functions taking either). Update the \
         contract in the same commit"
    );
}

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
            "store::Store::suggest_graph_name",
            "user-chosen parent folder input; read-only suggestion before graph creation",
        ),
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
            "page_host::PageHost::start",
            "host-chosen app-data crash-draft location (OS hand-off)",
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

/// Every name a production (non-test) source file of a consumer crate uses:
/// method calls, path segments, field accesses and struct-literal fields, and
/// every identifier inside a macro invocation (its arguments are not parsed).
/// Items compiled out of a production build are skipped.
#[derive(Default)]
struct Reach {
    names: BTreeSet<String>,
}

impl Reach {
    fn tokens(&mut self, tokens: &proc_macro2::TokenStream) {
        for tree in tokens.clone() {
            match tree {
                proc_macro2::TokenTree::Ident(i) => {
                    self.names.insert(i.to_string());
                }
                proc_macro2::TokenTree::Group(g) => self.tokens(&g.stream()),
                _ => {}
            }
        }
    }
}

impl<'ast> syn::visit::Visit<'ast> for Reach {
    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        if !is_cfg_test(&i.attrs) {
            syn::visit::visit_item_fn(self, i);
        }
    }
    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        if !is_cfg_test(&i.attrs) {
            syn::visit::visit_item_impl(self, i);
        }
    }
    fn visit_item_mod(&mut self, i: &'ast syn::ItemMod) {
        if !is_cfg_test(&i.attrs) {
            syn::visit::visit_item_mod(self, i);
        }
    }
    fn visit_item_const(&mut self, i: &'ast syn::ItemConst) {
        if !is_cfg_test(&i.attrs) {
            syn::visit::visit_item_const(self, i);
        }
    }
    fn visit_item_static(&mut self, i: &'ast syn::ItemStatic) {
        if !is_cfg_test(&i.attrs) {
            syn::visit::visit_item_static(self, i);
        }
    }
    fn visit_item_trait(&mut self, i: &'ast syn::ItemTrait) {
        if !is_cfg_test(&i.attrs) {
            syn::visit::visit_item_trait(self, i);
        }
    }
    fn visit_item_struct(&mut self, i: &'ast syn::ItemStruct) {
        if !is_cfg_test(&i.attrs) {
            syn::visit::visit_item_struct(self, i);
        }
    }
    fn visit_item_enum(&mut self, i: &'ast syn::ItemEnum) {
        if !is_cfg_test(&i.attrs) {
            syn::visit::visit_item_enum(self, i);
        }
    }
    fn visit_item_type(&mut self, i: &'ast syn::ItemType) {
        if !is_cfg_test(&i.attrs) {
            syn::visit::visit_item_type(self, i);
        }
    }
    fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
        if !is_cfg_test(&i.attrs) {
            syn::visit::visit_impl_item_fn(self, i);
        }
    }
    fn visit_stmt(&mut self, i: &'ast syn::Stmt) {
        let attrs: &[Attribute] = match i {
            syn::Stmt::Local(l) => &l.attrs,
            syn::Stmt::Macro(m) => &m.attrs,
            _ => &[],
        };
        if !is_cfg_test(attrs) {
            syn::visit::visit_stmt(self, i);
        }
    }
    fn visit_expr_method_call(&mut self, i: &'ast syn::ExprMethodCall) {
        self.names.insert(i.method.to_string());
        syn::visit::visit_expr_method_call(self, i);
    }
    fn visit_path(&mut self, i: &'ast syn::Path) {
        for segment in &i.segments {
            self.names.insert(segment.ident.to_string());
        }
        syn::visit::visit_path(self, i);
    }
    fn visit_member(&mut self, i: &'ast syn::Member) {
        if let syn::Member::Named(n) = i {
            self.names.insert(n.to_string());
        }
    }
    fn visit_macro(&mut self, i: &'ast syn::Macro) {
        self.tokens(&i.tokens);
    }
}

/// The production crate roots that consume tine-store (rule 4): its own
/// binaries, graph-features, and the desktop app. The wasm parser crate does
/// not depend on tine-store.
fn consumer_reach() -> BTreeSet<String> {
    let store = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = store.parent().unwrap().parent().unwrap();
    let mut roots = vec![
        workspace.join("crates/tine-graph-features/src/lib.rs"),
        workspace.join("src-tauri/src/lib.rs"),
        workspace.join("src-tauri/src/main.rs"),
    ];
    for e in std::fs::read_dir(store.join("src/bin")).unwrap() {
        roots.push(e.unwrap().path());
    }
    let mut reach = Reach::default();
    for root in roots {
        let (modules, _) = walk_from(&root, root.parent().unwrap());
        for m in modules {
            for item in &m.items {
                // Nested modules are walked as their own entries.
                if matches!(item, Item::Mod(_)) {
                    continue;
                }
                syn::visit::Visit::visit_item(&mut reach, item);
            }
        }
    }
    reach.names
}

/// Rule 4: every function, constant and field on the surface has a production
/// caller in a consuming crate, or is a tagged, `test-faults`-gated oracle.
/// Callers are matched by name, so a same-named item elsewhere can mask a
/// miss; types are reached through the signatures that carry them.
#[test]
fn surface_items_have_a_production_caller_or_are_gated_oracles() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let production = surface(&src).items;
    let gated: BTreeSet<String> = surface_with_faults(&src)
        .items
        .difference(&production)
        .cloned()
        .collect();
    let file = SurfaceFile::read();
    let misgated: Vec<&String> = file
        .oracles
        .iter()
        .filter(|o| !gated.contains(*o))
        .collect();
    assert!(
        misgated.is_empty(),
        "tine-store rule 4: a `# test-oracle:` entry must be compiled only with \
         `#[cfg(any(test, feature = \"test-faults\"))]`; exemplar `Subscription::try_recv`:\n{misgated:#?}"
    );
    let reach = consumer_reach();
    let unreached: Vec<&String> = file
        .entries()
        .filter(|e| ["fn ", "const ", "field "].iter().any(|k| e.starts_with(k)))
        .filter(|e| !file.oracles.contains(*e))
        .filter(|e| {
            let name = e.rsplit([':', '.', ' ']).next().unwrap();
            !reach.contains(name)
        })
        .collect();
    assert!(
        unreached.is_empty(),
        "tine-store rule 4: these surface items have no production caller in \
         tine-graph-features, src-tauri or a tine-store binary. Make them private, or gate a \
         test oracle with `#[cfg(any(test, feature = \"test-faults\"))]` and tag it \
         `# test-oracle: <reason>` in SURFACE.txt (exemplar `Subscription::try_recv`):\n{unreached:#?}"
    );
}

/// Words that promise cache, load or readiness state (rule 5).
fn state_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    for token in text.split(|c: char| !c.is_alphanumeric()) {
        // Split identifiers at `_` (already done) and at camel-case humps.
        let mut word = String::new();
        let mut parts = Vec::new();
        for c in token.chars() {
            if c.is_uppercase() && !word.is_empty() {
                parts.push(std::mem::take(&mut word));
            }
            word.extend(c.to_lowercase());
        }
        parts.push(word);
        for part in parts {
            if ["cache", "ready", "readiness", "loaded", "generation"]
                .iter()
                .any(|k| part.starts_with(k))
            {
                words.push(part);
            }
        }
    }
    words
}

/// Rule 5 (arrival rule A, made mechanical): no public name or signature
/// promises cache, load or readiness state, and no answer is `Option<bool>`
/// "unknown". Docs that use those words to state a cost or an internal
/// mechanism are listed with why they promise no caller-visible state.
#[test]
fn no_cache_or_readiness_state_on_the_surface() {
    let s = surface(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"));
    let named: Vec<String> = s
        .items
        .iter()
        .filter(|item| !item.starts_with("use "))
        .filter_map(|item| {
            let words = state_words(item);
            let sig = s
                .signatures
                .get(item)
                .map(|idents| state_words(&idents.join(" ")))
                .unwrap_or_default();
            (!words.is_empty() || !sig.is_empty()).then(|| format!("{item}: {words:?} {sig:?}"))
        })
        .collect();
    assert!(
        named.is_empty(),
        "tine-store rule 5: a public name or signature promises cache/load/readiness state. \
         Ask the domain question instead (exemplar `Store::may_carry_vcs_markers`):\n{named:#?}"
    );
    assert!(
        s.option_bool.is_empty(),
        "tine-store rule 5: `Option<bool>` answers \"unknown\"; answer the domain question \
         conservatively instead (exemplar `Store::may_carry_vcs_markers`):\n{:#?}",
        s.option_bool
    );
    let documented: BTreeSet<String> = s
        .docs
        .iter()
        .filter(|(_, doc)| !state_words(doc).is_empty())
        .map(|(item, _)| item.clone())
        .collect();
    if std::env::var_os("TINE_SHALLOW_PRINT").is_some() {
        for item in &documented {
            println!("rule5-doc {item}: {:?}", state_words(&s.docs[item]));
        }
    }
    let allowed: BTreeSet<String> = RULE5_DOCS
        .iter()
        .map(|(item, reason)| {
            assert!(!reason.is_empty());
            (*item).to_owned()
        })
        .collect();
    assert_eq!(
        documented, allowed,
        "tine-store rule 5: a public item's doc mentions cache/ready/loaded/generation. If it \
         promises callers that state, make it a domain question (row 10); if it only states a \
         cost or internal mechanism, list it in RULE5_DOCS with why"
    );
}

/// Docs that mention cache/readiness vocabulary without exposing that state.
const RULE5_DOCS: &[(&str, &str)] = &[
    (
        "struct page_host::PageHost",
        "names the window generation every command carries, a protocol field; nothing answers it",
    ),
    (
        "struct page_host::Reservation",
        "lists the host steps a reservation fences (open, load, save, draft); no state is returned",
    ),
    (
        "fn store::Store::diagnostics",
        "the diagnostics dump reports internal build state by design; no caller branches on it",
    ),
    (
        "fn store::Store::journal_id",
        "cost note: the id comes from the held day index, no disk read",
    ),
    (
        "fn store::Store::may_carry_vcs_markers",
        "the rule 5 exemplar: an unknown answer folds into the conservative `true`",
    ),
    (
        "fn store::Store::page",
        "locking and publication mechanism; the read returns the page, never a load state",
    ),
    (
        "fn store::Store::page_named",
        "states when the call waits for the graph; it returns the page, never a load state",
    ),
    (
        "fn store::Store::refresh",
        "cost and recovery note; returns the change-feed revision",
    ),
    (
        "fn store::Store::whole_graph",
        "states when the call waits for the initial parse; it returns a view or LoadError",
    ),
    (
        "fn store::WholeGraph::block_ref_counts",
        "cost note: the first read materializes the map, later reads clone it",
    ),
    (
        "fn store::WholeGraph::query_ir",
        "cost note: the registry is built once per published graph",
    ),
];
