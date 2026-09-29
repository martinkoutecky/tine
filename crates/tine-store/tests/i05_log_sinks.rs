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

/// Remove `#[cfg(test)]` items (an inline `mod … { … }` block or a one-line
/// declaration) so only production text remains. Brace matching ignores
/// braces inside string literals only roughly; test code is the only input.
fn production_text(source: &str) -> String {
    let mut out = String::new();
    let mut rest = source;
    while let Some(pos) = rest.find("#[cfg(test)]") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        let item_end = match (after.find('{'), after.find(';')) {
            (Some(open), semi) if semi.is_none_or(|semi| open < semi) => {
                let mut depth = 0usize;
                let mut end = after.len();
                for (index, ch) in after[open..].char_indices() {
                    match ch {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                end = open + index + 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                end
            }
            (_, Some(semi)) => semi + 1,
            _ => after.len(),
        };
        rest = &after[item_end..];
    }
    out.push_str(rest);
    out
}

fn production_print_sites(dir: &Path, root: &Path, found: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            if name != "bin" && name != "tests" {
                production_print_sites(&path, root, found);
            }
            continue;
        }
        if !name.ends_with(".rs") || name.ends_with("_tests.rs") || name == "diag_line.rs" {
            continue;
        }
        let text = production_text(&fs::read_to_string(&path).unwrap());
        if text.contains("eprintln!(") || text.contains("println!(") {
            found.push(path.strip_prefix(root).unwrap().display().to_string());
        }
    }
}

/// GH #594: a crate line written with `eprintln!` reached no Windows reporter
/// and no `--debug` log. Production code in these crates writes through
/// `tine_core::diag_line::diagnostic_line`, which also reaches the log.
#[test]
fn production_crates_write_stderr_only_through_diagnostic_line() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut found = Vec::new();
    for krate in [
        "crates/tine-core/src",
        "crates/tine-store/src",
        "crates/tine-graph-features/src",
    ] {
        production_print_sites(&root.join(krate), &root, &mut found);
    }
    assert!(
        found.is_empty(),
        "GH #594/I-5: production stderr goes through tine_core::diag_line::diagnostic_line \
         (exemplar crates/tine-store/src/model/page_parse.rs); offenders: {found:?}"
    );
    let planted = production_text(
        "fn a() { eprintln!(\"x\"); }\n#[cfg(test)]\nmod t { fn b() { eprintln!(\"y\"); } }",
    );
    assert!(planted.contains("eprintln!(\"x\")") && !planted.contains("eprintln!(\"y\")"));
}
