use std::fs;
use std::path::Path;

fn call_arguments(source: &str, needle: &str) -> Vec<String> {
    let mut calls = Vec::new();
    let mut rest = source;
    while let Some(pos) = rest.find(needle) {
        rest = &rest[pos + needle.len()..];
        let mut depth = 1;
        let mut end = 0;
        for (index, ch) in rest.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                end = index;
                break;
            }
        }
        if depth != 0 {
            break;
        }
        calls.push(rest[..end].to_owned());
        rest = &rest[end + 1..];
    }
    calls
}

fn violations(file: &str, source: &str) -> Vec<String> {
    let mut found = Vec::new();
    for args in call_arguments(source, "diag(") {
        if !args.trim().is_empty()
            && !args.trim_start().starts_with('"')
            && !args.trim_start().starts_with("event:")
        {
            found.push(format!("{file}: diag requires a fixed literal event"));
        }
    }
    for sink in ["eprintln!(", "println!("] {
        for args in call_arguments(source, sink) {
            if [
                "{name",
                "{path",
                "{title",
                "{content",
                "{detail",
                "{args",
                "{payload",
                "e.rel_path",
            ]
            .iter()
            .any(|private| args.contains(private))
            {
                found.push(format!("{file}: private data in stderr/stdout"));
            }
        }
    }
    found
}

fn assert_clean(file: &str, source: &str) {
    let found = violations(file, source);
    assert!(found.is_empty(), "I-5: stderr diagnostics use fixed vocabulary; private names, paths and content go through debug-only diag_private; exemplar src-tauri/src/commands.rs open_asset. {found:?}");
}

#[test]
fn rust_log_sinks_keep_private_data_off_stderr() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for file in [
        "src-tauri/src/debug.rs",
        "src-tauri/src/commands.rs",
        "src-tauri/src/commands/concord.rs",
        "src-tauri/src/lib.rs",
        "src-tauri/src/backup.rs",
        "src-tauri/src/platform.rs",
        "src-tauri/src/linux_window_identity.rs",
        "crates/tine-store/src/model.rs",
        "crates/tine-graph-features/src/render.rs",
    ] {
        let source = fs::read_to_string(root.join(file)).unwrap();
        let production = if file == "crates/tine-store/src/model.rs" {
            source.split("mod tests {").next().unwrap()
        } else {
            &source
        };
        assert_clean(file, production);
    }
}

#[test]
fn planted_private_stderr_call_fails() {
    let fake = r#"diag(format!("page {name}")); eprintln!("path {path}");"#;
    assert!(
        std::panic::catch_unwind(|| assert_clean("planted.rs", fake)).is_err(),
        "I-5: planted private stderr sink must fail; exemplar src-tauri/src/commands.rs open_asset"
    );
}
