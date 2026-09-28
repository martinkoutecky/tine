use super::*;

#[test]
fn group_keys_and_membership_order_remain_distinct() {
    let view = ViewSettings {
        group_by: Some(super::super::ir::Field::new("tags")),
        aggregates: vec![(super::super::ir::Field::new("cost"), AggFn::Sum)],
        ..ViewSettings::default()
    };
    let mut fold = StatisticsFold::new(&view, 4096).unwrap().unwrap();
    fold.add(
        &[Some("1".into())],
        vec![Some("(none)".into()), Some("".into()), None],
    )
    .unwrap();
    fold.add(
        &[Some("2".into())],
        vec![Some("".into()), Some("tail".into())],
    )
    .unwrap();
    let answer = fold.finish();
    assert_eq!(answer.count, 2);
    assert_eq!(
        answer.overall,
        vec![Cell::Number {
            value: 3.0,
            skipped: 0
        }]
    );
    let groups = answer.groups.unwrap();
    assert_eq!(
        groups
            .iter()
            .map(|g| (g.key.as_deref(), g.count))
            .collect::<Vec<_>>(),
        vec![
            (Some("(none)"), 1),
            (Some(""), 2),
            (None, 1),
            (Some("tail"), 1)
        ]
    );
}

#[test]
fn advanced_transforms_do_not_request_statistics_or_implicit_board_groups() {
    let (mut query, _) = super::super::parse_query_text(
        "(task TODO)",
        super::super::QueryDialect::Og,
        crate::date::JournalDate::today(),
    );
    query.source = super::super::ir::Source::Advanced {
        original: "[:find ?b]".into(),
        og_options: String::new(),
    };
    let view = ViewSettings {
        view: Some(super::super::ir::ViewKind::Board),
        sample: Some(2),
        aggregates: vec![(super::super::ir::Field::new("cost"), AggFn::Sum)],
        ..ViewSettings::default()
    };
    let execution = super::super::view::statistics_execution_view(&query, &view);
    assert!(StatisticsFold::new(&execution, 0).unwrap().is_none());
    assert_eq!(execution.sample, view.sample);
    assert_eq!(view.group_by, None, "execution never authors the default");
    assert_eq!(view.aggregates.len(), 1);
}

#[test]
fn shared_arithmetic_fixtures_match_the_javascript_oracle() {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/query-statistics/semantics.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let view = ViewSettings {
            aggregates: vec![
                (super::super::ir::Field::new(""), AggFn::Count),
                (super::super::ir::Field::new("cost"), AggFn::Sum),
                (super::super::ir::Field::new("cost"), AggFn::Avg),
            ],
            ..ViewSettings::default()
        };
        let mut fold = StatisticsFold::new(&view, usize::MAX).unwrap().unwrap();
        for value in case["values"].as_array().unwrap() {
            let value = value.as_str().map(str::to_owned);
            fold.add(&[None, value.clone(), value], vec![]).unwrap();
        }
        let expected: Vec<Cell> = serde_json::from_value(case["cells"].clone()).unwrap();
        assert_eq!(fold.finish().overall, expected, "{}", case["name"]);
    }
}
