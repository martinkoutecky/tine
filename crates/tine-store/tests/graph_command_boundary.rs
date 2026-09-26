//! Guard graph Tauri commands as transport adapters.
use quote::ToTokens;
use syn::visit::{self, Visit};

#[derive(Default)]
struct BodyScan {
    violations: Vec<&'static str>,
}

impl<'ast> Visit<'ast> for BodyScan {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let method = call.method.to_string();
        if method == "lock" || ((method == "read" || method == "write") && call.args.is_empty()) {
            self.violations.push("store-internal lock/read/write");
        }
        if ["contains", "starts_with", "ends_with", "find"].contains(&method.as_str()) {
            let receiver = call.receiver.to_token_stream().to_string();
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
        let mut scan = BodyScan::default();
        scan.visit_block(&function.block);
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
    assert!(violations.is_empty(), "Graph Tauri commands may decode arguments, make one store/client call, and map the result only; no store-internal lock/read/write, loop/while, or error-text matching. Imitate quick_switch. Found: {violations:#?}");
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
