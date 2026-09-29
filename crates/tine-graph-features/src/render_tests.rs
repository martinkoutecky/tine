use super::*;
use crate::render_query_cache::{QUERY_CACHE_MAX_BYTES, QUERY_CACHE_MAX_ENTRIES};
use std::fs;

fn no_refs() -> RefIndex {
    RefIndex::new()
}

mod tests {
    use super::*;

    #[test]
    fn published_alias_links_and_authored_anchors_resolve() {
        let dir =
            std::env::temp_dir().join(format!("tine-publish-alias-anchor-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::write(
            dir.join("pages/Actual.md"),
            "alias:: Other\n\n- authored\n  id:: b0\n- generated\n",
        )
        .unwrap();
        fs::write(dir.join("pages/Source.md"), "- [[Other]] and [[Actual]]\n").unwrap();
        let store = Store::open(&dir, Default::default()).unwrap().0;
        let whole = store.whole_graph().unwrap();
        let corpus = whole.corpus();
        let graph = RenderGraph::new(&corpus, &whole, &store, None);
        let mut files = HashMap::<String, String>::new();
        publish_graph(
            &graph,
            PageSelection::AllButOptedOut,
            &[],
            &mut |name, bytes| {
                files.insert(name.to_owned(), String::from_utf8(bytes.to_vec()).unwrap());
                Ok(())
            },
        )
        .unwrap();
        assert!(files["source.html"].contains("href=\"actual.html\""));
        assert!(!files["source.html"].contains("href=\"other.html\""));
        assert_eq!(files["actual.html"].matches("id=\"b0\"").count(), 1);
        assert!(files["actual.html"].contains("id=\"b1\""));
        assert!(files["source.html"].contains("id=\"b0\""));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn published_page_shows_logbook_badge_and_ordinal_markers() {
        let dir = std::env::temp_dir().join(format!("tine-publish-facets-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::write(
            dir.join("pages/Facets.md"),
            "public:: true\n\
             - DONE A task with a logbook\n  \
               :LOGBOOK:\n  \
               CLOCK: [2026-07-01 Wed 10:00:00]--[2026-07-01 Wed 10:30:00] =>  00:30:00\n  \
               :END:\n\
             - Numbered parent\n  \
               logseq.order-list-type:: number\n\
             \t- First\n\
             \t\tlogseq.order-list-type:: number\n\
             \t- Second\n\
             \t\tlogseq.order-list-type:: number\n\
             - Another numbered\n  \
               logseq.order-list-type:: number\n\
             \t- Plain child\n",
        )
        .unwrap();
        let store = Store::open(&dir, Default::default()).unwrap().0;
        let whole = store.whole_graph().unwrap();
        let corpus = whole.corpus();
        let graph = RenderGraph::new(&corpus, &whole, &store, None);
        let mut files = HashMap::<String, String>::new();
        publish_graph(
            &graph,
            PageSelection::AllButOptedOut,
            &[],
            &mut |name, bytes| {
                files.insert(name.to_owned(), String::from_utf8(bytes.to_vec()).unwrap());
                Ok(())
            },
        )
        .unwrap();
        let html = &files["facets.html"];
        // The hidden LOGBOOK drawer still yields its elapsed-time badge.
        assert!(html.contains("<div class=\"planning logbook\">"), "{html}");
        assert!(html.contains("00:30:00"), "clock total: {html}");
        assert!(
            !html.contains("CLOCK: [2026"),
            "drawer stays hidden: {html}"
        );
        // Own-numbered blocks show their ordinal: roots 1. and 2.; the children
        // of an own-numbered parent cycle to letters (a., b.); a plain child
        // and the logbook task show none.
        let marker = |m: &str| format!("<span class=\"ord-marker\">{m}</span>");
        for m in ["1.", "a.", "b."] {
            assert!(html.contains(&marker(m)), "missing {m}: {html}");
        }
        assert!(html.contains(&marker("2.")), "second root run: {html}");
        assert!(!html.contains(&marker("3.")), "{html}");
        assert_eq!(html.matches("ol-item").count(), 4, "{html}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn repeated_query_sources_use_one_render_cache_entry() {
        let dir = std::env::temp_dir().join(format!(
            "tine-publish-query-memo-cache-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::write(
            dir.join("pages/Tasks.md"),
            "- TODO repeated query memo target\n",
        )
        .unwrap();
        let store = Store::open(&dir, Default::default()).unwrap().0;
        let whole = store.whole_graph().unwrap();
        let corpus = whole.corpus();
        let graph = RenderGraph::new(&corpus, &whole, &store, None);
        let refs = no_refs();
        let cache = RefCell::new(QueryCache::default());
        let ctx = Ctx {
            refs: &refs,
            reverse_refs: None,
            graph: Some(&graph),
            slugs: None,
            inline_assets: false,
            print_asset_budget: None,
            query_cache: Some(&cache),
            pages: None,
        };
        for _ in 0..5 {
            assert!(render_query(&graph, "(task TODO)", false, &ctx, 0)
                .contains("repeated query memo target"));
        }
        assert_eq!(cache.borrow().entries.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn repeated_page_embeds_render_from_corpus_after_source_removal() {
        let dir = std::env::temp_dir().join(format!(
            "tine-publish-embed-reuse-corpus-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::write(
            dir.join("pages/Target.md"),
            "- shared embed target\n  id:: 33333333-3333-3333-3333-333333333333\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages/Dashboard.md"),
            "- {{embed [[Target]]}}\n- {{embed [[Target]]}}\n- {{embed [[Target]]}}\n- {{embed [[Target]]}}\n",
        ).unwrap();
        let store = Store::open(&dir, Default::default()).unwrap().0;
        let whole = store.whole_graph().unwrap();
        let corpus = whole.corpus();
        fs::remove_file(dir.join("pages/Target.md")).unwrap();
        let graph = RenderGraph::new(&corpus, &whole, &store, None);
        let mut dashboard = Vec::new();
        publish_graph(
            &graph,
            PageSelection::AllButOptedOut,
            &[],
            &mut |name, bytes| {
                if name == "dashboard.html" {
                    dashboard = bytes.to_vec();
                }
                Ok(())
            },
        )
        .unwrap();
        let dashboard = String::from_utf8(dashboard).unwrap();
        assert_eq!(dashboard.matches("shared embed target").count(), 4);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn base64_matches_known_vectors() {
        // RFC 4648 test vectors + a binary triple that exercises all 6-bit lanes.
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(&[0xff, 0xef, 0xbf]), "/++/");
    }

    #[test]
    fn print_asset_inlining_enforces_per_file_and_shared_export_budgets() {
        let dir = std::env::temp_dir().join(format!("tine-print-budget-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("assets")).unwrap();
        fs::write(dir.join("assets/one.png"), b"1234").unwrap();
        fs::write(dir.join("assets/two.png"), b"5678").unwrap();
        fs::write(dir.join("assets/large.png"), b"123456").unwrap();
        let store = Store::open(&dir, Default::default()).unwrap().0;
        let whole = store.whole_graph().unwrap();
        let corpus = whole.corpus();
        let graph = RenderGraph::new(&corpus, &whole, &store, None);
        let refs = no_refs();
        let cumulative = RefCell::new(PrintAssetBudget {
            per_asset: 5,
            remaining: 7,
        });
        let cumulative_ctx = Ctx {
            refs: &refs,
            reverse_refs: None,
            graph: Some(&graph),
            slugs: None,
            inline_assets: true,
            print_asset_budget: Some(&cumulative),
            query_cache: None,
            pages: None,
        };

        assert!(inline_asset_uri(&cumulative_ctx, "../assets/one.png").is_some());
        assert_eq!(cumulative.borrow().remaining, 3);
        assert!(
            inline_asset_uri(&cumulative_ctx, "../assets/two.png").is_none(),
            "the second valid file must not cross the shared export ceiling"
        );
        assert_eq!(
            cumulative.borrow().remaining,
            3,
            "a rejection consumes no budget"
        );

        let per_file = RefCell::new(PrintAssetBudget {
            per_asset: 5,
            remaining: 20,
        });
        let per_file_ctx = Ctx {
            print_asset_budget: Some(&per_file),
            ..cumulative_ctx
        };
        assert!(
            inline_asset_uri(&per_file_ctx, "../assets/large.png").is_none(),
            "one oversized file must be rejected before it is returned"
        );
        assert_eq!(per_file.borrow().remaining, 20);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn publication_rejects_query_sources_before_keying_and_bounds_valid_memos() {
        let dir = std::env::temp_dir().join(format!(
            "tine-publish-query-source-bound-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(dir.join("pages").join("P.md"), "- TODO target\n").unwrap();
        let store = Store::open(&dir, Default::default()).unwrap().0;
        let whole = store.whole_graph().unwrap();
        let corpus = whole.corpus();
        let graph = RenderGraph::new(&corpus, &whole, &store, None);
        let _ = whole.corpus();
        let refs = RefIndex::new();
        let cache: SharedQueryCache = RefCell::new(QueryCache::default());
        let ctx = Ctx {
            refs: &refs,
            reverse_refs: None,
            graph: Some(&graph),
            slugs: None,
            inline_assets: false,
            print_asset_budget: None,
            query_cache: Some(&cache),
            pages: None,
        };

        let oversized = "x".repeat(tine_core::query::QUERY_SOURCE_MAX_BYTES + 1);
        assert!(render_query(&graph, &oversized, false, &ctx, 0).contains("publication limit"));
        let nested = format!("{}(task TODO){}", "(and ".repeat(1_000), ")".repeat(1_000));
        assert!(render_query(&graph, &nested, false, &ctx, 0).contains("nesting is too deep"));
        assert!(cache.borrow().entries.is_empty());

        for index in 0..(QUERY_CACHE_MAX_ENTRIES + 20) {
            let source = format!("(and (task TODO) (content \"memo-{index}\"))");
            let _ = render_query(&graph, &source, false, &ctx, 0);
        }
        let cache = cache.borrow();
        assert_eq!(cache.entries.len(), QUERY_CACHE_MAX_ENTRIES);
        assert!(cache.bytes <= QUERY_CACHE_MAX_BYTES);
        let _ = fs::remove_dir_all(&dir);
    }

    /// I-15: embeds, queries and namespace lists look pages up in indexes built
    /// once per publish, so lookup work does not grow with the macro count.
    #[test]
    fn publish_macro_lookups_are_built_once_per_publish() {
        let dir = std::env::temp_dir().join(format!("tine-publish-probes-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        const PAGES: usize = 40;
        const MACROS: usize = 30;
        let mut host = String::new();
        for i in 0..PAGES {
            let id = format!("00000000-0000-4000-8000-{i:012}");
            fs::write(
                dir.join(format!("pages/P{i}.md")),
                format!("- TODO item {i}\n  id:: {id}\n"),
            )
            .unwrap();
            if i < MACROS {
                host.push_str(&format!(
                    "- {{{{embed (({id}))}}}}\n- {{{{query (property k{i} v)}}}}\n- {{{{namespace N{i}}}}}\n"
                ));
            }
        }
        fs::write(dir.join("pages/Host.md"), host).unwrap();
        let store = Store::open(&dir, Default::default()).unwrap().0;
        let whole = store.whole_graph().unwrap();
        let corpus = whole.corpus();
        let graph = RenderGraph::new(&corpus, &whole, &store, None);
        render_lookups::PAGE_PROBES.with(|probes| probes.set(0));
        let mut host_html = String::new();
        publish_graph(
            &graph,
            PageSelection::AllButOptedOut,
            &[],
            &mut |name, bytes| {
                if name == "host.html" {
                    host_html = String::from_utf8(bytes.to_vec()).unwrap();
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(host_html.matches("embed block-embed").count(), MACROS);
        assert_eq!(host_html.matches("<div class=\"query\">").count(), MACROS);
        let probes = render_lookups::PAGE_PROBES.with(|probes| probes.get());
        assert!(
            probes <= 3 * (PAGES + 1),
            "{probes} page probes for {MACROS} embeds + queries + namespaces over {} pages",
            PAGES + 1
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
