use tine_core::model::PageKind;
use tine_store::{PageId, Resolved, Store};

/// CM-DAYFILE and CM-ORDER (D-20261111-CMR14, D-20261110-CM): a pages file
/// whose effective name is a date is that day's journal where it lies, and a
/// journals-directory file of the same day wins over it, cold and ready
/// alike. (Before CM step 2 the literal pages file opened ahead of the
/// journal; that rule is superseded by those decisions.)
#[test]
fn a_date_named_pages_file_is_the_day_unless_a_journal_holds_it_in_cold_and_ready_reads() {
    let _guard = super::integration_lock();
    for (name, filename, preamble) in [
        ("Oct 26th, 2026", "Oct 26th, 2026.md", ""),
        ("Oct 26th, 2026", "imported.md", "title:: Oct 26th, 2026\n"),
        ("2026-10-26", "2026-10-26.md", ""),
    ] {
        for journal in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            std::fs::create_dir(temp.path().join("pages")).unwrap();
            std::fs::create_dir(temp.path().join("journals")).unwrap();
            std::fs::write(
                temp.path().join("pages/Owner.md"),
                "alias:: [[Oct 27th, 2026]]\n- alias only\n",
            )
            .unwrap();
            std::fs::write(
                temp.path().join("pages").join(filename),
                format!("{preamble}- imported content\n"),
            )
            .unwrap();
            if journal {
                std::fs::write(
                    temp.path().join("journals/2026_10_26.md"),
                    "- journal content\n",
                )
                .unwrap();
            }
            let pause = temp.path().join(".tine-test-pause-load");
            std::fs::write(&pause, "").unwrap();
            let store = Store::open(temp.path(), Default::default()).unwrap().0;
            let expected = if journal {
                "journals/2026_10_26.md".to_owned()
            } else {
                format!("pages/{filename}")
            };
            let early = store.page_named(name, PageKind::Journal);
            std::fs::remove_file(pause).unwrap();
            assert_eq!(
                early
                    .unwrap()
                    .unwrap_or_else(|| panic!("cold {name} in {filename}, journal {journal}"))
                    .id,
                PageId::from(expected.clone())
            );
            let view = store.whole_graph().unwrap();
            assert!(
                matches!(view.resolve(name, true), Resolved::Existing { id, .. } if id == PageId::from(expected.clone()))
            );
            let read = store.page_named(name, PageKind::Journal).unwrap().unwrap();
            assert_eq!(read.id, PageId::from(expected.clone()));
            assert_eq!(read.doc.kind, PageKind::Journal);
            // A date names no ordinary page (CM-DATE): a page lookup of it
            // opens the same day.
            assert!(
                matches!(view.resolve(name, false), Resolved::Existing { id, .. } if id == PageId::from(expected.clone()))
            );
            assert!(matches!(
                view.resolve("Oct 27th, 2026", true),
                Resolved::Alias { .. }
            ));
            assert!(matches!(
                view.resolve("Oct 27th, 2026", false),
                Resolved::Alias { .. }
            ));
            store.close();
        }
    }
}

/// What `name` opens as a journal lookup, cold (before the first parse,
/// `page_named` from the file listing) and ready (`WholeGraph::resolve`):
/// `(cold, ready)`, each `path` or `path+others` or `absent`/`alias`.
fn day_opens(root: &std::path::Path, names: &[&str]) -> Vec<(String, String, String)> {
    let pause = root.join(".tine-test-pause-load");
    std::fs::write(&pause, "").unwrap();
    let store = Store::open(root, Default::default()).unwrap().0;
    let cold: Vec<String> = names
        .iter()
        .map(
            |name| match store.page_named(name, PageKind::Journal).unwrap() {
                Some(read) => read.id.as_str().to_owned(),
                None => "absent".to_owned(),
            },
        )
        .collect();
    std::fs::remove_file(pause).unwrap();
    let view = store.whole_graph().unwrap();
    let out = names
        .iter()
        .zip(cold)
        .map(|(name, cold)| {
            let ready = match view.resolve(name, true) {
                Resolved::Existing { id, others } => std::iter::once(id)
                    .chain(others)
                    .map(|id| id.as_str().to_owned())
                    .collect::<Vec<_>>()
                    .join("+"),
                Resolved::Alias { .. } => "alias".to_owned(),
                Resolved::Absent { .. } => "absent".to_owned(),
            };
            (name.to_string(), cold, ready)
        })
        .collect();
    store.close();
    out
}

/// Model r6 O3 ("a journal holds every accepted spelling of its day"): a
/// spelling that parses to a day is held only by that day's journal. Under
/// `MM-dd`, which writes no year and so is not used (D-20261119-YL), the two
/// years of 26 October keep the default titles, and `10-26` is an ordinary
/// name no file claims. A format with a year is unchanged, and two files of
/// one day stay twins in path order.
#[test]
fn a_full_day_spelling_is_held_only_by_that_days_journal() {
    let _guard = super::integration_lock();
    let graph = |config: &str, journals: &[&str]| {
        let temp = tempfile::tempdir().unwrap();
        for dir in ["pages", "journals", "logseq"] {
            std::fs::create_dir(temp.path().join(dir)).unwrap();
        }
        std::fs::write(temp.path().join("logseq/config.edn"), config).unwrap();
        for file in journals {
            std::fs::write(temp.path().join("journals").join(file), "- day\n").unwrap();
        }
        temp
    };
    let row = |name: &str, opens: &str| (name.to_owned(), opens.to_owned(), opens.to_owned());
    let ambiguous = graph(
        "{:journal/page-title-format \"MM-dd\"}\n",
        &["2025_10_26.md", "2026_10_26.md"],
    );
    assert_eq!(
        day_opens(
            ambiguous.path(),
            &[
                "2026-10-26",
                "2025_10_26",
                "2027-10-26",
                "Oct 26th, 2025",
                "10-26"
            ]
        ),
        [
            row("2026-10-26", "journals/2026_10_26.md"),
            row("2025_10_26", "journals/2025_10_26.md"),
            row("2027-10-26", "absent"),
            row("Oct 26th, 2025", "journals/2025_10_26.md"),
            row("10-26", "absent"),
        ]
    );
    let with_year = graph(
        "{:journal/page-title-format \"yyyy-MM-dd\"}\n",
        &["2025_10_26.md", "2026_10_26.md", "2026-10-26.md"],
    );
    assert_eq!(
        day_opens(
            with_year.path(),
            &["2026-10-26", "Oct 26th, 2026", "2025-10-26"]
        ),
        [
            (
                "2026-10-26".to_owned(),
                "journals/2026-10-26.md".to_owned(),
                "journals/2026-10-26.md+journals/2026_10_26.md".to_owned()
            ),
            (
                "Oct 26th, 2026".to_owned(),
                "journals/2026-10-26.md".to_owned(),
                "journals/2026-10-26.md+journals/2026_10_26.md".to_owned()
            ),
            row("2025-10-26", "journals/2025_10_26.md"),
        ]
    );
}
