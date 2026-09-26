use super::*;
use std::fs;

fn no_refs() -> RefIndex {
    RefIndex::new()
}

mod tests {
    use super::*;

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
        let graph = RenderGraph {
            corpus: &corpus,
            whole: &whole,
            store: &store,
        };
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
            assert!(
                render_query(&graph, "(task TODO)", &ctx, 0).contains("repeated query memo target")
            );
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
        let graph = RenderGraph {
            corpus: &corpus,
            whole: &whole,
            store: &store,
        };
        let mut dashboard = Vec::new();
        publish_graph(&graph, true, &[], &mut |name, bytes| {
            if name == "dashboard.html" {
                dashboard = bytes.to_vec();
            }
            Ok(())
        })
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
        let graph = RenderGraph {
            corpus: &corpus,
            whole: &whole,
            store: &store,
        };
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
        let graph = RenderGraph {
            corpus: &corpus,
            whole: &whole,
            store: &store,
        };
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
        assert!(render_query(&graph, &oversized, &ctx, 0).contains("publication limit"));
        let nested = format!("{}(task TODO){}", "(and ".repeat(1_000), ")".repeat(1_000));
        assert!(render_query(&graph, &nested, &ctx, 0).contains("nesting is too deep"));
        assert!(cache.borrow().entries.is_empty());

        for index in 0..(QUERY_CACHE_MAX_ENTRIES + 20) {
            let source = format!("(and (task TODO) (content \"memo-{index}\"))");
            let _ = render_query(&graph, &source, &ctx, 0);
        }
        let cache = cache.borrow();
        assert_eq!(cache.entries.len(), QUERY_CACHE_MAX_ENTRIES);
        assert!(cache.bytes <= QUERY_CACHE_MAX_BYTES);
        let _ = fs::remove_dir_all(&dir);
    }
}
