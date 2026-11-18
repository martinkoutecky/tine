//! The merge writer places reader-owned coordinates; it does not read its
//! inputs again. This guard checks the writer's production modules
//! syntactically, by an allow-list: every method, called path and macro the
//! writer uses is named below with its purpose, and some only on a fixed
//! receiver or argument. A new operation fails until it is listed here.
//!
//! What it does not see, stated so the claim stays no larger than the check:
//! it has no types, so it cannot tell a method on input text from the same name
//! on another value (hence the receiver pins); it does not look inside macro
//! bodies, so every macro must be on the allow-list and none may be defined;
//! and it does not check operators or indexing (`==` on a slice). It rejects
//! every cfg-attributed production module except `cfg(test)` modules, which
//! it does not read, and byte literals.
use std::path::{Path, PathBuf};
use syn::visit::Visit;

/// Methods the writer may call: name, receiver pin (the receiver's tokens
/// must end with it), purpose.
const METHODS: &[(&str, Option<&str>, &str)] = &[
    // Collections, options, iterators and the writer's own state.
    (
        "add_input",
        None,
        "partition an input at the description's cuts",
    ),
    ("and_then", None, "option"),
    ("any", None, "iterator"),
    ("at", None, "error provenance"),
    ("sum", None, "test counter"),
    ("as_ref", None, "option"),
    ("as_slice", None, "vector"),
    ("chain", None, "iterator"),
    ("check", None, "trace certificate and acceptance"),
    ("check_header", None, "acceptance"),
    ("clear", None, "scratch buffer"),
    ("clone", None, "value"),
    ("collect", None, "iterator"),
    ("consume", None, "writer: drop pieces"),
    ("contains", None, "set or range membership"),
    ("contains_key", None, "map"),
    ("copied", None, "iterator"),
    ("decoded", None, "writer: member name as written"),
    ("each_ref", None, "array"),
    ("emit", None, "acceptance: record emitted header intent"),
    ("entry", None, "map"),
    ("enumerate", None, "iterator"),
    ("expect", None, "option"),
    ("extend", None, "collection"),
    ("field", None, "writer: united alias declaration"),
    ("filter", None, "iterator or option"),
    ("filter_map", None, "iterator"),
    ("find", None, "iterator"),
    ("find_map", None, "iterator"),
    ("flat_map", None, "iterator"),
    ("flatten", None, "iterator"),
    ("fmt", None, "Display"),
    ("get", None, "collection"),
    ("ids", None, "writer: place pieces"),
    ("insert", None, "writer insert or set"),
    ("interval", None, "trace pieces of a range"),
    ("into", None, "conversion"),
    ("into_iter", None, "iterator"),
    ("is_empty", None, "collection or range"),
    ("is_none", None, "option"),
    ("is_none_or", None, "option"),
    ("is_some", None, "option"),
    ("is_some_and", None, "option"),
    ("iter", None, "collection"),
    ("iter_mut", None, "collection"),
    ("last", None, "collection"),
    ("layout", None, "writer: drop layout pieces"),
    ("len", None, "collection or range"),
    ("map", None, "iterator or option"),
    ("map_err", None, "result"),
    ("map_or", None, "option"),
    ("max", None, "number"),
    ("min", None, "number"),
    ("newline", None, "writer: terminate the output line"),
    ("next", None, "iterator"),
    ("or_default", None, "map entry"),
    ("or_insert", None, "map entry"),
    ("original", None, "writer: place original pieces"),
    ("payload", None, "writer: pieces' current text"),
    ("peek", None, "iterator"),
    ("peekable", None, "iterator"),
    ("position", None, "iterator"),
    ("prose", None, "writer: promote the reader's prose range"),
    ("push", None, "collection or output buffer"),
    ("push_str", None, "output buffer"),
    ("rposition", None, "iterator"),
    ("set", None, "test counter"),
    ("take_while", None, "iterator"),
    ("to_owned", None, "value"),
    ("unwrap", None, "option"),
    ("unwrap_or", None, "option"),
    ("unwrap_or_else", None, "option"),
    ("with", None, "test counter"),
    ("write_str", None, "Display"),
    ("zip", None, "iterator"),
    // The reader's source map: lookups, never a scan of text.
    ("block", Some("map"), "source map: a block's lines"),
    ("block_start", Some("map"), "source map"),
    (
        "end",
        Some("map"),
        "source map: line end before its terminator",
    ),
    ("indent", Some("map"), "source map"),
    ("line", Some("map"), "source map"),
    ("line_at", Some("map"), "source map"),
    ("lines", Some("map"), "source map: physical line indices"),
    ("read_end", Some("map"), "source map"),
    ("removed", Some("map"), "source map"),
    // Output and identity checks, each on its one receiver.
    ("raw", None, "output and survivor reader trees"),
    (
        "projection",
        Some("actual"),
        "reference evidence of the output leftover",
    ),
    (
        "reference_source",
        None,
        "reference evidence of the output leftover",
    ),
    (
        "parse",
        Some("days"),
        "journal identity through the shared date reader",
    ),
    ("ordinal_key", None, "journal identity"),
    (
        "ends_with",
        Some("text"),
        "CR/LF seam of the assembled output",
    ),
    (
        "starts_with",
        Some("bytes"),
        "CR/LF seam of the placed bytes",
    ),
];

/// Called paths (last segment): name, required first-argument tokens, purpose.
const CALLS: &[(&str, Option<&str>, &str)] = &[
    (
        "rename_described_proposal",
        None,
        "input rename and description share one reader pass",
    ),
    (
        "read",
        Some("& w . text"),
        "NativeDocument reads only the completed output",
    ),
    (
        "line_terminator",
        Some("& self . text"),
        "the assembled text's insertion boundary",
    ),
    (
        "normalize_line_endings",
        Some("raw"),
        "canonicalize an intended payload",
    ),
    (
        "parse_text_bounded",
        Some("root . raw ()"),
        "output reader: did O6 quote framing escape",
    ),
    (
        "alias_nonsyntax",
        None,
        "the certificate's protected alias fragments",
    ),
    (
        "property_key_norm",
        Some("to"),
        "O6 key of a rewritten key site",
    ),
    ("page_key", None, "page identity of a reference name"),
    ("same_page", None, "survivor identity"),
    ("reading", None, "writer: member identity"),
    ("place", None, "writer: append bytes"),
    ("members", None, "writer"),
    ("join_end", None, "writer"),
    ("body_prefixes", None, "writer"),
    (
        "size",
        None,
        "writer: preorder block count of a reader subtree",
    ),
    ("cuts", None, "writer"),
    ("append", None, "acceptance"),
    ("prose", None, "acceptance"),
    ("tree", None, "acceptance"),
    // Local closures, whose bodies the guard checks where they are written.
    ("refused", None, "closure"),
    ("keep", None, "closure"),
    ("emitted", None, "closure"),
    ("entries", None, "closure"),
    ("source_drawer", None, "closure"),
    ("input", None, "closure"),
    ("target_property_origin", None, "closure"),
    ("directive", None, "closure"),
    ("default", None, "value"),
    ("new", None, "value"),
    ("from", None, "conversion"),
    ("replace", None, "std::mem::replace"),
    ("Some", None, "option"),
    ("Ok", None, "result"),
    ("Err", None, "result"),
    ("Trace", None, "error variant"),
    ("MissingNativeNameRange", None, "error variant"),
    ("Original", None, "placement variant"),
    ("Insert", None, "placement variant"),
    ("Target", None, "origin variant"),
    ("Source", None, "origin variant"),
];

const MACROS: &[&str] = &["format", "matches", "thread_local", "vec", "write"];

#[derive(Default)]
struct Calls {
    errors: Vec<String>,
    function: String,
}

impl<'ast> Visit<'ast> for Calls {
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if !test_only(&node.attrs) {
            if node.attrs.iter().any(|a| a.path().is_ident("cfg")) {
                self.errors
                    .push(format!("cfg-attributed production module {}", node.ident));
            }
            syn::visit::visit_item_mod(self, node);
        }
    }
    /// Only a type may be imported under another name (`as Header`): a
    /// renamed function would bypass the call allow-list.
    fn visit_use_rename(&mut self, node: &'ast syn::UseRename) {
        let types = [&node.ident, &node.rename]
            .iter()
            .all(|i| i.to_string().starts_with(|c: char| c.is_ascii_uppercase()));
        if !types || CALLS.iter().any(|(n, _, _)| node.rename == n) {
            self.errors.push(format!("renamed import {}", node.ident));
        }
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
    fn visit_item_macro(&mut self, node: &'ast syn::ItemMacro) {
        if node.ident.is_some() {
            self.errors
                .push(format!("macro definition {:?}", node.ident));
        }
        self.visit_macro(&node.mac);
    }
    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        let name = node.path.segments.last().unwrap().ident.to_string();
        if !MACROS.contains(&name.as_str()) {
            self.errors
                .push(format!("{}: macro {name}! is not allowed", self.function));
        }
    }
    fn visit_lit(&mut self, node: &'ast syn::Lit) {
        if matches!(node, syn::Lit::Byte(_) | syn::Lit::ByteStr(_)) {
            self.errors.push(format!("{}: byte literal", self.function));
        }
    }
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*node.func {
            let name = path.path.segments.last().unwrap().ident.to_string();
            match CALLS.iter().find(|(n, _, _)| *n == name) {
                None => self
                    .errors
                    .push(format!("{}: call {name} is not allowed", self.function)),
                Some((_, Some(arg), _)) => {
                    let first = node.args.first().map(|a| quote::quote!(#a).to_string());
                    if first.as_deref() != Some(*arg) {
                        self.errors
                            .push(format!("{}: {name} on {first:?}", self.function));
                    }
                }
                Some(_) => {}
            }
        }
        syn::visit::visit_expr_call(self, node);
    }
    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let name = node.method.to_string();
        let receiver = &node.receiver;
        let receiver = quote::quote!(#receiver).to_string();
        let allowed = METHODS.iter().any(|(n, pin, _)| {
            *n == name && pin.is_none_or(|pin| receiver.split(' ').next_back() == Some(pin))
        });
        if !allowed {
            self.errors
                .push(format!("{}: method {receiver}.{name}", self.function));
        }
        syn::visit::visit_expr_method_call(self, node);
    }
}

fn test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && a.parse_args::<syn::Ident>()
                .is_ok_and(|ident| ident == "test")
    })
}

fn modules(path: &Path, files: &mut Vec<PathBuf>) {
    files.push(path.to_owned());
    let ast = syn::parse_file(&std::fs::read_to_string(path).unwrap()).unwrap();
    for item in ast.items {
        if let syn::Item::Mod(m) = item {
            if !test_only(&m.attrs) && m.content.is_none() {
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
        "use tine_core::doc::parse as innocent; fn f(s: &str) { innocent(s); }",
        "use tine_core::doc::physical_lines as place;",
        "use tine_core::doc::Parser as Source;",
        "fn f(s: &str) { tine_core::doc::physical_lines(s); }",
        "fn f(input: &str) { input.trim(); }",
        "fn f(input: &str) { Header::read(input, false); }",
        "fn f(input: &str) { tine_core::doc::line_terminator(input); }",
        "fn f(input: &str, map: &M) { input.lines(); }",
        // The review's three evasions of the previous deny-list guard.
        "#[cfg(any(unix, windows))]\nmod input_reader { fn f(s: &str) { s.lines(); } }",
        "fn f(input: &str) { for byte in input.as_bytes() { if *byte == b'-' { break; } } }",
        "macro_rules! rescan { ($s:expr) => { $s.lines().next() }; }\nfn f(input: &str) { let _ = rescan!(input); }",
    ] {
        let mut calls = Calls::default();
        calls.visit_file(&syn::parse_file(source).unwrap());
        assert!(!calls.errors.is_empty(), "guard missed {source}");
    }
}
