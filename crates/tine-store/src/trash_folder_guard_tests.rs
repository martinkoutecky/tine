//! The graph trash's kind folders are spelled in one place,
//! `TrashEntryKind::dir_name` in `model.rs`; every other production module
//! asks it (`typed_trash_dir`, `TrashEntryKind::graph_rel`). A production
//! item outside `model.rs` that names the trash root (a `.tine-trash` string
//! or a `trash_root` call) and also writes a kind folder's name, or that
//! writes `.tine-trash/<kind>`, spells a folder of its own.
//!
//! Exempt by name: checks that only ask whether a file is under the trash
//! root, and restore's own retirement tree under the trash root.
use crate::test_cfg::is_test;
use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use std::path::{Path, PathBuf};

const KINDS: [&str; 5] = ["pages", "journals", "conflicts", "assets", "other"];

/// (file, item) pairs that may spell a folder; `None` exempts the whole file.
/// Besides the owner and restore, each is a check that a path is under the
/// trash root, where `"assets"` is the graph's own assets folder.
const EXEMPT: [(&str, Option<&str>); 7] = [
    ("crates/tine-store/src/model.rs", None),
    ("crates/tine-store/src/restore.rs", None),
    ("crates/tine-store/src/transaction/preflight.rs", None),
    ("crates/tine-store/src/store/page_identity.rs", None),
    ("crates/tine-store/src/store.rs", Some("file_id")),
    ("crates/tine-store/src/store.rs", Some("scan_area")),
    (
        "crates/tine-store/src/model/page_identity.rs",
        Some("graph_text_directory_scannable"),
    ),
];

/// One item's string literals (doc attributes skipped) and whether it calls
/// `trash_root`.
fn scan(tokens: TokenStream, literals: &mut Vec<String>, calls_root: &mut bool) {
    let tokens: Vec<_> = tokens.into_iter().collect();
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            TokenTree::Punct(p) if p.as_char() == '#' => {
                // An attribute (`#[doc = "…"]`, `#[cfg(…)]`) names no folder.
                if matches!(tokens.get(i + 1), Some(TokenTree::Group(_))) {
                    i += 2;
                    continue;
                }
            }
            TokenTree::Group(group) => scan(group.stream(), literals, calls_root),
            TokenTree::Ident(ident) if ident == "trash_root" => *calls_root = true,
            TokenTree::Literal(literal) => {
                if let Ok(syn::Lit::Str(text)) = syn::parse_str::<syn::Lit>(&literal.to_string()) {
                    literals.push(text.value());
                }
            }
            _ => {}
        }
        i += 1;
    }
}

/// What in one item spells a kind folder.
fn spellings(tokens: TokenStream) -> Vec<String> {
    let (mut literals, mut calls_root) = (Vec::new(), false);
    scan(tokens, &mut literals, &mut calls_root);
    let names_root = calls_root || literals.iter().any(|text| text.contains(".tine-trash"));
    let mut found = Vec::new();
    for text in &literals {
        let spelled_path = KINDS.iter().any(|kind| {
            text.match_indices(&format!(".tine-trash/{kind}"))
                .any(|(at, m)| matches!(text[at + m.len()..].chars().next(), None | Some('/')))
        });
        if spelled_path || (names_root && KINDS.contains(&text.as_str())) {
            found.push(text.clone());
        }
    }
    found
}

/// Each production item of a parsed file, named, with its tokens.
fn items(items: &[syn::Item], out: &mut Vec<(String, TokenStream)>) {
    for item in items {
        match item {
            syn::Item::Fn(f) if !is_test(&f.attrs) => {
                out.push((f.sig.ident.to_string(), f.to_token_stream()))
            }
            syn::Item::Impl(block) if !is_test(&block.attrs) => {
                for member in &block.items {
                    match member {
                        syn::ImplItem::Fn(f) if !is_test(&f.attrs) => {
                            out.push((f.sig.ident.to_string(), f.block.to_token_stream()))
                        }
                        syn::ImplItem::Fn(_) => {}
                        other => out.push((String::new(), other.to_token_stream())),
                    }
                }
            }
            syn::Item::Mod(m) if !is_test(&m.attrs) => {
                if let Some((_, inner)) = &m.content {
                    self::items(inner, out);
                }
            }
            syn::Item::Fn(_) | syn::Item::Impl(_) | syn::Item::Mod(_) => {}
            other => {
                let attrs: &[syn::Attribute] = match other {
                    syn::Item::Const(c) => &c.attrs,
                    syn::Item::Static(s) => &s.attrs,
                    syn::Item::Trait(t) => &t.attrs,
                    syn::Item::Macro(m) => &m.attrs,
                    _ => &[],
                };
                if !is_test(attrs) {
                    out.push((String::new(), other.to_token_stream()));
                }
            }
        }
    }
}

/// The production module files reachable from `file` through `mod`
/// declarations without a test-only `cfg`.
fn production_modules(file: &Path, out: &mut Vec<PathBuf>) {
    let source = std::fs::read_to_string(file).unwrap();
    out.push(file.to_path_buf());
    let parsed = syn::parse_file(&source).unwrap();
    let dir = file.parent().unwrap();
    let stem = file.file_stem().unwrap().to_str().unwrap();
    let own_dir = if matches!(stem, "lib" | "main" | "mod") {
        dir.to_path_buf()
    } else {
        dir.join(stem)
    };
    fn walk(items: &[syn::Item], base: &Path, file_dir: &Path, out: &mut Vec<PathBuf>) {
        for item in items {
            let syn::Item::Mod(m) = item else { continue };
            if is_test(&m.attrs) {
                continue;
            }
            let name = m.ident.to_string();
            if let Some((_, inner)) = &m.content {
                walk(inner, &base.join(&name), file_dir, out);
                continue;
            }
            let explicit = m.attrs.iter().find_map(|attr| match &attr.meta {
                syn::Meta::NameValue(nv) if nv.path.is_ident("path") => match &nv.value {
                    syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(path),
                        ..
                    }) => Some(file_dir.join(path.value())),
                    _ => None,
                },
                _ => None,
            });
            let path = explicit.unwrap_or_else(|| {
                let flat = base.join(format!("{name}.rs"));
                if flat.is_file() {
                    flat
                } else {
                    base.join(&name).join("mod.rs")
                }
            });
            production_modules(&path, out);
        }
    }
    walk(&parsed.items, &own_dir, dir, out);
}

fn crate_roots(workspace: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut sources = vec![workspace.join("src-tauri/src")];
    for entry in std::fs::read_dir(workspace.join("crates")).unwrap() {
        sources.push(entry.unwrap().path().join("src"));
    }
    for src in sources {
        for root in ["lib.rs", "main.rs"] {
            if src.join(root).is_file() {
                roots.push(src.join(root));
            }
        }
        if let Ok(bins) = std::fs::read_dir(src.join("bin")) {
            for bin in bins {
                roots.push(bin.unwrap().path());
            }
        }
    }
    roots
}

#[test]
fn only_the_trash_model_spells_a_kind_folder() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workspace = workspace.canonicalize().unwrap();
    let mut files = Vec::new();
    for root in crate_roots(&workspace) {
        production_modules(&root, &mut files);
    }
    files.sort();
    files.dedup();
    assert!(
        files.len() > 100,
        "the guard reads every production module, found {}",
        files.len()
    );
    let mut found = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(&workspace)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if EXEMPT.contains(&(rel.as_str(), None)) {
            continue;
        }
        let parsed = syn::parse_file(&std::fs::read_to_string(&file).unwrap()).unwrap();
        let mut units = Vec::new();
        items(&parsed.items, &mut units);
        for (name, tokens) in units {
            if EXEMPT.contains(&(rel.as_str(), Some(name.as_str()))) {
                continue;
            }
            for text in spellings(tokens) {
                found.push(format!("{rel}::{name}: {text:?}"));
            }
        }
    }
    assert!(
        found.is_empty(),
        "kind folders spelled outside model.rs:\n{}",
        found.join("\n")
    );
}

/// The forms the folders were spelled in before they had one owner.
#[test]
fn the_guard_sees_each_way_a_folder_was_spelled() {
    let spelled = |code: &str| spellings(code.parse().unwrap());
    for code in [
        r#"fn f() { let t = trash_root(&root).join("pages"); }"#,
        r#"fn f() { let t = crate::model::trash_root(&r).join("pages").join(n); }"#,
        r#"fn f() { format!("logseq/.tine-trash/conflicts/{}", n) }"#,
        r#"fn f() { let k = if a { "conflicts" } else { "other" }; format!("logseq/.tine-trash/{k}/{}", n) }"#,
        r#"fn f() { let base = root.join("logseq/.tine-trash"); for kind in ["pages", "journals"] {} }"#,
        r#"const L: [&str; 1] = ["logseq/.tine-trash/pages/tine-ignore-check.md"];"#,
        r#"fn f() { Path::new("logseq").join(".tine-trash").join("conflicts") }"#,
        r#"fn f() { e.replace("logseq/.tine-trash/assets", &x) }"#,
    ] {
        assert!(!spelled(code).is_empty(), "missed: {code}");
    }
    for code in [
        r#"fn f() { rel.starts_with("logseq/.tine-trash/") }"#,
        r#"fn f() { let pages = "pages"; }"#,
        r#"fn f() { typed_trash_dir(&root, TrashEntryKind::Page) }"#,
        r#"/// See `logseq/.tine-trash/pages`.
        fn f() {}"#,
    ] {
        assert!(spelled(code).is_empty(), "flagged: {code}");
    }
}

/// Whether `tokens` hold the identifier `name`.
fn names(tokens: TokenStream, name: &str) -> bool {
    tokens.into_iter().any(|token| match token {
        TokenTree::Ident(ident) => ident == name,
        TokenTree::Group(group) => names(group.stream(), name),
        _ => false,
    })
}

/// One producer builds a kind folder's path: in `model.rs`, only
/// `TrashEntryKind::folder` joins the trash root and a folder name, and
/// `trash_dir_kind` reads names back; the trash root is spelled once.
#[test]
fn one_function_builds_a_kind_folder() {
    let model = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/model.rs");
    let parsed = syn::parse_file(&std::fs::read_to_string(model).unwrap()).unwrap();
    let mut units = Vec::new();
    items(&parsed.items, &mut units);
    // Unnamed items (consts, statics) build no path; the root's const is one.
    let users = |name: &str| {
        let mut found: Vec<_> = (units.iter())
            .filter(|(item, tokens)| {
                !item.is_empty() && item != name && names(tokens.clone(), name)
            })
            .map(|(item, _)| item.clone())
            .collect();
        found.sort();
        found
    };
    assert_eq!(users("dir_name"), ["folder", "trash_dir_kind"]);
    assert_eq!(users("TRASH_ROOT"), ["folder", "trash_root"]);
    let spelled_root = (units.iter())
        .filter(|(_, tokens)| {
            let (mut literals, mut calls_root) = (Vec::new(), false);
            scan(tokens.clone(), &mut literals, &mut calls_root);
            literals.iter().any(|text| text.contains(".tine-trash"))
        })
        .count();
    assert_eq!(
        spelled_root, 1,
        "the trash root is spelled once in model.rs"
    );
}
