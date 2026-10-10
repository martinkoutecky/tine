//! A-K3: no call path takes `GraphSlot::host_slot()` while it already holds
//! it. `std::sync::RwLock` read guards are not reentrant: once a restore
//! waits for the write lock, a nested read waits for the restore, which waits
//! for the outer read. Every guard is therefore one statement's temporary,
//! `slot.host_slot().running()`, passed straight to a `tine_graph_features`
//! writer (a crate that cannot name the slot), and nothing else that
//! statement runs (closures, callbacks, chained calls) reaches a function
//! that takes the slot's host lock. Exemplar: `commands.rs` `merge_pages`.

use std::collections::BTreeSet;
use std::path::Path;

/// Production source of `src-tauri/src`: test files and trailing test
/// modules are left out.
fn production_files(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            production_files(&path, out);
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".rs") || name.ends_with("_tests.rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        let production = source
            .split("#[cfg(test)]\nmod tests")
            .next()
            .unwrap()
            .to_owned();
        out.push((path.display().to_string(), production));
    }
}

fn ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether `text` runs function `ident`: calls it, or passes it as a value
/// (a callback) to a call. A borrowed `&name` is a variable.
fn uses(text: &str, ident: &str) -> bool {
    text.match_indices(ident).any(|(at, _)| {
        let (before, after) = (&text[..at], &text[at + ident.len()..]);
        if before.chars().next_back().is_some_and(ident_char)
            || after.chars().next().is_some_and(ident_char)
        {
            return false;
        }
        let (before, after) = (before.trim_end(), after.trim_start());
        after.starts_with('(')
            || (before.ends_with(['(', ',', ':']) && after.starts_with([')', ',']))
    })
}

/// Every `fn name … { body }` in `source`.
fn functions(source: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for (at, _) in source.match_indices("fn ") {
        if source[..at].chars().next_back().is_some_and(ident_char) {
            continue;
        }
        let name: String = source[at + 3..]
            .chars()
            .take_while(|c| ident_char(*c))
            .collect();
        let Some(open) = source[at..].find(['{', ';']).map(|o| o + at) else {
            continue;
        };
        if name.is_empty() || source.as_bytes()[open] == b';' {
            continue;
        }
        let mut depth = 0i32;
        for (offset, c) in source[open..].char_indices() {
            depth += match c {
                '{' => 1,
                '}' => -1,
                _ => 0,
            };
            if depth == 0 {
                found.push((name, source[open..open + offset + 1].to_owned()));
                break;
            }
        }
    }
    found
}

/// Functions that take the slot's host lock, directly or through a callee
/// (by name; an over-approximation fails loudly, never silently).
fn acquirers(files: &[(String, String)]) -> BTreeSet<String> {
    let all: Vec<(String, String)> = files.iter().flat_map(|(_, s)| functions(s)).collect();
    let mut found: BTreeSet<String> = all
        .iter()
        .filter(|(_, body)| {
            body.contains("host_slot()")
                || body.contains(".host.read()")
                || body.contains(".host.write()")
        })
        .map(|(name, _)| name.clone())
        .collect();
    loop {
        let more: Vec<String> = all
            .iter()
            .filter(|(name, body)| !found.contains(name) && found.iter().any(|f| uses(body, f)))
            .map(|(name, _)| name.clone())
            .collect();
        if more.is_empty() {
            return found;
        }
        found.extend(more);
    }
}

/// Every `host_slot()` taken outside the one-statement shape, or under
/// which a function that takes the host lock may run.
fn violations(files: &[(String, String)], feature_module: &dyn Fn(&str) -> bool) -> Vec<String> {
    let acquirers = acquirers(files);
    let mut found = Vec::new();
    for (file, source) in files {
        for (at, _) in source.match_indices("host_slot()") {
            if source[..at].ends_with("fn ") {
                continue;
            }
            let line = source[..at].lines().count();
            let site = format!("{file}:{line}");
            let shaped = source[..at].ends_with("slot.")
                && source[at..].starts_with("host_slot().running()")
                && source[..at - "slot.".len()]
                    .trim_end()
                    .ends_with(['(', ',']);
            if !shaped {
                found.push(format!(
                    "{site}: not a `slot.host_slot().running()` call argument"
                ));
                continue;
            }
            // The innermost call the guard is an argument of.
            let mut depth = 0i32;
            let open = source[..at].char_indices().rev().find_map(|(i, c)| {
                depth += match c {
                    ')' | ']' | '}' => 1,
                    '(' | '[' | '{' => -1,
                    _ => 0,
                };
                (depth < 0).then_some(i)
            });
            let Some(open) = open.filter(|&i| source.as_bytes()[i] == b'(') else {
                found.push(format!("{site}: not inside a call"));
                continue;
            };
            let callee_start = source[..open]
                .char_indices()
                .rev()
                .find(|(_, c)| !(ident_char(*c) || *c == ':'))
                .map_or(0, |(i, c)| i + c.len_utf8());
            let callee = &source[callee_start..open];
            let segments: Vec<&str> = callee.split("::").collect();
            let module = segments.len().checked_sub(2).map(|i| segments[i]);
            if !module.is_some_and(feature_module)
                || !(segments[0] == "tine_graph_features" || Some(segments[0]) == module)
            {
                found.push(format!(
                    "{site}: the guard is passed to {callee:?}, not a tine_graph_features writer"
                ));
                continue;
            }
            // The statement the guard lives for: to its `;` or enclosing closer.
            let mut depth = 0i32;
            let end = source[open..]
                .char_indices()
                .find_map(|(i, c)| {
                    depth += match c {
                        '(' | '[' | '{' => 1,
                        ')' | ']' | '}' => -1,
                        _ => 0,
                    };
                    (depth < 0 || (depth == 0 && c == ';')).then_some(open + i)
                })
                .unwrap_or(source.len());
            let statement = &source[open..end];
            if statement.matches("host_slot()").count() != 1 {
                found.push(format!("{site}: the statement takes host_slot() twice"));
            }
            let rest = statement.replacen("slot.host_slot().running()", "", 1);
            for name in &acquirers {
                if uses(&rest, name) {
                    found.push(format!(
                        "{site}: {name} takes the host lock and runs under the guard"
                    ));
                }
            }
        }
    }
    found
}

fn feature_modules() -> impl Fn(&str) -> bool {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/tine-graph-features/src");
    move |module| dir.join(format!("{module}.rs")).is_file()
}

#[test]
fn no_call_path_reacquires_the_host_slot_while_holding_it() {
    let mut files = Vec::new();
    production_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let sites: usize = files
        .iter()
        .map(|(_, s)| s.matches("slot.host_slot()").count())
        .sum();
    assert!(sites >= 10, "the host_slot() scan found {sites} sites");
    let found = violations(&files, &feature_modules());
    assert!(
        found.is_empty(),
        "A-K3: no call path takes host_slot() while holding it (a nested std RwLock read \
         deadlocks once a restore waits for the write lock): {found:?}. Exemplar \
         src-tauri/src/commands.rs merge_pages: one statement-temporary \
         `slot.host_slot().running()` passed to a tine_graph_features writer"
    );
    // The writers' crate cannot name the slot, so cannot reacquire it.
    let features = Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/tine-graph-features/src");
    let mut feature_files = Vec::new();
    production_files(&features, &mut feature_files);
    for (file, source) in feature_files {
        assert!(
            !source.contains("host_slot") && !source.contains("GraphSlot"),
            "{file}"
        );
    }
}

#[test]
fn a_planted_reacquisition_fails_the_guard() {
    let planted = r#"
fn ok_writer(slot: &GraphSlot) {
    tine_graph_features::pages::merge_pages(&slot.store, slot.host_slot().running(), &a)
        .map_err(|error| error.to_string());
}
fn bound(slot: &GraphSlot) {
    let held = slot.host_slot();
}
fn twice(slot: &GraphSlot) {
    tine_graph_features::pages::merge_pages(&slot.store, slot.host_slot().running(), slot.host_slot().running());
}
fn nested(slot: &GraphSlot) {
    tine_graph_features::pages::merge_pages(&slot.store, slot.host_slot().running(), &a)
        .map_err(|_| report(slot));
}
fn report(slot: &GraphSlot) -> String {
    helper(slot)
}
fn helper(slot: &GraphSlot) -> String {
    let _ = slot.host.read();
    String::new()
}
fn callback(slot: &GraphSlot) {
    tine_graph_features::pages::merge_pages(&slot.store, slot.host_slot().running(), &a)
        .map_err(report);
}
fn local(slot: &GraphSlot) {
    own_writer(&slot.store, slot.host_slot().running());
}
"#;
    let files = vec![("planted.rs".to_owned(), planted.to_owned())];
    let found = violations(&files, &|module: &str| module == "pages");
    let lines: Vec<&str> = found
        .iter()
        .map(|v| v.split(": ").nth(1).unwrap())
        .collect();
    assert_eq!(
        lines,
        [
            "not a `slot.host_slot().running()` call argument",
            "the statement takes host_slot() twice",
            "the statement takes host_slot() twice",
            "report takes the host lock and runs under the guard",
            "report takes the host lock and runs under the guard",
            "the guard is passed to \"own_writer\", not a tine_graph_features writer",
        ],
        "{found:?}"
    );
}
