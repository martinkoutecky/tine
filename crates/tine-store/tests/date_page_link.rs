use tine_core::model::PageKind;
use tine_store::{PageId, Resolved, Store};

#[test]
fn date_link_falls_back_to_page_file_but_journal_keeps_precedence() {
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
            assert_eq!(early.unwrap().unwrap().id, PageId::from(expected.clone()));
            let view = store.whole_graph().unwrap();
            assert!(
                matches!(view.resolve(name, true), Resolved::Existing { id, .. } if id == PageId::from(expected.clone()))
            );
            let read = store.page_named(name, PageKind::Journal).unwrap().unwrap();
            assert_eq!(read.id, PageId::from(expected));
            assert_eq!(
                read.doc.kind,
                if journal {
                    PageKind::Journal
                } else {
                    PageKind::Page
                }
            );
            // The fallback changes link resolution, never the ordinary file's identity.
            assert!(
                matches!(view.resolve(name, false), Resolved::Existing { id, .. } if id == PageId::from(format!("pages/{filename}")))
            );
            assert!(matches!(
                view.resolve("Oct 27th, 2026", true),
                Resolved::Absent { .. }
            ));
            assert!(matches!(
                view.resolve("Oct 27th, 2026", false),
                Resolved::Alias { .. }
            ));
            store.close();
        }
    }
}
