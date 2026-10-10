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
            .or_else(|| trimmed.strip_prefix("pub(super) fn "))
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
                            | "UnreadableOwner"
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
    // The whole `transaction` module: a refusal moved into a `transaction/` child
    // file must stay counted, or splitting the file would silently empty the table.
    let mut transaction =
        fs::read_to_string(root.join("crates/tine-store/src/transaction.rs")).unwrap();
    let mut children: Vec<_> = fs::read_dir(root.join("crates/tine-store/src/transaction"))
        .map(|dir| dir.map(|entry| entry.unwrap().path()).collect())
        .unwrap_or_default();
    children.sort();
    for child in children
        .iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
    {
        transaction.push('\n');
        transaction.push_str(&fs::read_to_string(child).unwrap());
    }
    // `store/save_failure.rs` (`Store::save`) is a test oracle since step 3b
    // (`#[cfg(any(test, feature = "test-faults"))]`): production saves are
    // the page host's (STEP3 §12).
    let mut actual = constructions("transaction.rs", &transaction);
    let mapping = fs::read_to_string(root.join("crates/tine-store/src/store.rs")).unwrap();
    // The end marker must exist: a missing one would sweep the rest of the
    // file, tests included, into the production table.
    let (mapping, _) = mapping
        .split("impl SaveOutcome {")
        .nth(1)
        .unwrap()
        .split_once("/// Parsed page and the raw-byte revision used for a guarded save.")
        .expect("the item after `impl SaveOutcome` ends the mapping");
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
