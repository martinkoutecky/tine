//! Guard graph Tauri commands as transport adapters.
use quote::ToTokens;
use std::path::{Path, PathBuf};
use syn::visit::{self, Visit};

// Device ingress and devtools own local OS data, not graph transport.
const DEVICE_COMMAND_EXEMPTIONS: &[(&str, &str)] = &[
    ("import_native_capture", "native device capture ingress"),
    ("tine_open_devtools", "developer tools window"),
    ("read_local_image", "caller-chosen device image read"),
    ("decode_asset_b64", "IPC payload decoding"),
];

#[derive(Default)]
struct BodyScan {
    violations: Vec<&'static str>,
    graph_calls: usize,
}

impl<'ast> Visit<'ast> for BodyScan {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*call.func {
            let segments: Vec<_> = path
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            if segments.first().is_some_and(|s| s == "tine_graph_features")
                || segments.as_slice() == ["asset_handoff_target"]
            {
                self.graph_calls += 1;
            }
            if segments.iter().any(|s| s.contains("retry")) {
                self.violations.push("retry");
            }
            if segments
                .as_slice()
                .starts_with(&["std".into(), "fs".into()])
                || segments.iter().any(|s| s == "Path" || s == "PathBuf")
            {
                self.violations.push("path or filesystem call");
            }
        }
        visit::visit_expr_call(self, call);
    }
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let method = call.method.to_string();
        let receiver = call.receiver.to_token_stream().to_string();
        if receiver == "slot . store" || receiver == "store" {
            self.graph_calls += 1;
        }
        if method == "join" {
            self.violations.push("path join");
        }
        if method.contains("retry") {
            self.violations.push("retry");
        }
        if method == "lock" || ((method == "read" || method == "write") && call.args.is_empty()) {
            self.violations.push("store-internal lock/read/write");
        }
        if ["contains", "starts_with", "ends_with", "find"].contains(&method.as_str()) {
            if receiver.contains("error")
                || receiver.contains("reason")
                || receiver.contains("message")
                || receiver.contains("to_string")
            {
                self.violations.push("error-text matching");
            }
        }
        visit::visit_expr_method_call(self, call);
    }
    fn visit_expr_loop(&mut self, node: &'ast syn::ExprLoop) {
        self.violations.push("loop");
        visit::visit_expr_loop(self, node);
    }
    fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
        self.violations.push("while");
        visit::visit_expr_while(self, node);
    }
    fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
        self.violations.push("for loop");
        visit::visit_expr_for_loop(self, node);
    }
}

fn scan(source: &str) -> Vec<String> {
    let file = syn::parse_file(source).unwrap();
    let mut violations = Vec::new();
    for item in file.items {
        let syn::Item::Fn(function) = item else {
            continue;
        };
        if !function.attrs.iter().any(|attr| {
            attr.path()
                .segments
                .last()
                .is_some_and(|seg| seg.ident == "command")
        }) {
            continue;
        }
        if !function
            .sig
            .inputs
            .to_token_stream()
            .to_string()
            .contains("GraphContext")
        {
            continue;
        }
        if DEVICE_COMMAND_EXEMPTIONS
            .iter()
            .any(|(name, _reason)| function.sig.ident == *name)
        {
            continue;
        }
        let mut scan = BodyScan::default();
        scan.visit_block(&function.block);
        if scan.graph_calls > 1 {
            scan.violations.push("more than one store/client call");
        }
        for issue in scan.violations {
            violations.push(format!("{}: {}", function.sig.ident, issue));
        }
    }
    violations
}

#[test]
fn graph_commands_are_thin_transport() {
    let source = include_str!("../../../src-tauri/src/commands.rs");
    let violations = scan(source);
    assert!(violations.is_empty(), "Graph Tauri commands may decode arguments, make one store/client call, and map the result only; no lock, retry, loop, path work, or error-text matching. Found: {violations:#?}");
}

#[test]
fn planted_graph_command_violation_is_detected() {
    let planted = r#"#[tauri::command]
        fn bad(state: GraphContext<'_>) {
            let _ = slot.block_search_lanes.lock().unwrap();
            while true { break; }
            let _ = error.to_string().contains("conflict");
        }"#;
    let violations = scan(planted);
    assert_eq!(
        violations.len(),
        3,
        "the planted lock, while, and error text must all fail"
    );
}

#[test]
fn second_store_call_is_detected() {
    let planted = r#"#[tauri::command]
        fn bad(state: GraphContext<'_>) {
            let _ = slot.store.page(&id);
            let _ = slot.store.search(&query, 10);
        }"#;
    assert!(scan(planted).contains(&"bad: more than one store/client call".into()));
    let clients = r#"#[tauri::command]
        fn bad(state: GraphContext<'_>) {
            let _ = tine_graph_features::pages::get_page(&slot.store, "A", kind);
            let _ = tine_graph_features::pages::get_page(&slot.store, "B", kind);
        }"#;
    assert!(scan(clients).contains(&"bad: more than one store/client call".into()));
}

#[test]
fn graph_command_path_and_filesystem_calls_are_detected() {
    let planted = r#"#[tauri::command]
        fn bad(state: GraphContext<'_>) {
            let _ = std::path::PathBuf::from("root").join("pages");
            let _ = std::fs::read("page.md");
        }"#;
    let violations = scan(planted);
    assert!(violations.contains(&"bad: path or filesystem call".into()));
    assert!(violations.contains(&"bad: path join".into()));
}

#[test]
fn graph_command_retry_and_for_loop_are_detected() {
    let planted = r#"#[tauri::command]
        fn bad(state: GraphContext<'_>) {
            retry_save();
            for _ in 0..2 { let _ = slot.store.page(&id); }
        }"#;
    let violations = scan(planted);
    assert!(violations.contains(&"bad: retry".into()));
    assert!(violations.contains(&"bad: for loop".into()));
}

#[derive(Default)]
struct ErrorTextScan(Vec<String>);

impl<'ast> Visit<'ast> for ErrorTextScan {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if !item.attrs.iter().any(|attr| {
            attr.path().is_ident("cfg") && attr.meta.to_token_stream().to_string().contains("test")
        }) {
            visit::visit_item_mod(self, item);
        }
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let method = call.method.to_string();
        let receiver = call.receiver.to_token_stream().to_string();
        if ["contains", "starts_with", "ends_with", "find"].contains(&method.as_str())
            && ["error", "reason", "message", "to_string"]
                .iter()
                .any(|word| receiver.contains(word))
        {
            self.0.push(format!("{receiver}.{method}"));
        }
        visit::visit_expr_method_call(self, call);
    }
}

fn feature_error_text_matches(source: &str) -> Vec<String> {
    let parsed = syn::parse_file(source).unwrap();
    let mut scan = ErrorTextScan::default();
    scan.visit_file(&parsed);
    scan.0
}

fn feature_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            feature_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            && !path.file_name().unwrap().to_string_lossy().contains("test")
        {
            out.push(path);
        }
    }
}

#[test]
fn graph_features_do_not_classify_error_text() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tine-graph-features/src");
    let mut files = Vec::new();
    feature_files(&root, &mut files);
    let violations: Vec<_> = files
        .into_iter()
        .flat_map(|path| {
            let source = std::fs::read_to_string(&path).unwrap();
            feature_error_text_matches(&source)
                .into_iter()
                .map(move |issue| format!("{}: {issue}", path.display()))
        })
        .collect();
    assert!(
        violations.is_empty(),
        "I-9: feature code must classify typed errors, not message text: {violations:#?}"
    );
}

#[test]
fn planted_feature_error_text_match_is_detected() {
    let source = r#"fn bad(error: StoreError) {
        if let StoreError::InvalidTarget(reason) = error {
            let _ = reason.starts_with("symlink:");
        }
    }"#;
    assert_eq!(feature_error_text_matches(source).len(), 1);
}
