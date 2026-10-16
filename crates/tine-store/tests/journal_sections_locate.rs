//! GH #605: the today-sections page is found by its marker page property
//! `tine/journal-sections:: true` (Org: `#+tine/journal-sections: true`)
//! through the query engine, the way `src/journalSections.ts`
//! `locateSectionsPages` asks: `(page-property tine/journal-sections true)`,
//! page-anchored rows. A block-level property or `false` is not the marker.

use std::collections::BTreeSet;

use tine_core::query::ir::{ExecutionContext, QueryRows};
use tine_core::query::registry::Registry;
use tine_core::query::{parse_query_input, QueryInput};
use tine_store::{IrAnswer, IrRequest, Store};

#[test]
fn the_sections_marker_locates_markdown_and_org_pages_only() {
    let _integration_guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    std::fs::create_dir(dir.path().join("journals")).unwrap();
    let w =
        |name: &str, text: &str| std::fs::write(dir.path().join("pages").join(name), text).unwrap();
    w(
        "Journal sections.md",
        "tine/journal-sections:: true\n\n- {{query (task NOW)}}\n",
    );
    w(
        "Org sections.org",
        "#+tine/journal-sections: true\n\n* {{query (task LATER)}}\n",
    );
    w("Off.md", "tine/journal-sections:: false\n\n- body\n");
    w("Block level.md", "- body\n  tine/journal-sections:: true\n");
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();
    let (query, view) = parse_query_input(
        "(page-property tine/journal-sections true)",
        QueryInput::MacroQuery,
        tine_core::date::JournalDate::today(),
        Registry::none(),
    );
    let IrAnswer::Result(result) = graph
        .query_ir(IrRequest::Run {
            query: &query,
            view: &view,
            context: &ExecutionContext::default(),
        })
        .unwrap()
    else {
        panic!("query_ir(Run) returns a result");
    };
    let QueryRows::Page { pages } = result.rows else {
        panic!("page-property is page-anchored");
    };
    let names: BTreeSet<String> = pages.into_iter().map(|p| p.name.to_lowercase()).collect();
    assert_eq!(
        names,
        BTreeSet::from(["journal sections".to_string(), "org sections".to_string()])
    );
}
