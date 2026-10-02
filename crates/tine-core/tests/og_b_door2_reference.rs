use tine_core::{doc::DocBlock, model::ReferenceKind, reference_evidence as evidence, Config};

#[test]
fn plain_reference_clients_keep_nfd_graphemes_whole() {
    let block = DocBlock::new("Cafe\u{301} and Café");
    let projection = block.projection().reference_source();
    let config = Config::default();
    let bare = vec!["cafe".into()];
    assert!(
        !evidence::has_occurrence_kind(
            block.raw(),
            projection,
            &bare,
            ReferenceKind::Plain,
            &config
        ),
        "I-4: membership must not stop before an NFD accent"
    );
    assert!(evidence::occurrences_of_kind_bounded(
        block.raw(),
        projection,
        "Cafe",
        &bare,
        ReferenceKind::Plain,
        &config
    )
    .occurrences
    .is_empty());
    let names = vec!["café".into()];
    assert!(evidence::has_occurrence_kind(
        block.raw(),
        projection,
        &names,
        ReferenceKind::Plain,
        &config
    ));
    let found = evidence::occurrences_of_kind(
        block.raw(),
        projection,
        "Café",
        &names,
        ReferenceKind::Plain,
        &config,
    );
    assert_eq!(found.len(), 2);
    assert_eq!((found[0].span.start, found[0].span.end), (0, 5));
}
