use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

fn constructions(file: &str, source: &str) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    let mut function = String::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(after) = trimmed
            .strip_prefix("fn ")
            .or_else(|| trimmed.strip_prefix("pub fn "))
            .or_else(|| trimmed.strip_prefix("pub(crate) fn "))
        {
            function = after.split('(').next().unwrap_or("").to_owned();
        }
        if trimmed.starts_with("//") || trimmed.starts_with('*') {
            continue;
        }
        let family = if file == "transaction.rs" {
            trimmed.split("Refusal::").nth(1).map(|rest| {
                rest.chars()
                    .take_while(|ch| ch.is_ascii_alphanumeric())
                    .collect::<String>()
            })
        } else {
            trimmed
                .split("SaveOutcome::")
                .nth(1)
                .map(|rest| {
                    rest.chars()
                        .take_while(|ch| ch.is_ascii_alphanumeric())
                        .collect::<String>()
                })
                .filter(|family| {
                    matches!(
                        family.as_str(),
                        "Closed"
                            | "Conflict"
                            | "Deleted"
                            | "ReadOnly"
                            | "Twin"
                            | "Repeated"
                            | "InvalidTarget"
                            | "GuideEphemeral"
                    )
                })
        };
        if let Some(family) = family {
            if !family.is_empty() {
                *counts
                    .entry(format!("{file}::{function}::{family}"))
                    .or_insert(0) += 1;
            }
        }
    }
    counts
}

fn contract_rows(source: &str) -> BTreeMap<String, usize> {
    source
        .lines()
        .filter_map(|line| {
            let mut cells = line.split('|').map(str::trim);
            let _ = cells.next()?;
            let key = cells.next()?.strip_prefix('`')?.strip_suffix('`')?;
            let count = cells.next()?.parse().ok()?;
            let scenario = cells.next()?;
            if scenario.is_empty() {
                return None;
            }
            Some((key.to_owned(), count))
        })
        .collect()
}

fn assert_table(actual: &BTreeMap<String, usize>, expected: &BTreeMap<String, usize>) {
    assert_eq!(actual, expected,
        "I-8: every production refusal needs a counted in-scope scenario in docs/storage-contract.md; exemplar transaction.rs::stage changed-revision conflict. New sites require a scenario row");
}

#[test]
fn all_production_refusals_have_scenarios() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let transaction =
        fs::read_to_string(root.join("crates/tine-store/src/transaction.rs")).unwrap();
    let store = fs::read_to_string(root.join("crates/tine-store/src/store.rs")).unwrap();
    let store = store
        .split("    pub fn save(")
        .nth(1)
        .unwrap()
        .split("    pub fn trash_stats(")
        .next()
        .unwrap();
    let store = format!("    pub fn save({store}");
    let mut actual = constructions("transaction.rs", &transaction);
    actual.extend(constructions("store.rs", &store));
    let mapping = fs::read_to_string(root.join("crates/tine-store/src/store.rs")).unwrap();
    let mapping = mapping
        .split("impl SaveOutcome {")
        .nth(1)
        .unwrap()
        .split("/// Result of one ordered page-save request.")
        .next()
        .unwrap();
    actual.extend(constructions("store.rs", mapping));
    let contract = fs::read_to_string(root.join("docs/storage-contract.md")).unwrap();
    assert_table(&actual, &contract_rows(&contract));
}

#[test]
fn planted_refusal_fails_table() {
    let mut actual = BTreeMap::new();
    actual.insert("transaction.rs::new_operation::Twin".to_owned(), 1);
    assert!(
        std::panic::catch_unwind(|| assert_table(&actual, &BTreeMap::new())).is_err(),
        "I-8: planted refusal must fail; exemplar transaction.rs::stage"
    );
}
