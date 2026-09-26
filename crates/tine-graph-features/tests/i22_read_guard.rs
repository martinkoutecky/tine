//! I-22 ratchet for reads that feed page, EDN, or HTML consumption.
use std::fs;
use std::path::PathBuf;

fn violations(file: &str, source: &str) -> Vec<String> {
    let mut failures = Vec::new();
    let production = if file.ends_with("model.rs") {
        source
            .split("#[cfg(test)]\nfn atomic_update_with_hooks")
            .next()
            .unwrap()
    } else {
        source
    };
    for (index, line) in production.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with("///") {
            continue;
        }
        if trimmed.contains("read_to_string(")
            && !(file.ends_with("store.rs") && trimmed.contains("if fs::read_to_string(&path)"))
        {
            failures.push(format!("{file}:{} uncapped read_to_string", index + 1));
        }
    }
    let without_comments = production
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("//") {
                ""
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut offset = 0;
    while let Some(found) = without_comments[offset..].find(".read(") {
        let start = offset + found + ".read(".len();
        let mut depth = 1usize;
        let mut end = start;
        for (relative, byte) in without_comments[start..].bytes().enumerate() {
            match byte {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                end = start + relative;
                break;
            }
        }
        if end > start && without_comments[start..end].trim_end().ends_with(", None") {
            let line = without_comments[..start]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                + 1;
            failures.push(format!("{file}:{line} read(_, None)"));
        }
        offset = if end > start { end + 1 } else { start };
    }
    failures
}

#[test]
fn parsed_and_rendered_inputs_use_the_shared_cap() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let files = [
        "crates/tine-store/src/model.rs",
        "crates/tine-store/src/watch.rs",
        "crates/tine-store/src/store.rs",
        "crates/tine-graph-features/src/print.rs",
        "crates/tine-graph-features/src/conflicts.rs",
        "crates/tine-graph-features/src/pdf.rs",
        "crates/tine-graph-features/src/pages.rs",
        "crates/tine-graph-features/src/config.rs",
        "crates/tine-graph-features/src/journals.rs",
    ];
    let mut found = Vec::new();
    for file in files {
        let source = fs::read_to_string(root.join(file)).unwrap();
        found.extend(violations(file, &source));
    }
    if std::env::var_os("TINE_I22_PLANT").is_some() {
        found.extend(violations(
            "crates/tine-graph-features/src/print.rs",
            "store.read(&id, None)",
        ));
    }
    assert!(found.is_empty(), "I-22: every read feeding parse/render must use the 64 MiB cap; exemplar print.rs:48 page_print_html. Violations: {found:?}");
}
