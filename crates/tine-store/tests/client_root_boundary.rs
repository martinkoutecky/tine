//! Guard the rev-5 graph-root ownership rule in production clients.
use quote::ToTokens;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use syn::visit::{self, Visit};

fn cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg") && attr.meta.to_token_stream().to_string().contains("test")
    })
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            && !path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("_tests")
        {
            out.push(path);
        }
    }
}

#[derive(Default)]
struct RootCalls {
    tainted: BTreeSet<String>,
    violations: Vec<String>,
}

impl RootCalls {
    fn graph_value(&self, expr: &syn::Expr) -> bool {
        // Follow direct aliases, references, and path joins. A call such as
        // backup_base(root_key) produces a device path, so taint stops there.
        match expr {
            syn::Expr::Path(path) => path
                .path
                .get_ident()
                .is_some_and(|name| name == "root_key" || self.tainted.contains(&name.to_string())),
            syn::Expr::Field(field) => {
                field.member.to_token_stream().to_string() == "root_key"
                    || (field.member.to_token_stream().to_string() == "root"
                        && field.base.to_token_stream().to_string() == "source")
            }
            syn::Expr::Reference(value) => self.graph_value(&value.expr),
            syn::Expr::Paren(value) => self.graph_value(&value.expr),
            syn::Expr::Group(value) => self.graph_value(&value.expr),
            syn::Expr::Try(value) => self.graph_value(&value.expr),
            syn::Expr::MethodCall(call)
                if ["join", "clone", "to_path_buf"].contains(&call.method.to_string().as_str()) =>
            {
                self.graph_value(&call.receiver)
            }
            syn::Expr::Call(call) => {
                let name = call.func.to_token_stream().to_string();
                name.ends_with("canonical_graph_root")
                    || name.ends_with("canonical_root")
                    || (name.ends_with("canonicalize")
                        && call.args.iter().any(|arg| self.graph_value(arg)))
            }
            _ => false,
        }
    }

    fn record(&mut self, operation: &str, args: &[&syn::Expr]) {
        if args.iter().any(|arg| self.graph_value(arg)) {
            self.violations.push(operation.to_owned());
        }
    }
}

impl<'ast> Visit<'ast> for RootCalls {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if !cfg_test(&item.attrs) {
            visit::visit_item_mod(self, item);
        }
    }
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if !cfg_test(&item.attrs) {
            let previous = std::mem::take(&mut self.tainted);
            visit::visit_item_fn(self, item);
            self.tainted = previous;
        }
    }
    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if !cfg_test(&item.attrs) {
            let previous = std::mem::take(&mut self.tainted);
            visit::visit_impl_item_fn(self, item);
            self.tainted = previous;
        }
    }
    fn visit_local(&mut self, local: &'ast syn::Local) {
        if let Some(init) = &local.init {
            self.visit_expr(&init.expr);
            if self.graph_value(&init.expr) {
                if let syn::Pat::Ident(ident) = &local.pat {
                    self.tainted.insert(ident.ident.to_string());
                }
            }
        }
    }
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        let target = call.func.to_token_stream().to_string();
        let operation = target.split(" :: ").last().unwrap_or("");
        const FS: &[&str] = &[
            "read",
            "read_to_string",
            "write",
            "canonicalize",
            "metadata",
            "symlink_metadata",
            "read_dir",
            "rename",
            "remove_file",
            "remove_dir",
            "remove_dir_all",
            "create_dir",
            "create_dir_all",
            "copy",
            "hard_link",
            "open",
        ];
        if FS.contains(&operation)
            && (target.contains("fs ::") || target.contains("File ::") || target == "File :: open")
        {
            let args: Vec<_> = call.args.iter().collect();
            self.record(&target, &args);
        }
        visit::visit_expr_call(self, call);
    }
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        const PATH_OPS: &[&str] = &[
            "canonicalize",
            "metadata",
            "symlink_metadata",
            "read_dir",
            "is_file",
            "is_dir",
            "exists",
            "open",
            "read_to_string",
        ];
        if PATH_OPS.contains(&call.method.to_string().as_str()) {
            self.record(&call.method.to_string(), &[&call.receiver]);
        }
        visit::visit_expr_method_call(self, call);
    }
}

fn scan(source: &str) -> Vec<String> {
    let parsed = syn::parse_file(source).unwrap();
    let mut scan = RootCalls::default();
    scan.visit_file(&parsed);
    scan.violations
}

#[test]
fn clients_never_operate_on_graph_root_paths() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in ["src-tauri/src", "crates/tine-graph-features/src"] {
        rust_files(&root.join(dir), &mut files);
    }
    let mut violations = Vec::new();
    for file in files {
        let source = std::fs::read_to_string(&file).unwrap();
        for operation in scan(&source) {
            violations.push(format!("{}: {operation}", file.display()));
        }
    }
    assert!(violations.is_empty(), "tine-store Rule 1: clients must use the store for graph-root file operations; imitate crates/tine-store/src/store.rs. Allowed device reads: read_local_image and read_text_file; backup writes are only to the device backup directory. Found: {violations:#?}");
}

#[test]
fn planted_graph_root_violation_is_detected() {
    let planted = r#"fn bad(slot: &GraphSlot) {
        let graph_file = slot.root_key.join("pages/Secret.md");
        let _ = std::fs::read(graph_file);
    }"#;
    assert!(!scan(planted).is_empty(), "tine-store Rule 1: source scan must catch a direct graph-root read; imitate crates/tine-store/src/store.rs");
}
