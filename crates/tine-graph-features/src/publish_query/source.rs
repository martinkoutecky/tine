//! Query publication inputs. Selection reads a held view; the shared publisher
//! owns evaluation, validation, review, output and recovery.
use super::*;
use tine_core::doc::DocBlock;
use tine_core::model::PageId;
use tine_core::CorpusPage;

#[cfg(test)]
thread_local! {
    static PAGE_SOURCE_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Resolve one persisted block UUID to the dialog's query publication input.
/// Within a loaded graph: O(log I + candidate-page blocks), no writes or source reads. The body
/// parser owns query recognition; an ambiguous UUID or multi-query block refuses.
pub fn query_request_by_uuid(
    store: &Store,
    uuid: &str,
    name: Option<&str>,
    replace: bool,
) -> io::Result<QueryExportRequest> {
    let graph = store
        .whole_graph()
        .map_err(|e| io::Error::other(format!("graph load failed: {e:?}")))?;
    let corpus = graph.corpus(tine_store::CorpusScope::BlockIdCandidates(uuid), true);
    let mut found = None;
    for page in &corpus.pages {
        let mut stack: Vec<_> = page.document.roots.iter().collect();
        while let Some(block) = stack.pop() {
            #[cfg(feature = "test-faults")]
            tine_store::cost_counters::query_uuid_block(uuid);
            stack.extend(block.children.iter());
            if block
                .property("id")
                .is_some_and(|id| id.eq_ignore_ascii_case(uuid))
            {
                if found.is_some() {
                    return Err(refusal(format!("ambiguous block UUID: {uuid}")));
                }
                found = Some((page, block));
            }
        }
    }
    let (page, block) = found.ok_or_else(|| refusal(format!("unknown block UUID: {uuid}")))?;
    let query =
        render::query_for_export(block.visible_text(), block.is_org(), false).map_err(refusal)?;
    Ok(block_request(
        page,
        block,
        query,
        name,
        &format!("Query {uuid}"),
        replace,
    ))
}

/// Inline argument of a query macro: OG simple or advanced, discriminated by
/// the shared parser during planning. O(text bytes), no graph access or writes;
/// there are no block properties or host-page context. The caller supplies a name.
pub fn query_request_inline(argument: String, name: &str, replace: bool) -> QueryExportRequest {
    QueryExportRequest {
        argument,
        dialect: QueryTextDialect::MacroQuery,
        properties: vec![],
        current_page: None,
        name: name.into(),
        host_block_id: None,
        folder: None,
        replace,
        asset_budget_bytes: None,
        include_query_text: false,
        include_comments: None,
    }
}

/// Select the first parser-recognized query on an already resolved page.
/// O(bounded indexed lookup + candidate-page bytes), zero source reads/writes.
/// Walk parents before children, siblings in authored order, without following
/// embeds. A missing page, no query or unsupported first query refuses.
pub fn query_request_on_page(
    graph: &WholeGraph,
    id: &PageId,
    name: Option<&str>,
    replace: bool,
) -> io::Result<QueryExportRequest> {
    let inventory = graph.inventory(tine_store::InventoryScope::File(id));
    let entry = inventory
        .0
        .first()
        .ok_or_else(|| refusal("query page is absent"))?;
    let corpus = graph.corpus(Some(entry.name.as_str()), true);
    let page = corpus
        .pages
        .iter()
        .find(|page| &page.id == id)
        .ok_or_else(|| refusal("query page is absent"))?;
    let mut stack = vec![page.document.roots.iter()];
    while let Some(blocks) = stack.last_mut() {
        let Some(block) = blocks.next() else {
            stack.pop();
            continue;
        };
        #[cfg(test)]
        PAGE_SOURCE_PARSES.with(|count| count.set(count.get() + 1));
        match render::query_for_export(block.visible_text(), block.is_org(), true) {
            Ok(query) => {
                return Ok(block_request(
                    page,
                    block,
                    query,
                    name,
                    &format!("Query {}", page.name),
                    replace,
                ))
            }
            Err("block is not a query") => {
                if !block.children.is_empty() {
                    stack.push(block.children.iter());
                }
            }
            Err(error) => return Err(refusal(error)),
        }
    }
    Err(refusal(format!("page `{}` has no query", page.name)))
}

fn block_request(
    page: &CorpusPage,
    block: &DocBlock,
    (argument, dialect, title): (String, QueryTextDialect, Option<String>),
    name: Option<&str>,
    fallback: &str,
    replace: bool,
) -> QueryExportRequest {
    let mut request = query_request_inline(
        argument,
        name.or(title.as_deref()).unwrap_or(fallback),
        replace,
    );
    request.dialect = dialect;
    request.properties = block
        .properties()
        .into_iter()
        .filter(|(key, _)| query_parse_property(key) || legacy_table_property(key))
        .collect();
    request.current_page = Some(page.name.clone());
    request.host_block_id = Some(block.uuid.to_string());
    request
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page_source_large_prefix(org: bool, code: bool, newline: &str) {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("pages")).unwrap();
        let prose = "x".repeat(65_537);
        let prefix = match (org, code) {
            (true, true) => format!(
                "#+BEGIN_SRC text{newline}{prose} {{{{query (task DOING)}}}}{newline}#+END_SRC"
            ),
            (false, true) => {
                format!("```text{newline}{prose} {{{{query (task DOING)}}}}{newline}```")
            }
            (_, false) => prose,
        };
        let raw = if org {
            format!("* {prefix}{newline}* {{{{query (task TODO)}}}}{newline}")
        } else {
            format!(
                "- {}{newline}- {{{{query (task TODO)}}}}{newline}",
                prefix.replace(newline, &format!("{newline}  "))
            )
        };
        let extension = if org { "org" } else { "md" };
        std::fs::write(root.path().join(format!("pages/Large.{extension}")), raw).unwrap();
        let store = Store::open(root.path(), Default::default()).unwrap().0;
        store.stop_watcher_for_tests();
        let graph = store.whole_graph().unwrap();
        let Resolved::Existing { id, .. } = graph.resolve("Large", false) else {
            panic!("fixture page");
        };
        assert_eq!(
            query_request_on_page(&graph, &id, None, false)
                .unwrap()
                .argument,
            "(task TODO)"
        );
        store.close();
    }

    macro_rules! large_prefix_case {
        ($name:ident, $org:expr, $code:expr, $newline:expr) => {
            #[test]
            fn $name() {
                page_source_large_prefix($org, $code, $newline);
            }
        };
    }

    large_prefix_case!(
        page_source_large_prefix_markdown_prose_lf,
        false,
        false,
        "\n"
    );
    large_prefix_case!(
        page_source_large_prefix_markdown_prose_crlf,
        false,
        false,
        "\r\n"
    );
    large_prefix_case!(page_source_large_prefix_markdown_code_lf, false, true, "\n");
    large_prefix_case!(
        page_source_large_prefix_markdown_code_crlf,
        false,
        true,
        "\r\n"
    );
    large_prefix_case!(page_source_large_prefix_org_prose_lf, true, false, "\n");
    large_prefix_case!(page_source_large_prefix_org_prose_crlf, true, false, "\r\n");
    large_prefix_case!(page_source_large_prefix_org_code_lf, true, true, "\n");
    large_prefix_case!(page_source_large_prefix_org_code_crlf, true, true, "\r\n");

    #[test]
    fn page_source_large_query_still_refuses() {
        for org in [false, true] {
            for first in [false, true] {
                for raw in [
                    format!("{{{{query (task {})}}}}", "x".repeat(65_537)),
                    format!("{{{{tine-query @block {}}}}}", " ".repeat(65_537)),
                    format!(
                        "#+BEGIN_QUERY\n{{:query [{}]}}\n#+END_QUERY",
                        " ".repeat(65_537)
                    ),
                ] {
                    assert_eq!(
                        render::query_for_export(&raw, org, first).unwrap_err(),
                        "query block exceeds the query source limit"
                    );
                }
            }
        }
    }

    #[test]
    fn page_source_large_query_refuses_before_later_query_and_inline_plan_is_bounded() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("pages")).unwrap();
        let argument = format!("(task {})", "x".repeat(65_537));
        let uuid = "a6f5c174-8f35-4000-8000-000000000588";
        std::fs::write(
            root.path().join("pages/Large.md"),
            format!("- {{{{query {argument}}}}}\n  id:: {uuid}\n- {{{{query (task TODO)}}}}\n"),
        )
        .unwrap();
        let store = Store::open(root.path(), Default::default()).unwrap().0;
        store.stop_watcher_for_tests();
        let graph = store.whole_graph().unwrap();
        let Resolved::Existing { id, .. } = graph.resolve("Large", false) else {
            panic!("fixture page");
        };
        for error in [
            query_request_on_page(&graph, &id, None, false).unwrap_err(),
            query_request_by_uuid(&store, uuid, None, false).unwrap_err(),
        ] {
            assert_eq!(
                error.to_string(),
                "query block exceeds the query source limit"
            );
        }
        let request = query_request_inline(argument, "Large", false);
        assert_eq!(
            plan_query(&store, &request).unwrap_err().to_string(),
            "query exceeds the export parser limit"
        );
        store.close();
    }

    #[test]
    fn page_source_work_stops_at_the_first_query() {
        for tail_blocks in [8, 2048] {
            let root = tempfile::tempdir().unwrap();
            std::fs::create_dir(root.path().join("pages")).unwrap();
            let mut raw = "- ordinary\n- {{query (task TODO)}}\n".to_owned();
            for index in 0..tail_blocks {
                raw.push_str(&format!("- later {index}\n"));
            }
            std::fs::write(root.path().join("pages/SourceCost.md"), raw).unwrap();
            let store = Store::open(root.path(), Default::default()).unwrap().0;
            store.stop_watcher_for_tests();
            let graph = store.whole_graph().unwrap();
            let Resolved::Existing { id, .. } = graph.resolve("SourceCost", false) else {
                panic!("fixture page");
            };
            PAGE_SOURCE_PARSES.with(|count| count.set(0));
            let start = std::time::Instant::now();
            for _ in 0..100 {
                let request = query_request_on_page(&graph, &id, None, false).unwrap();
                assert_eq!(request.argument, "(task TODO)");
            }
            let elapsed = start.elapsed();
            assert_eq!(PAGE_SOURCE_PARSES.with(|count| count.get()), 200);
            assert!(
                elapsed < std::time::Duration::from_millis(500),
                "100 selections with {tail_blocks} later blocks: {elapsed:?}"
            );
            store.close();
        }
    }
}
