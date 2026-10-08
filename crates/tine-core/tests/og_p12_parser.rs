use tine_core::model::{page_title_from_preamble, Format};

#[test]
fn preamble_titles_exclude_literal_source() {
    assert_eq!(
        page_title_from_preamble("```\ntitle:: Ghost\n```\n\n- body\n", Format::Md),
        None
    );
    assert_eq!(
        page_title_from_preamble(
            "#+BEGIN_SRC\n#+TITLE: Ghost\n#+END_SRC\n\n* body\n",
            Format::Org
        ),
        None
    );
}

#[test]
fn reference_evidence_excludes_unlabeled_local_assets_and_keeps_nested_pages() {
    let asset = {
        let mut block = tine_core::doc::DocBlock::new("[[file:../assets/paper.pdf]]");
        block.set_org(true);
        block
    };
    assert!(asset.projection().reference_source().explicit.is_empty());
    let nested = tine_core::doc::DocBlock::new("[[Outer [[Inner]]]]");
    let names: Vec<_> = nested
        .projection()
        .reference_source()
        .explicit
        .iter()
        .map(|p| p.name.clone())
        .collect();
    assert_eq!(names, vec!["Inner", "Outer [[Inner]]"]);
}

#[test]
fn file_candidate_and_evidence_selection_are_named_policies() {
    use tine_core::block_regions::reference_target_name;
    assert_eq!(
        reference_target_name("file", "file:../pages/Ten___Child.org", "ten", true, true)
            .as_deref(),
        Some("Ten/Child")
    );
    assert_eq!(
        reference_target_name("file", "file:../pages/Ten___Child.org", "ten", true, false)
            .as_deref(),
        Some("ten")
    );
}

#[test]
fn empty_property_refill_preserves_accepted_syntax() {
    use tine_core::block_regions::{edit, parse, Edit};
    for tail in ["", " ", "\r\n"] {
        let raw = format!("Query\ntine.group-field::{tail}");
        let next = edit(
            &raw,
            false,
            Edit::Property {
                key: "tine.group-field".into(),
                value: Some("prop:status".into()),
            },
        )
        .unwrap();
        assert_eq!(
            parse(&next, false)
                .property("tine.group-field")
                .unwrap()
                .value,
            "prop:status"
        );
    }
}

#[test]
fn quick_switch_candidates_include_accepted_alias_and_tag_values() {
    let block = tine_core::doc::DocBlock::new("tags:: ProjectX， [[Linear IP]]\naliases:: LP Survey，Paper Notes\nstatus:: \"Private, Draft\"");
    let projection = block.projection();
    let names: Vec<_> = tine_core::reference_evidence::linkable_property_names(
        projection.reference_source(),
        &projection.regions,
    )
    .collect();
    assert_eq!(
        names,
        vec!["ProjectX", "Linear IP", "LP Survey", "Paper Notes"]
    );
    let quoted = tine_core::doc::DocBlock::new("alias:: \"[[Ghost]]\"");
    let projection = quoted.projection();
    assert_eq!(
        tine_core::reference_evidence::linkable_property_names(
            projection.reference_source(),
            &projection.regions
        )
        .count(),
        0
    );
}
