//! I-4/I-22: fixed byte-prefix probes must use checked `str::get`, as in
//! tine-core/src/sync_diff.rs::without_id_line. Length alone proves no UTF-8 boundary.
use quote::ToTokens;
use std::{collections::HashSet, fs, path::Path};
use syn::{
    visit::{self, Visit},
    Expr, Item,
};

#[derive(Default)]
struct PrefixSlices {
    byte_buffers: HashSet<String>,
    violations: Vec<String>,
}

impl<'ast> Visit<'ast> for PrefixSlices {
    fn visit_item(&mut self, item: &'ast Item) {
        let attrs = match item {
            Item::Fn(item) => &item.attrs,
            Item::Mod(item) => &item.attrs,
            _ => {
                visit::visit_item(self, item);
                return;
            }
        };
        if attrs.iter().any(|attr| {
            attr.path().is_ident("test")
                || (attr.path().is_ident("cfg")
                    && attr.meta.to_token_stream().to_string() == "cfg (test)")
        }) {
            return;
        }
        visit::visit_item(self, item);
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        let previous = std::mem::take(&mut self.byte_buffers);
        visit::visit_item_fn(self, item);
        self.byte_buffers = previous;
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        if let (syn::Pat::Ident(name), Some(init)) = (&local.pat, &local.init) {
            if matches!(init.expr.as_ref(), Expr::MethodCall(call) if call.method == "as_bytes") {
                self.byte_buffers.insert(name.ident.to_string());
            }
        }
        visit::visit_local(self, local);
    }

    fn visit_expr_index(&mut self, index: &'ast syn::ExprIndex) {
        if let Expr::Range(range) = index.index.as_ref() {
            let fixed_end = range.end.as_deref().is_some_and(|end|
                matches!(end, Expr::Lit(lit) if matches!(&lit.lit, syn::Lit::Int(n) if n.base10_parse::<usize>().is_ok_and(|n| n > 0))));
            let bytes = matches!(index.expr.as_ref(), Expr::MethodCall(call) if call.method == "as_bytes")
                || matches!(index.expr.as_ref(), Expr::Path(path) if self.byte_buffers.contains(&path.to_token_stream().to_string()));
            if fixed_end && !bytes {
                self.violations.push(index.to_token_stream().to_string());
            }
        }
        visit::visit_expr_index(self, index);
    }
}

fn scan(path: &Path, violations: &mut Vec<String>) {
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            scan(&path, violations);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            && !path.file_name().unwrap().to_string_lossy().contains("test")
        {
            let mut guard = PrefixSlices::default();
            guard.visit_file(&syn::parse_file(&fs::read_to_string(&path).unwrap()).unwrap());
            violations.extend(
                guard
                    .violations
                    .into_iter()
                    .map(|index| format!("{}: {index}", path.display())),
            );
        }
    }
}

#[test]
fn fixed_byte_prefixes_are_checked_throughout_core_and_store() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut violations = Vec::new();
    for source in ["tine-core/src", "tine-store/src"] {
        scan(&root.join(source), &mut violations);
    }
    assert!(violations.is_empty(), "I-4/I-22: byte length does not prove a UTF-8 boundary; \
        probe fixed text prefixes through str::get, as in tine-core/src/sync_diff.rs::without_id_line. \
        Byte buffers are safe. Found: {violations:#?}");
}

#[test]
fn prefix_guard_rejects_length_only_and_accepts_checked_or_byte_access() {
    let mut guard = PrefixSlices::default();
    guard.visit_file(
        &syn::parse_file(
            r#"
        fn bad(t: &str) { if t.len() > 4 { let _ = &t[..4]; } }
        fn variable(t: &str, end: usize) { let _ = t.get(..end); }
        fn good(t: &str) { let _ = t.get(..4); let b = t.as_bytes(); let _ = &b[..4]; }
    "#,
        )
        .unwrap(),
    );
    assert_eq!(guard.violations, ["t [.. 4]"]);
}

#[test]
fn transaction_unchanged_path_checks_the_same_guard_as_changed_writes() {
    let source = include_str!("../src/transaction.rs");
    let start = source
        .find("Step::Save { .. } | Step::Replace { .. } | Step::Rewrite { .. } =>")
        .unwrap();
    let apply = &source[start..];
    assert!(apply.find("self.verify(&plan.src, old, index)?;").unwrap() < apply.find("if old == Some(new.as_slice())").unwrap(),
        "I-2/I-12: all Save/Replace/Rewrite steps verify preflight bytes before returning Unchanged; \
         exemplar Transaction::apply in tine-store/src/transaction.rs");
}

#[test]
fn disabled_tql_diagnostics_have_one_owner() {
    let source = include_str!("../../tine-core/src/query/tql.rs");
    assert_eq!(
        source.matches("self.diagnostics.push(").count(),
        1,
        "I-4/I-12: every TQL lowering diagnostic goes through Lower::diagnose, which marks off() \
         diagnostics disabled; exemplar tine-core/src/query/tql.rs::diagnose"
    );
}
