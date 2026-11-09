use super::super::*;

#[test]
fn guide_explains_export_namespace_membership() {
    let guide = include_str!("../../tine-guide/src/templates/files-external-edits-backups.md");
    assert!(guide.contains(
        "`{{namespace Project}}` lists descendants such as `project/y` using page identity"
    ));
    assert!(guide.contains("HTML lists only pages included in that export"));
}

#[test]
fn namespace_identity_membership_in_print_and_publish() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("pages")).unwrap();
    for (file, title, body) in [
        ("Host", "Host", "{{namespace Project}}"),
        ("a", "Project/Z", "winner"),
        ("b", "project/z", "losing twin"),
        ("c", "project/y", "child"),
        ("d", "Projects/x", "unrelated prefix"),
        ("e", "project/deep/leaf", "descendant"),
        ("f", "Project", "parent"),
        // A page-level alias: `alias::` in the preamble, beside `title::`.
        ("g", "AliasOwner\nalias:: Project/Alias", "owner"),
        ("h", "Refs", "[[Project/ReferenceOnly]]"),
    ] {
        std::fs::write(
            root.path().join(format!("pages/{file}.md")),
            format!("title:: {title}\n- {body}\n"),
        )
        .unwrap();
    }
    let store = Store::open(root.path(), Default::default()).unwrap().0;
    let whole = store.whole_graph_reconciled().unwrap();
    let corpus = whole.corpus(None, true);
    for keyed in [false, true] {
        let mut graph = RenderGraph::new(&corpus, &whole, &store, None);
        graph.lookups.keyed = keyed;
        let html = page_print_html(&graph, "Host", Default::default())
            .unwrap()
            .unwrap();
        let expected = [">Project/Z</a>", ">project/deep/leaf</a>", ">project/y</a>"];
        let positions: Vec<_> = expected
            .iter()
            .map(|name| html.find(name).expect(name))
            .collect();
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "displayed-name byte order"
        );
        for excluded in [
            ">project/z</a>",
            ">Projects/x</a>",
            ">Project/Alias</a>",
            ">Project/ReferenceOnly</a>",
        ] {
            assert!(!html.contains(excluded), "excluded {excluded}");
        }
        // Both full and preselected publication retain the same membership,
        // with the existing export admission filter applied afterwards.
        let mut published = String::new();
        publish_graph(
            &graph,
            PageSelection::AllButOptedOut,
            &[],
            &mut |file, bytes| {
                if file == "host.html" {
                    published = String::from_utf8(bytes.to_vec()).unwrap();
                }
                Ok(())
            },
        )
        .unwrap();
        // Publication already refuses ambiguous physical identities, even
        // though print can list their semantic owner. Preserve that filter.
        for name in &expected[1..] {
            assert!(published.contains(name), "publish: {name}");
        }
        let namespace = published
            .split("<div class=\"namespace-macro\">")
            .nth(1)
            .unwrap()
            .split("</ul>")
            .next()
            .unwrap();
        assert!(!namespace.contains(">Project/Z</a>"));
        let selected = Corpus {
            pages: corpus
                .pages
                .iter()
                .filter(|page| ["Host", "project/y"].contains(&page.name.as_str()))
                .cloned()
                .collect(),
        };
        let mut graph = RenderGraph::new(&selected, &whole, &store, None);
        graph.lookups.keyed = keyed;
        let mut published = String::new();
        publish_graph(
            &graph,
            PageSelection::Preselected,
            &[],
            &mut |file, bytes| {
                if file == "host.html" {
                    published = String::from_utf8(bytes.to_vec()).unwrap();
                }
                Ok(())
            },
        )
        .unwrap();
        let namespace = published
            .split("<div class=\"namespace-macro\">")
            .nth(1)
            .unwrap()
            .split("</ul>")
            .next()
            .unwrap();
        assert!(namespace.contains(">project/y</a>"));
        assert!(!namespace.contains(">Project/Z</a>"));
    }
    store.close();
}

#[test]
fn namespace_lookup_visits_only_matching_names() {
    let mut counts = Vec::new();
    for unrelated in [20, 200] {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("pages")).unwrap();
        for i in 0..unrelated {
            std::fs::write(
                root.path().join(format!("pages/Other{i}.md")),
                "- unrelated\n",
            )
            .unwrap();
        }
        std::fs::write(
            root.path().join("pages/Child.md"),
            "title:: Project/Child\n- child\n",
        )
        .unwrap();
        let store = Store::open(root.path(), Default::default()).unwrap().0;
        let whole = store.whole_graph_reconciled().unwrap();
        let corpus = Corpus::default();
        let graph = RenderGraph::for_pages(&corpus, &whole, &store, None);
        render_lookups::PAGE_PROBES.with(|probes| probes.set(0));
        let names = graph.lookups.names_under(&whole, "Project/");
        let probes = render_lookups::PAGE_PROBES.with(|probes| probes.get());
        eprintln!(
            "namespace lookup {unrelated}: {probes} corpus page probes; {} names returned",
            names.len()
        );
        assert_eq!(names, ["Project/Child"]);
        counts.push(probes);
        store.close();
    }
    assert_eq!(
        counts,
        [0, 0],
        "namespace lookup must not visit unrelated corpus pages"
    );
}
