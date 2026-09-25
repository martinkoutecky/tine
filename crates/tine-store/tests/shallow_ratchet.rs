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

fn is_cfg_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && a.meta
                .require_list()
                .map(|l| {
                    l.tokens
                        .to_string()
                        .split(|c: char| !c.is_alphanumeric() && c != '_')
                        .any(|t| t == "test")
                })
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

fn collect(module: &str, items: &[Item], out: &mut BTreeSet<String>) {
    for item in items {
        let (vis, attrs, entry): (Option<&Visibility>, &[Attribute], Option<String>) = match item {
            Item::Fn(i) => (
                Some(&i.vis),
                &i.attrs,
                Some(format!("fn {module}::{}", i.sig.ident)),
            ),
            Item::Struct(i) => {
                if is_pub(&i.vis) && !is_cfg_test(&i.attrs) {
                    for f in &i.fields {
                        if let (true, Some(id)) = (is_pub(&f.vis), &f.ident) {
                            out.insert(format!("field {module}::{}.{}", i.ident, id));
                        }
                    }
                }
                (
                    Some(&i.vis),
                    &i.attrs,
                    Some(format!("struct {module}::{}", i.ident)),
                )
            }
            Item::Enum(i) => (
                Some(&i.vis),
                &i.attrs,
                Some(format!("enum {module}::{}", i.ident)),
            ),
            Item::Type(i) => (
                Some(&i.vis),
                &i.attrs,
                Some(format!("type {module}::{}", i.ident)),
            ),
            Item::Const(i) => (
                Some(&i.vis),
                &i.attrs,
                Some(format!("const {module}::{}", i.ident)),
            ),
            Item::Static(i) => (
                Some(&i.vis),
                &i.attrs,
                Some(format!("static {module}::{}", i.ident)),
            ),
            Item::Trait(i) => (
                Some(&i.vis),
                &i.attrs,
                Some(format!("trait {module}::{}", i.ident)),
            ),
            Item::Union(i) => (
                Some(&i.vis),
                &i.attrs,
                Some(format!("union {module}::{}", i.ident)),
            ),
            Item::Use(i) => {
                use quote::ToTokens;
                let tree = i.tree.to_token_stream().to_string().replace(' ', "");
                (
                    Some(&i.vis),
                    &i.attrs,
                    Some(format!("use {module}::{tree}")),
                )
            }
            Item::Macro(i) => {
                let exported = i.attrs.iter().any(|a| a.path().is_ident("macro_export"));
                match (&i.ident, exported) {
                    (Some(id), true) => (None, &i.attrs, Some(format!("macro {id}"))),
                    _ => (None, &i.attrs, None),
                }
            }
            Item::Mod(i) => {
                if is_pub(&i.vis) && !is_cfg_test(&i.attrs) {
                    if let Some((_, inner)) = &i.content {
                        collect(&format!("{module}::{}", i.ident), inner, out);
                    }
                }
                (None, &i.attrs, None)
            }
            Item::Impl(i) => {
                if i.trait_.is_none() && !is_cfg_test(&i.attrs) {
                    let ty = type_name(&i.self_ty);
                    for it in &i.items {
                        match it {
                            ImplItem::Fn(f) if is_pub(&f.vis) && !is_cfg_test(&f.attrs) => {
                                out.insert(format!("fn {module}::{ty}::{}", f.sig.ident));
                            }
                            ImplItem::Const(c) if is_pub(&c.vis) && !is_cfg_test(&c.attrs) => {
                                out.insert(format!("const {module}::{ty}::{}", c.ident));
                            }
                            _ => {}
                        }
                    }
                }
                (None, &i.attrs, None)
            }
            _ => (None, &[], None),
        };
        let Some(entry) = entry else { continue };
        if is_cfg_test(attrs) {
            continue;
        }
        if vis.is_none_or(is_pub) {
            out.insert(entry);
        }
    }
}

fn public_items(src: &Path) -> BTreeSet<String> {
    let lib = syn::parse_file(&std::fs::read_to_string(src.join("lib.rs")).unwrap()).unwrap();
    let mut out = BTreeSet::new();
    collect("crate", &lib.items, &mut out);
    for item in &lib.items {
        let Item::Mod(m) = item else { continue };
        if m.content.is_some() || !is_pub(&m.vis) || is_cfg_test(&m.attrs) {
            continue;
        }
        let file = src.join(format!("{}.rs", m.ident));
        let parsed = syn::parse_file(&std::fs::read_to_string(&file).unwrap())
            .unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        collect(&m.ident.to_string(), &parsed.items, &mut out);
    }
    out
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
