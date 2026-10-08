//! I-15/I-12: parsers at preparation, indexed identity at move preflight.
#[test]
fn save_publication_reuses_only_verified_prepared_bytes() {
    let tx = include_str!("../src/transaction/publication.rs");
    assert!(tx.contains(".filter(|plan| plan.new.as_deref() == now.as_deref())"),
        "I-4/I-15: publish a prepared parse only for its exact final bytes; exemplar transaction/publication.rs post-apply sweep");
    let model = include_str!("../src/model/transaction_publish.rs");
    let publish = model
        .split("if let Some(saved) = saved {")
        .nth(1)
        .unwrap()
        .split("} else if own_rename")
        .next()
        .unwrap();
    assert!(
        publish.contains("self.cache_upsert(entry, saved.clone(), DiskObs::of(content))"),
        "I-15: publish the checked serialization parse; exemplar model/transaction_publish.rs verified saved branch"
    );
    assert!(!publish.contains("parse_doc(") && !publish.contains("parse_page_content("),
        "I-15: do not reparse a verified prepared page during publication; exemplar model/transaction_publish.rs verified saved branch");
    let layout = include_str!("../src/model/layout_retention.rs");
    assert!(layout.contains("let reparsed = super::parse_doc"),
        "I-4: retained layout must reparse the splice before publication; exemplar model/layout_retention.rs");
}

#[test]
fn move_twin_checks_use_the_published_name_answerer() {
    let tx = include_str!("../src/transaction.rs");
    let twin = tx
        .split("fn twin(")
        .nth(1)
        .unwrap()
        .split("fn disk_twin(")
        .next()
        .unwrap();
    assert!(twin.contains("self.store.move_claimant(&entry.name, entry.kind)"),
        "I-12/I-25: a move consults the published identity index; exemplar store/snapshot.rs move_claimant");
    let snapshot = include_str!("../src/store/snapshot.rs");
    assert!(
        snapshot.contains("name_claimants(&snapshot.claimants, name, kind)"),
        "I-12: move and WholeGraph resolve share name_claimants; exemplar store/snapshot.rs"
    );
}
