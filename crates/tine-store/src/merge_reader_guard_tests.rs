//! Inspect Rust calls/imports, never Markdown/Org syntax, at the writer boundary.
use std::path::{Path, PathBuf};
use syn::visit::Visit;

// Reader entries allowed at this boundary, with their complete purpose.
const READER_ENTRIES: &[(&str, &str)] = &[
    (
        "rename_described_proposal",
        "input rename and coordinates share one reader pass",
    ),
    ("read", "NativeDocument reads only the completed output"),
    (
        "projection",
        "reference evidence of the output leftover, never an input",
    ),
    (
        "has_directive",
        "output reader checks whether O6 quote framing escaped",
    ),
    (
        "line_terminator",
        "only the assembled writer text's insertion boundary",
    ),
    (
        "normalize_line_endings",
        "canonicalize an intended payload, without recognizing structure",
    ),
    (
        "parse",
        "journal identity through the shared date reader, never Markdown/Org",
    ),
];

#[derive(Default)]
struct Calls {
    errors: Vec<String>,
    function: String,
}

impl<'ast> Visit<'ast> for Calls {
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if !node.attrs.iter().any(|a| a.path().is_ident("cfg")) {
            syn::visit::visit_item_mod(self, node);
        }
    }
    fn visit_use_name(&mut self, node: &'ast syn::UseName) {
        self.check_import(&node.ident.to_string());
    }
    fn visit_use_rename(&mut self, node: &'ast syn::UseRename) {
        self.check_import(&node.ident.to_string());
    }
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        let previous = std::mem::replace(&mut self.function, node.sig.ident.to_string());
        syn::visit::visit_item_fn(self, node);
        self.function = previous;
    }
    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        let previous = std::mem::replace(&mut self.function, node.sig.ident.to_string());
        syn::visit::visit_impl_item_fn(self, node);
        self.function = previous;
    }
    fn visit_path(&mut self, path: &'ast syn::Path) {
        for segment in &path.segments {
            let name = segment.ident.to_string();
            if name.starts_with("parse")
                || [
                    "physical_lines",
                    "physical_outline",
                    "continuation_raw",
                    "alias_key",
                    "alias_syntax",
                    "blank",
                    "layout",
                    "page_header_text",
                    "pre_block_projections",
                    "preamble_range_in",
                    "page_title_from_header",
                    "is_alias_key",
                ]
                .contains(&name.as_str())
            {
                // Local layout consumes reader-owned coordinates; it is not an
                // imported recognizer. Path imports/calls to core layout fail.
                if name != "layout" || path.segments.len() > 1 {
                    self.errors
                        .push(format!("{}: forbidden reader path {name}", self.function));
                }
            }
        }
        syn::visit::visit_path(self, path);
    }
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*node.func {
            let name = path.path.segments.last().unwrap().ident.to_string();
            if name == "read" {
                assert!(READER_ENTRIES.iter().any(|(entry, _)| *entry == name));
                let arg = node.args.first().unwrap();
                let tokens = quote::quote!(#arg).to_string();
                if tokens != "& w . text" {
                    self.errors.push(format!("input read: {tokens}"));
                }
            }
            if name == "line_terminator" {
                let arg = node.args.first().unwrap();
                if quote::quote!(#arg).to_string() != "& self . text" {
                    self.errors.push("input line terminator".into());
                }
            }
        }
        syn::visit::visit_expr_call(self, node);
    }
    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let name = node.method.to_string();
        if name.starts_with("parse")
            || [
                "split",
                "split_inclusive",
                "split_terminator",
                "lines",
                "bytes",
                "chars",
                "trim",
                "trim_start",
                "trim_end",
                "trim_matches",
                "trim_end_matches",
                "match_indices",
                "starts_with",
                "ends_with",
                "projection",
                "has_directive",
                "is_whitespace",
                "is_ascii_whitespace",
            ]
            .contains(&name.as_str())
        {
            let receiver = &node.receiver;
            let receiver = quote::quote!(#receiver).to_string();
            let allowed = match name.as_str() {
                // CR/LF seam detection certifies assembled output framing.
                "ends_with" | "starts_with" => self.function == "place",
                // Only the placed output leftover is projected.
                "projection" => self.function == "check" && receiver == "actual",
                "has_directive" => self.function == "check" && receiver == "read",
                "parse" => receiver == "days",
                _ => false,
            };
            if !allowed {
                self.errors
                    .push(format!("{}: text method {receiver}.{name}", self.function));
            }
        }
        syn::visit::visit_expr_method_call(self, node);
    }
}

impl Calls {
    fn check_import(&mut self, name: &str) {
        if name.starts_with("parse")
            || [
                "physical_lines",
                "physical_outline",
                "blank",
                "layout",
                "alias_key",
                "alias_syntax",
            ]
            .contains(&name)
        {
            self.errors.push(format!("forbidden reader import {name}"));
        }
    }
}

fn modules(path: &Path, files: &mut Vec<PathBuf>) {
    files.push(path.to_owned());
    let ast = syn::parse_file(&std::fs::read_to_string(path).unwrap()).unwrap();
    for item in ast.items {
        if let syn::Item::Mod(m) = item {
            if m.attrs.iter().any(|a| a.path().is_ident("cfg")) {
                continue;
            }
            if m.content.is_none() {
                let base = if path.file_stem().unwrap() == "merge_payload" {
                    path.with_extension("")
                } else {
                    path.parent().unwrap().to_owned()
                };
                modules(&base.join(format!("{}.rs", m.ident)), files);
            }
        }
    }
}

#[test]
fn merge_writer_uses_reader_coordinates_without_reading_inputs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tine-graph-features/src/pages/merge_payload.rs");
    let mut files = Vec::new();
    modules(&root, &mut files);
    assert!(files.len() >= 2, "guard must follow production modules");
    let mut calls = Calls::default();
    for path in files {
        let ast = syn::parse_file(&std::fs::read_to_string(path).unwrap()).unwrap();
        calls.visit_file(&ast);
    }
    assert!(calls.errors.is_empty(), "{}", calls.errors.join("\n"));
}

#[test]
fn guard_rejects_aliased_readers_and_input_scans() {
    for source in [
        "use tine_core::doc::parse as innocent;",
        "use tine_core::doc::physical_lines as innocent;",
        "fn f(input: &str) { input.trim(); }",
        "fn f(input: &str) { Header::read(input, false); }",
    ] {
        let mut calls = Calls::default();
        calls.visit_file(&syn::parse_file(source).unwrap());
        assert!(!calls.errors.is_empty(), "guard missed {source}");
    }
}
