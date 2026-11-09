use super::ir::{ExecutionContext, Field, SortDir};
use super::registry::Registry;
use super::{parse_query_input, resolve_for_execution, QueryInput};
use crate::date::JournalDate;

const FIRST: &str = r#"(fn [result] (sort-by (juxt (fn [h] (get h :block/priority "Z")) (fn [h] (get h :block/content))) result))"#;
const SECOND: &str = r#"(fn [result] (sort-by (juxt (fn [h] (get h :block/priority "Z"))
                            (fn [h] (get h :block/scheduled (get h :block/deadline)))
                            (fn [h] (get h :block/content))) result))"#;

#[test]
fn gh852_first_literal_reporter_transform_is_recognised_whole() {
    recognised(FIRST, &["priority", "content"]);
}

#[test]
fn gh852_second_literal_reporter_transform_is_recognised_whole() {
    recognised(SECOND, &["priority", "scheduled", "content"]);
}

fn recognised(transform: &str, fields: &[&str]) {
    let today = JournalDate {
        year: 2026,
        month: 11,
        day: 5,
    };
    let src = format!(
        r#"{{:query [:find (pull ?h [*]) :where [?h :block/marker "TODO"]] :result-transform {transform}}}"#
    );
    let (query, view) = parse_query_input(&src, QueryInput::Advanced, today, Registry::none());
    let resolved = resolve_for_execution(&query, &ExecutionContext::none(), today);
    assert!(
        resolved.report().supported,
        "{transform}: {:?}",
        resolved.report()
    );
    assert_eq!(
        view.sort,
        fields
            .iter()
            .map(|f| (Field::new(*f), SortDir::Asc))
            .collect::<Vec<_>>()
    );
}

#[test]
fn gh852_single_keys_defaults_names_and_whitespace_use_the_same_recogniser() {
    recognised(
        "(fn [%] (sort-by (fn [?row!] (get ?row! :block/content)) %))",
        &["content"],
    );
    for (key, default) in [
        ("priority", "\"A\""),
        ("scheduled", "0"),
        ("deadline", "20991231"),
        ("content", "\"fallback\\ntext\""),
    ] {
        recognised(&format!("(fn [rows] ; outer name\n (sort-by (fn [item] (get item :block/{key} {default})) rows))"), &[key]);
        recognised(
            &format!("(fn [same] (sort-by (fn [same] (get same :block/{key})) same))"),
            &[key],
        );
        recognised(
            &format!("(fn [rows] (sort-by (juxt (fn [a] (get a :block/{key} nil))) rows))"),
            &[key],
        );
    }
}

#[test]
fn gh852_one_unknown_key_or_default_refuses_the_whole_query() {
    let today = JournalDate {
        year: 2026,
        month: 11,
        day: 5,
    };
    for key in [
        "(fn [h] (get h :block/unknown))",
        "(fn [h] (get h :block/scheduled (get other :block/deadline)))",
        "(fn [h] (get h :block/scheduled (get h :block/unknown)))",
        "(fn [h] (get h :block/scheduled (get h :block/content)))",
        "(fn [h] (get h :block/content (str h)))",
        "(fn [h] (get h :block/scheduled \"Z\"))",
        "(fn [h] (get h :block/scheduled 9007199254740992))",
        "(fn [h] (get other :block/content))",
        "(fn [get] (get get :block/content))",
        "(fn [&] (get & :block/content))",
        "#(get % :block/content)",
    ] {
        let src = format!(
            r#"{{:query [:find (pull ?h [*]) :where [?h :block/marker "TODO"]] :result-transform (fn [r] (sort-by (juxt (fn [h] (get h :block/priority "Z")) {key}) r))}}"#
        );
        let (query, view) = parse_query_input(&src, QueryInput::Advanced, today, Registry::none());
        let resolved = resolve_for_execution(&query, &ExecutionContext::none(), today);
        assert!(!resolved.report().supported, "{key}");
        assert_eq!(resolved.report().ignored, ["result-transform"]);
        assert!(view.sort.is_empty(), "no partially recognised sort: {key}");
    }
}

#[test]
fn gh852_keys_match_og_nil_numeric_raw_utf16_and_stable_order() {
    use super::advanced_sort::{advanced_result_sort, AdvancedSortRow};
    use super::sort::compare_sort_decorations;
    let sort_rows = |transform: &str, rows: &[AdvancedSortRow<'_>]| {
        let src = format!("{{:result-transform {transform}}}");
        let sort = advanced_result_sort(&src).unwrap();
        let mut decorated: Vec<_> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| (sort.decorate(row), index))
            .collect();
        decorated.sort_by(|a, b| compare_sort_decorations(&a.0, &b.0, &vec![true; a.0.len()]));
        decorated
            .into_iter()
            .map(|(_, index)| index)
            .collect::<Vec<_>>()
    };
    let row = |priority, scheduled, deadline, content| AdvancedSortRow {
        priority,
        scheduled,
        deadline,
        content,
    };
    let rows = [
        row(Some("B"), None, None, Some("TODO [#B] task")),
        row(None, None, None, Some("z no priority")),
        row(Some("A"), None, None, Some("TODO [#A] same\nprop:: z")),
        row(Some("A"), None, None, Some("TODO [#A] same\nprop:: a")),
        row(Some("A"), None, None, Some("TODO [#A] same\nprop:: a")),
    ];
    assert_eq!(
        sort_rows(FIRST, &rows),
        [3, 4, 2, 0, 1],
        "raw properties break ties; equal keys stay stable"
    );
    let rows = [
        row(Some("A"), Some(20261107), Some(20261101), Some("z")),
        row(Some("A"), None, Some(20261106), Some("a")),
        row(Some("A"), None, None, Some("missing")),
        row(Some("A"), Some(20261107), None, Some("b")),
    ];
    assert_eq!(
        sort_rows(SECOND, &rows),
        [2, 1, 3, 0],
        "nil first; scheduled wins over an earlier deadline; content breaks date ties"
    );
    let rows = [
        row(None, None, None, Some("\u{e000}")),
        row(None, None, None, Some("😀")),
        row(None, None, None, Some("a")),
        row(None, None, None, Some("Z")),
        row(None, None, None, Some("")),
        row(None, None, None, None),
    ];
    let content = "(fn [r] (sort-by (fn [h] (get h :block/content)) r))";
    assert_eq!(
        sort_rows(content, &rows),
        [5, 4, 3, 2, 1, 0],
        "nil, empty, uppercase, lowercase, surrogate pair, BMP private use"
    );
    let rows = [
        row(Some("A"), Some(10), None, Some("x")),
        row(None, Some(2), None, Some("x")),
        row(Some("B"), None, None, Some("x")),
    ];
    assert_eq!(
        sort_rows(
            "(fn [r] (sort-by (fn [h] (get h :block/scheduled 0)) r))",
            &rows
        ),
        [2, 1, 0],
        "numeric, not lexical dates/defaults"
    );
    assert_eq!(
        sort_rows(
            "(fn [r] (sort-by (fn [h] (get h :block/priority)) r))",
            &rows
        ),
        [1, 0, 2],
        "omitted priority default is nil, not Z"
    );
}
