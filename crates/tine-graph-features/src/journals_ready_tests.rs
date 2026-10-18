use super::*;

#[test]
fn gf4_ready_feed_matches_legacy_days_bodies_and_cursors() {
    let root = tempfile::tempdir().unwrap();
    for dir in ["pages", "journals"] {
        std::fs::create_dir(root.path().join(dir)).unwrap();
    }
    let today = JournalDate::today();
    let fmt = JournalFormat::new(None, None);
    for (date, body) in [
        (today, ""),
        (JournalDate::from_ordinal(20000101), "- old body\n"),
        (JournalDate::from_ordinal(20990103), "- future body\n"),
    ] {
        std::fs::write(
            root.path()
                .join(format!("journals/{}.md", fmt.file_stem(date))),
            body,
        )
        .unwrap();
    }
    std::fs::write(
        root.path().join("pages/Plan.md"),
        "- [[2000-01-03]] [[2000-01-02]] [[2099-01-03]] [[2099-01-01]]\n",
    )
    .unwrap();
    let store = Store::open(root.path(), Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    let initial = feed_page(&store, 20, None).unwrap();
    assert!(
        initial.references_pending,
        "first request must leave graph additions to completion even when Ready"
    );
    assert_eq!(
        initial.pages.len(),
        2,
        "first request reads saved days only"
    );
    // Frozen pre-GF4 algorithm: blocking inventory, descending union,
    // direct body reads with virtual fallback, and first-page Upcoming.
    let legacy = |limit, before_day| {
        let as_of_day = today.ordinal_key();
        let (entries, unreadable) = feed_days_through(&store, Day(as_of_day)).unwrap();
        let referenced: std::collections::BTreeSet<_> = store
            .whole_graph()
            .unwrap()
            .inventory(tine_store::InventoryScope::ReferencedJournals)
            .0
            .iter()
            .filter_map(|e| e.day)
            .collect();
        let mut days: BTreeMap<_, _> = entries.into_iter().collect();
        for day in referenced.iter().filter(|d| d.0 <= as_of_day) {
            days.entry(*day).or_insert_with(|| store.journal_id(*day));
        }
        let by_id: BTreeMap<_, _> = days.iter().map(|(day, id)| (id.clone(), *day)).collect();
        let mut page = collect_feed_page(
            days.into_iter().rev().collect(),
            limit,
            before_day,
            as_of_day,
            |id| {
                Ok(match store.page(id) {
                    Ok(read) => Loaded::Page(feed_read(read)),
                    Err(StoreError::NotFound) if referenced.contains(&by_id[id]) => {
                        Loaded::Page(virtual_day(&store, by_id[id]))
                    }
                    Err(StoreError::NotFound) => Loaded::Gone,
                    Err(error) => Loaded::Unreadable(store_error(error).to_string()),
                })
            },
        )
        .unwrap();
        if before_day.is_none() {
            page.unreadable.splice(0..0, unreadable);
            page.upcoming = referenced
                .into_iter()
                .filter(|d| d.0 > as_of_day)
                .map(|d| virtual_day(&store, d))
                .collect();
        }
        page
    };
    let wire = |page: FeedPage<FeedRead>| {
        serde_json::json!({
            "pages": page.pages.into_iter().map(|p| (p.id.as_str().to_owned(), p.doc)).collect::<Vec<_>>(),
            "upcoming": page.upcoming.into_iter().map(|p| (p.id.as_str().to_owned(), p.doc)).collect::<Vec<_>>(),
            "cursor": page.next_before_day, "done": page.done, "as_of_day": page.as_of_day, "unreadable": page.unreadable,
        })
    };
    for limit in [0, 1, 3, 20] {
        for before in [
            None,
            Some(today.ordinal_key()),
            Some(20000103),
            Some(20000101),
        ] {
            let new = feed_page_through(&store, limit, before, None, true).unwrap();
            assert!(!new.references_pending);
            assert_eq!(wire(new), wire(legacy(limit, before)));
        }
    }
    let extended = feed_page_through(&store, 1, None, Some(20000101), true).unwrap();
    assert_eq!(extended.pages.len(), 4);
    assert!(extended.done);
    store.close();
}
